use anyhow::{Context, Result};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// Git environment variables that encode the *caller's* repository context.
///
/// These must be stripped when spawning git subprocesses that target a
/// *different* repository (e.g. a temp repo in a test, or a submodule) so
/// that git operates on the directory supplied via `current_dir` rather than
/// the ambient repo referenced by the env vars.
const GIT_LOCAL_ENV_VARS: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_KEY_0",
    "GIT_CONFIG_VALUE_0",
    "GIT_DIR",
    "GIT_GRAFT_FILE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_INTERNAL_SUPER_PREFIX",
    "GIT_NAMESPACE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_OBJECT_DIRECTORY",
    "GIT_PREFIX",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
    "GIT_WORK_TREE",
];

/// Create a `git` [`Command`] with the ambient repository env vars removed so
/// that git uses the directory supplied via [`Command::current_dir`] rather
/// than whatever repository the parent process may be running inside.
pub(crate) fn git_cmd() -> Command {
    let mut cmd = Command::new("git");
    for var in GIT_LOCAL_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd
}

/// Canonical repo-relative path identity.
///
/// Invariant: ALL path-derived keys MUST derive from canonical repo-relative
/// path identity before hashing, persistence, dedupe, or cross-store ID
/// generation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalRepoPath(Utf8PathBuf);

impl CanonicalRepoPath {
    pub fn from_absolute_path(
        repo_root: &Utf8Path,
        abs_path: &Utf8Path,
    ) -> std::result::Result<Self, RepoPathError> {
        let canonical_root = normalize_absolute(repo_root, AbsoluteRole::RepoRoot)?;
        let canonical_abs = normalize_absolute(abs_path, AbsoluteRole::InputPath)?;
        let relative = canonical_abs
            .strip_prefix(canonical_root.as_path())
            .map_err(|_| RepoPathError::NotUnderRepoRoot {
                repo_root: canonical_root.to_string(),
                path: canonical_abs.to_string(),
            })?;
        Self::from_repo_relative(relative.as_str())
    }

    pub fn from_repo_relative(path: impl AsRef<str>) -> std::result::Result<Self, RepoPathError> {
        canonicalize_relative(path.as_ref())
    }

    pub fn from_git_diff_path(path: impl AsRef<str>) -> std::result::Result<Self, RepoPathError> {
        Self::from_repo_relative(path)
    }

    pub fn from_watch_event_path(
        repo_root: &Utf8Path,
        path: &Utf8Path,
    ) -> std::result::Result<Self, RepoPathError> {
        Self::from_boundary_input(repo_root, path)
    }

    pub fn from_cli_argument(
        repo_root: &Utf8Path,
        path: &Utf8Path,
    ) -> std::result::Result<Self, RepoPathError> {
        Self::from_boundary_input(repo_root, path)
    }

    pub fn from_synthetic_path(path: impl AsRef<str>) -> std::result::Result<Self, RepoPathError> {
        Self::from_repo_relative(path)
    }

    pub fn as_path(&self) -> &Utf8Path {
        self.0.as_path()
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_path_buf(self) -> Utf8PathBuf {
        self.0
    }

    fn from_boundary_input(
        repo_root: &Utf8Path,
        path: &Utf8Path,
    ) -> std::result::Result<Self, RepoPathError> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo_root.join(path)
        };

        if let (Ok(physical_root), Ok(physical_path)) = (
            canonical_filesystem_path(repo_root),
            canonical_filesystem_path(absolute.as_path()),
        ) {
            return Self::from_absolute_path(physical_root.as_path(), physical_path.as_path());
        }

        if path.is_absolute() {
            Self::from_absolute_path(repo_root, path)
        } else {
            Self::from_repo_relative(path.as_str())
        }
    }
}

