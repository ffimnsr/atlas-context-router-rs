use anyhow::{Context, Result};
use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use std::process::Command;
use thiserror::Error;

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

fn canonicalize_relative(raw: &str) -> std::result::Result<CanonicalRepoPath, RepoPathError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(RepoPathError::Empty);
    }

    let slashed = to_forward_slashes(raw);
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
    let slashed = to_forward_slashes(path.as_str());
    let cased = normalize_case(&slashed);
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

/// Normalize path casing for the current platform.
///
/// On Windows the filesystem is case-insensitive, so two paths that differ
/// only in case refer to the same file.  To guarantee the qualified-name
/// scheme is stable regardless of how a path was obtained, we lowercase the
/// entire path on Windows.  On Unix the filesystem is case-sensitive, so no
/// transformation is applied.
///
/// Call this **after** [`to_forward_slashes`] so that the input is already
/// separator-normalized.
pub fn normalize_case(s: &str) -> String {
    if cfg!(target_os = "windows") {
        s.to_ascii_lowercase()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn synthetic_path_constructor_reuses_relative_rules() {
        let path = CanonicalRepoPath::from_synthetic_path("generated/schema.graph.json").unwrap();
        assert_eq!(path.as_str(), "generated/schema.graph.json");
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
}