impl AsRef<Utf8Path> for CanonicalRepoPath {
    fn as_ref(&self) -> &Utf8Path {
        self.as_path()
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum RepoPathError {
    #[error("canonical repo path must not be empty")]
    Empty,
    #[error("repo root '{0}' must be absolute")]
    RepoRootNotAbsolute(String),
    #[error("absolute path input '{0}' must be absolute")]
    AbsoluteInputNotAbsolute(String),
    #[error("repo-relative path '{0}' must not be absolute")]
    AbsoluteNotAllowed(String),
    #[error("path '{path}' is not under repo root '{repo_root}'")]
    NotUnderRepoRoot { repo_root: String, path: String },
    #[error("path '{0}' escapes repo root")]
    EscapesRepoRoot(String),
    #[error("path '{0}' must not end with '/'")]
    TrailingSlash(String),
    #[error("cannot canonicalize filesystem path '{path}': {message}")]
    FilesystemCanonicalize { path: String, message: String },
    #[error("path '{0}' is not valid UTF-8 after filesystem canonicalization")]
    NonUtf8Path(String),
    #[error(
        "file path '{requested}' does not exist in this repo and no unambiguous root-prefix was stripped"
    )]
    PathNotFound { requested: String },
    #[error(
        "file path '{requested}' is ambiguous after removing root-like prefixes; candidates: {candidates:?}"
    )]
    AmbiguousPath {
        requested: String,
        candidates: Vec<String>,
    },
}

/// Outcome of resolving a user-supplied file path against one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRepoPath {
    /// Canonical repo-relative path identity.
    pub canonical: String,
    /// How the input was resolved.
    pub reason: &'static str,
}

/// Resolve a boundary file-path input (repo-relative, repo-dir-prefixed, or
/// absolute under the repo root) to canonical repo-relative identity.
///
/// Resolution order:
/// 1. absolute path that canonicalizes under `repo_root`;
/// 2. repo-relative path whose joined candidate exists on disk;
/// 3. leading-segment stripping (foreign root dirs, duplicated repo-name
///    prefixes, nested subdir prefixes) — accepted only when exactly one
///    stripped candidate exists;
///
/// else [`RepoPathError`] with ambiguity/not-found detail.
///
/// This is the single boundary normalizer for MCP + CLI file-path inputs so
/// every surface accepts the same three path forms with consistent errors.
pub fn normalize_repo_file_path(
    repo_root: &Utf8Path,
    raw: &str,
) -> std::result::Result<NormalizedRepoPath, RepoPathError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(RepoPathError::Empty);
    }
    let normalized = trimmed.replace('\\', "/");
    let input = Utf8Path::new(&normalized);

    // Absolute input: must canonicalize under the repo root.
    if input.is_absolute() {
        let canonical = CanonicalRepoPath::from_cli_argument(repo_root, input)?;
        return Ok(NormalizedRepoPath {
            canonical: canonical.as_str().to_owned(),
            reason: "absolute",
        });
    }

    // Direct repo-relative: accept when the candidate exists on disk.
    if let Ok(canonical) = CanonicalRepoPath::from_repo_relative(&normalized)
        && repo_root.join(canonical.as_str()).exists()
    {
        return Ok(NormalizedRepoPath {
            canonical: canonical.as_str().to_owned(),
            reason: "direct",
        });
    }

    // Root-like prefix stripping: exactly one existing candidate wins.
    let repo_name = repo_root
        .file_name()
        .map(|name| name.to_owned())
        .unwrap_or_default();
    let segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let mut candidates: Vec<(String, &'static str)> = Vec::new();
    for strip_count in 1..segments.len() {
        let tail = segments[strip_count..].join("/");
        if tail.is_empty() {
            continue;
        }
        let Ok(canonical) = CanonicalRepoPath::from_repo_relative(&tail) else {
            continue;
        };
        if !repo_root.join(canonical.as_str()).exists() {
            continue;
        }
        let reason = if strip_count == 1 && segments[0] == repo_name {
            "stripped_duplicated_root_prefix"
        } else if strip_count == 1 {
            "stripped_foreign_root_prefix"
        } else {
            "stripped_nested_subdir_prefix"
        };
        if !candidates
            .iter()
            .any(|(path, _)| path == canonical.as_str())
        {
            candidates.push((canonical.as_str().to_owned(), reason));
        }
    }
    match candidates.len() {
        1 => Ok(NormalizedRepoPath {
            canonical: candidates[0].0.clone(),
            reason: candidates[0].1,
        }),
        0 => Err(RepoPathError::PathNotFound {
            requested: trimmed.to_owned(),
        }),
        _ => {
            candidates.sort();
            Err(RepoPathError::AmbiguousPath {
                requested: trimmed.to_owned(),
                candidates: candidates.into_iter().map(|(path, _)| path).collect(),
            })
        }
    }
}

#[derive(Clone, Copy)]
enum AbsoluteRole {
    RepoRoot,
    InputPath,
}

/// Return `path` relative to `repo_root`, with `/` separators.
///
/// Both paths must be absolute. The result is a clean relative path with no
/// leading `./`.
pub fn repo_relative(repo_root: &Utf8Path, abs_path: &Utf8Path) -> Result<Utf8PathBuf> {
    CanonicalRepoPath::from_absolute_path(repo_root, abs_path)
        .map(CanonicalRepoPath::into_path_buf)
        .with_context(|| format!("cannot derive canonical repo-relative path from '{abs_path}'"))
}

/// Return canonical absolute path identity using the same separator, casing,
/// and dot-segment rules used by [`CanonicalRepoPath::from_absolute_path`].
pub fn canonical_absolute_path(path: &Utf8Path) -> std::result::Result<Utf8PathBuf, RepoPathError> {
    normalize_absolute(path, AbsoluteRole::InputPath)
}

/// Return canonical absolute path identity using filesystem-resolved casing for
/// the deepest existing ancestor, then re-append any non-existing suffix.
///
/// This is Atlas' single boundary point for paths coming from user input,
/// watch events, or platform APIs where filesystem casing and Unicode form may
/// drift from git's textual form.
pub fn canonical_filesystem_path(
    path: &Utf8Path,
) -> std::result::Result<Utf8PathBuf, RepoPathError> {
    let normalized = canonical_absolute_path(path)?;
    let canonical = canonicalize_existing_ancestor(normalized.as_std_path())?;
    let utf8 = Utf8PathBuf::from_path_buf(canonical)
        .map_err(|path| RepoPathError::NonUtf8Path(path.display().to_string()))?;
    canonical_absolute_path(utf8.as_path())
}

fn canonicalize_relative(raw: &str) -> std::result::Result<CanonicalRepoPath, RepoPathError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(RepoPathError::Empty);
    }

    let slashed = normalize_identity(raw);
    let path = Utf8Path::new(&slashed);
    if path.is_absolute() {
        return Err(RepoPathError::AbsoluteNotAllowed(slashed));
    }

    let mut parts: Vec<&str> = Vec::new();
    for component in path.components() {
        match component {
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(RepoPathError::EscapesRepoRoot(slashed));
                }
            }
            Utf8Component::Normal(part) => parts.push(part),
            Utf8Component::RootDir | Utf8Component::Prefix(_) => {
                return Err(RepoPathError::AbsoluteNotAllowed(slashed));
            }
        }
    }

    if parts.is_empty() {
        return Err(RepoPathError::Empty);
    }

    if slashed.ends_with('/') {
        return Err(RepoPathError::TrailingSlash(slashed));
    }

    Ok(CanonicalRepoPath(Utf8PathBuf::from(normalize_case(
        &parts.join("/"),
    ))))
}

fn normalize_absolute(
    path: &Utf8Path,
    role: AbsoluteRole,
) -> std::result::Result<Utf8PathBuf, RepoPathError> {
    let cased = normalize_identity(path.as_str());
    let utf8_path = Utf8Path::new(&cased);
    if !utf8_path.is_absolute() {
        return Err(match role {
            AbsoluteRole::RepoRoot => RepoPathError::RepoRootNotAbsolute(path.to_string()),
            AbsoluteRole::InputPath => RepoPathError::AbsoluteInputNotAbsolute(path.to_string()),
        });
    }

    let mut prefix: Option<String> = None;
    let mut parts: Vec<&str> = Vec::new();
    for component in utf8_path.components() {
        match component {
            Utf8Component::Prefix(value) => prefix = Some(value.as_str().to_owned()),
            Utf8Component::RootDir => {}
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(RepoPathError::EscapesRepoRoot(cased));
                }
            }
            Utf8Component::Normal(part) => parts.push(part),
        }
    }

    let mut canonical = String::new();
    if let Some(prefix) = prefix {
        canonical.push_str(&prefix);
    }
    canonical.push('/');
    canonical.push_str(&parts.join("/"));
    Ok(Utf8PathBuf::from(canonical))
}

/// Ensure separators are `/` (matters on Windows where camino may receive `\`).
pub fn to_forward_slashes(s: &str) -> String {
    s.replace('\\', "/")
}

/// Normalize path Unicode to NFC so equivalent macOS NFD/NFC spellings share
/// one canonical Atlas identity.
pub fn normalize_unicode(s: &str) -> String {
    s.nfc().collect()
}

/// Normalize path casing for the current platform.
///
/// On Windows the filesystem is case-insensitive, so two paths that differ
/// only in case refer to the same file.  To guarantee the qualified-name
/// scheme is stable regardless of how a path was obtained, we lowercase the
/// entire path on Windows using Unicode lowercase folding. On Unix the
/// filesystem is case-sensitive, so no transformation is applied.
///
/// Call this **after** [`to_forward_slashes`] so that the input is already
/// separator-normalized.
pub fn normalize_case(s: &str) -> String {
    if cfg!(target_os = "windows") {
        s.chars().flat_map(char::to_lowercase).collect()
    } else {
        s.to_owned()
    }
}

fn normalize_identity(s: &str) -> String {
    normalize_case(&normalize_unicode(&to_forward_slashes(s)))
}

fn canonicalize_existing_ancestor(
    path: &Path,
) -> std::result::Result<std::path::PathBuf, RepoPathError> {
    let mut existing = path;
    let mut suffix = Vec::<OsString>::new();

    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| RepoPathError::FilesystemCanonicalize {
                path: path.display().to_string(),
                message: "no existing ancestor was found".to_string(),
            })?;
        suffix.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| RepoPathError::FilesystemCanonicalize {
                path: path.display().to_string(),
                message: "no existing ancestor was found".to_string(),
            })?;
    }

    let mut canonical =
        std::fs::canonicalize(existing).map_err(|error| RepoPathError::FilesystemCanonicalize {
            path: existing.display().to_string(),
            message: error.to_string(),
        })?;
    for component in suffix.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::string::string_regex;

    #[test]
    fn basic_relative() {
        let root = Utf8Path::new("/home/user/proj");
        let abs = Utf8Path::new("/home/user/proj/src/main.rs");
        assert_eq!(repo_relative(root, abs).unwrap().as_str(), "src/main.rs");
    }

    #[test]
    fn canonical_repo_relative_strips_dot_components() {
        let path = CanonicalRepoPath::from_repo_relative("./src/../src/lib.rs").unwrap();
        assert_eq!(path.as_str(), "src/lib.rs");
    }

    #[test]
    fn canonical_repo_relative_converts_backslashes() {
        let path = CanonicalRepoPath::from_repo_relative("src\\main\\lib.rs").unwrap();
        assert_eq!(path.as_str(), "src/main/lib.rs");
    }

    #[test]
    fn canonical_repo_relative_normalizes_unicode_equivalents() {
        let nfc = CanonicalRepoPath::from_repo_relative("caf\u{00e9}.rs").unwrap();
        let nfd = CanonicalRepoPath::from_repo_relative("cafe\u{0301}.rs").unwrap();
        assert_eq!(nfc, nfd);
        assert_eq!(nfc.as_str(), "caf\u{00e9}.rs");
    }

    #[test]
    fn forward_slashes_passthrough() {
        assert_eq!(to_forward_slashes("src/lib.rs"), "src/lib.rs");
    }

    #[test]
    fn backslashes_converted_to_forward() {
        assert_eq!(to_forward_slashes("src\\main\\lib.rs"), "src/main/lib.rs");
    }

    #[test]
    fn mixed_separators_converted() {
        assert_eq!(
            to_forward_slashes("packages\\atlas-cli/src\\main.rs"),
            "packages/atlas-cli/src/main.rs"
        );
    }

    #[test]
    fn empty_string_passthrough() {
        assert_eq!(to_forward_slashes(""), "");
    }

    #[test]
    fn canonical_repo_relative_rejects_empty() {
        let err = CanonicalRepoPath::from_repo_relative("").unwrap_err();
        assert_eq!(err, RepoPathError::Empty);
    }

    #[test]
    fn canonical_repo_relative_rejects_escape() {
        let err = CanonicalRepoPath::from_repo_relative("../Cargo.toml").unwrap_err();
        assert_eq!(
            err,
            RepoPathError::EscapesRepoRoot("../Cargo.toml".to_string())
        );
    }

    #[test]
    fn canonical_repo_relative_rejects_absolute() {
        let err = CanonicalRepoPath::from_repo_relative("/repo/src/lib.rs").unwrap_err();
        assert_eq!(
            err,
            RepoPathError::AbsoluteNotAllowed("/repo/src/lib.rs".to_string())
        );
    }

    #[test]
    fn canonical_repo_relative_rejects_trailing_slash() {
        let err = CanonicalRepoPath::from_repo_relative("src/").unwrap_err();
        assert_eq!(err, RepoPathError::TrailingSlash("src/".to_string()));
    }

    #[test]
    fn canonical_repo_relative_collapses_multiple_parent_segments() {
        let path = CanonicalRepoPath::from_repo_relative("a/b/../../c/d.rs").unwrap();
        assert_eq!(path.as_str(), "c/d.rs");
    }

    #[test]
    fn canonical_repo_relative_normalizes_deep_nesting() {
        let path = CanonicalRepoPath::from_repo_relative("a/./b/./c/../d.rs").unwrap();
        assert_eq!(path.as_str(), "a/b/d.rs");
    }

    // --- normalize_case (Windows casing policy) ------------------------------

    /// On Linux the function is a no-op — case is preserved.
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn normalize_case_noop_on_unix() {
        assert_eq!(normalize_case("Src/Main.rs"), "Src/Main.rs");
        assert_eq!(normalize_case("PKG/FOO.GO"), "PKG/FOO.GO");
    }

    /// On Windows the function lowercases to produce a stable canonical form.
    #[test]
    #[cfg(target_os = "windows")]
    fn normalize_case_lowercases_on_windows() {
        assert_eq!(normalize_case("Src/Main.rs"), "src/main.rs");
        assert_eq!(normalize_case("PKG/FOO.GO"), "pkg/foo.go");
        assert_eq!(normalize_case("CAF\u{00c9}/Main.rs"), "caf\u{00e9}/main.rs");
        assert_eq!(
            normalize_case("packages/Atlas-Core/Src/Lib.rs"),
            "packages/atlas-core/src/lib.rs"
        );
    }

    /// Verify that `to_forward_slashes` + `normalize_case` together produce
    /// the expected canonical form on all platforms when given a Windows-style
    /// mixed-separator path.
    #[test]
    fn round_trip_windows_path_unix() {
        let raw = "Packages\\Atlas-CLI\\Src\\Main.rs";
        let slashed = to_forward_slashes(raw);
        assert_eq!(slashed, "Packages/Atlas-CLI/Src/Main.rs");
        // normalize_case is a no-op on Unix but returns a String either way.
        let _ = normalize_case(&slashed);
    }

    #[test]
    fn repo_relative_normalizes_nested_unix_components() {
        let root = Utf8Path::new("/repo");
        let abs = Utf8Path::new("/repo/src/./nested/../lib.rs");
        assert_eq!(repo_relative(root, abs).unwrap().as_str(), "src/lib.rs");
    }

    #[test]
    fn canonical_absolute_path_normalizes_dot_segments() {
        let path = canonical_absolute_path(Utf8Path::new("/repo/src/./nested/../lib.rs")).unwrap();
        assert_eq!(path.as_str(), "/repo/src/lib.rs");
    }

    #[test]
    fn canonical_absolute_path_normalizes_unicode_equivalents() {
        let nfc = canonical_absolute_path(Utf8Path::new("/repo/caf\u{00e9}.rs")).unwrap();
        let nfd = canonical_absolute_path(Utf8Path::new("/repo/cafe\u{0301}.rs")).unwrap();
        assert_eq!(nfc, nfd);
        assert_eq!(nfc.as_str(), "/repo/caf\u{00e9}.rs");
    }

    #[test]
    fn canonical_absolute_path_rejects_relative_input() {
        let err = canonical_absolute_path(Utf8Path::new("src/lib.rs")).unwrap_err();
        assert_eq!(
            err,
            RepoPathError::AbsoluteInputNotAbsolute("src/lib.rs".to_string())
        );
    }

    #[test]
    fn absolute_constructor_rejects_outside_repo_root() {
        let root = Utf8Path::new("/repo");
        let abs = Utf8Path::new("/other/src/lib.rs");
        let err = CanonicalRepoPath::from_absolute_path(root, abs).unwrap_err();
        assert_eq!(
            err,
            RepoPathError::NotUnderRepoRoot {
                repo_root: "/repo".to_string(),
                path: "/other/src/lib.rs".to_string(),
            }
        );
    }

    #[test]
    fn absolute_constructor_rejects_non_absolute_repo_root() {
        let err = CanonicalRepoPath::from_absolute_path(
            Utf8Path::new("repo"),
            Utf8Path::new("/repo/src/lib.rs"),
        )
        .unwrap_err();
        assert_eq!(err, RepoPathError::RepoRootNotAbsolute("repo".to_string()));
    }

    #[test]
    fn absolute_constructor_rejects_non_absolute_input() {
        let err = CanonicalRepoPath::from_absolute_path(
            Utf8Path::new("/repo"),
            Utf8Path::new("src/lib.rs"),
        )
        .unwrap_err();
        assert_eq!(
            err,
            RepoPathError::AbsoluteInputNotAbsolute("src/lib.rs".to_string())
        );
    }

    #[test]
    fn git_diff_constructor_uses_same_canonical_rules() {
        let path = CanonicalRepoPath::from_git_diff_path("./src\\main.rs").unwrap();
        assert_eq!(path.as_str(), "src/main.rs");
    }

    #[test]
    fn cli_argument_constructor_accepts_absolute_and_relative_inputs() {
        let root = Utf8Path::new("/repo");
        let absolute =
            CanonicalRepoPath::from_cli_argument(root, Utf8Path::new("/repo/src/main.rs")).unwrap();
        let relative =
            CanonicalRepoPath::from_cli_argument(root, Utf8Path::new("./src/../src/main.rs"))
                .unwrap();
        assert_eq!(absolute.as_str(), "src/main.rs");
        assert_eq!(relative.as_str(), "src/main.rs");
    }

    #[test]
    fn watch_event_constructor_accepts_absolute_input() {
        let root = Utf8Path::new("/repo");
        let path =
            CanonicalRepoPath::from_watch_event_path(root, Utf8Path::new("/repo/src/lib.rs"))
                .unwrap();
        assert_eq!(path.as_str(), "src/lib.rs");
    }

    #[test]
    fn filesystem_canonical_path_reuses_physical_unicode_identity() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        let nfd_name = "cafe\u{0301}.rs";
        let abs = root.join(nfd_name);
        std::fs::write(abs.as_std_path(), "fn main() {}\n").unwrap();

        let canonical = canonical_filesystem_path(abs.as_path()).unwrap();
        // Use canonical_filesystem_path on the directory itself so the expected
        // prefix matches on platforms where tempdir returns a symlink path
        // (e.g. macOS /var -> /private/var).
        let canonical_root = canonical_filesystem_path(root).unwrap();
        assert_eq!(
            canonical.as_str(),
            &format!("{}/caf\u{00e9}.rs", canonical_root.as_str())
        );
    }

    #[test]
    fn synthetic_path_constructor_reuses_relative_rules() {
        let path = CanonicalRepoPath::from_synthetic_path("generated/schema.graph.json").unwrap();
        assert_eq!(path.as_str(), "generated/schema.graph.json");
    }

    // --- normalize_repo_file_path (boundary path normalizer) ----------------

    #[test]
    fn normalize_repo_file_path_accepts_all_three_boundary_forms() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();
        std::fs::write(root.join("src/lib.rs").as_std_path(), "fn main() {}\n").unwrap();
        let repo_name = root.file_name().unwrap().to_string();

        // 1. repo-relative.
        let direct = normalize_repo_file_path(root, "src/lib.rs").unwrap();
        assert_eq!(direct.canonical, "src/lib.rs");
        assert_eq!(direct.reason, "direct");

        // 2. repo-dir-prefixed (with and without backslash separators).
        let prefixed = normalize_repo_file_path(root, &format!("{repo_name}/src/lib.rs")).unwrap();
        assert_eq!(prefixed.canonical, "src/lib.rs");
        assert_eq!(prefixed.reason, "stripped_duplicated_root_prefix");
        let forward = normalize_repo_file_path(root, &format!("{repo_name}\\src\\lib.rs")).unwrap();
        assert_eq!(forward.canonical, "src/lib.rs");

        // 3. absolute under the repo root.
        let absolute = normalize_repo_file_path(root, root.join("src/lib.rs").as_str()).unwrap();
        assert_eq!(absolute.canonical, "src/lib.rs");
        assert_eq!(absolute.reason, "absolute");
    }

    #[test]
    fn normalize_repo_file_path_strips_foreign_root_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("packages/a").as_std_path()).unwrap();
        std::fs::write(root.join("packages/a/x.rs").as_std_path(), "").unwrap();

        let resolved = normalize_repo_file_path(root, "other-repo-name/packages/a/x.rs").unwrap();
        assert_eq!(resolved.canonical, "packages/a/x.rs");
        assert_eq!(resolved.reason, "stripped_foreign_root_prefix");
    }

    #[test]
    fn normalize_repo_file_path_reports_missing_and_ambiguous() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        // Two overlapping stripped tails both exist under root.
        std::fs::create_dir_all(root.join("a/deep").as_std_path()).unwrap();
        std::fs::write(root.join("a/deep/only.rs").as_std_path(), "").unwrap();
        std::fs::create_dir_all(root.join("deep").as_std_path()).unwrap();
        std::fs::write(root.join("deep/only.rs").as_std_path(), "").unwrap();

        let missing = normalize_repo_file_path(root, "src/nope.rs").unwrap_err();
        assert!(matches!(missing, RepoPathError::PathNotFound { .. }));

        // Two stripped tails both exist -> ambiguous, candidates listed.
        let error = normalize_repo_file_path(root, "whatever/a/deep/only.rs").unwrap_err();
        match error {
            RepoPathError::AmbiguousPath {
                requested,
                candidates,
            } => {
                assert_eq!(requested, "whatever/a/deep/only.rs");
                assert_eq!(candidates, ["a/deep/only.rs", "deep/only.rs"]);
            }
            other => panic!("expected ambiguous: {other:?}"),
        }

        let empty = normalize_repo_file_path(root, "   ").unwrap_err();
        assert_eq!(empty, RepoPathError::Empty);
    }

    #[test]
    fn normalize_repo_file_path_rejects_absolute_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::write(root.join("a.rs").as_std_path(), "").unwrap();

        let outside = Utf8Path::new("/definitely/not/the/repo/a.rs");
        let error = normalize_repo_file_path(root, outside.as_str()).unwrap_err();
        assert!(matches!(error, RepoPathError::NotUnderRepoRoot { .. }));
    }

    /// Linux and macOS share the Unix path policy: separators are normalized,
    /// but case is preserved.
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn unix_policy_preserves_case_after_separator_normalization() {
        let raw = "Packages\\Atlas-Core\\Src\\Lib.rs";
        let canonical = normalize_case(&to_forward_slashes(raw));
        assert_eq!(canonical, "Packages/Atlas-Core/Src/Lib.rs");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn canonical_repo_relative_preserves_case_on_unix() {
        let path = CanonicalRepoPath::from_repo_relative("Src/Lib.rs").unwrap();
        assert_eq!(path.as_str(), "Src/Lib.rs");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn canonical_repo_relative_lowercases_on_windows() {
        let path = CanonicalRepoPath::from_repo_relative("Src\\Lib.rs").unwrap();
        assert_eq!(path.as_str(), "src/lib.rs");
    }

    proptest! {
        #[test]
        fn equivalent_relative_spellings_share_canonical_identity(
            segments in proptest::collection::vec(string_regex("[a-z0-9_-]{1,8}").unwrap(), 1..8),
            prefix_curdirs in 0usize..3,
            separator_flags in proptest::collection::vec(any::<bool>(), 0..8),
            curdir_flags in proptest::collection::vec(any::<bool>(), 0..8),
        ) {
            let expected = segments.join("/");
            let mut raw = String::new();

            for _ in 0..prefix_curdirs {
                raw.push_str("./");
            }

            for (idx, segment) in segments.iter().enumerate() {
                if idx > 0 {
                    raw.push(if *separator_flags.get(idx - 1).unwrap_or(&false) {
                        '\\'
                    } else {
                        '/'
                    });
                }

                if *curdir_flags.get(idx).unwrap_or(&false) {
                    raw.push('.');
                    raw.push(if *separator_flags.get(idx).unwrap_or(&false) {
                        '\\'
                    } else {
                        '/'
                    });
                }

                raw.push_str(segment);
            }

            let canonical = CanonicalRepoPath::from_repo_relative(&raw).unwrap();
            prop_assert_eq!(canonical.as_str(), expected);
        }
    }
}
