use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};

use crate::path::{CanonicalRepoPath, canonical_filesystem_path, git_cmd};
use std::collections::HashSet;

/// Default maximum file size in bytes (10 MiB).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Bytes examined for binary detection (8 KiB).
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Atlas-specific ignore file name.
const ATLASIGNORE_FILE: &str = ".atlasignore";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollectFilesStats {
    pub skipped_unsupported: usize,
    pub skipped_by_size: usize,
}

enum CheckFileOutcome {
    Accept,
    SkipBySize,
    SkipOther,
}

/// Directory/path patterns that are always ignored regardless of `.atlasignore`.
///
/// These cover well-known build artefact and dependency directories that should
/// never be part of the code graph even if they are accidentally tracked by git.
pub const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
    ".atlas",
    ".git",
    "node_modules",
    "vendor",
    "dist",
    "build",
    ".next",
    "target",
    ".venv",
    "__pycache__",
];

/// Collect all git-tracked files under `repo_root`, filtering out:
/// - files larger than `max_bytes` (defaults to [`DEFAULT_MAX_FILE_BYTES`])
/// - binary files (null byte in first 8 KiB)
/// - symlinks (skipped — git tracks symlinks as pointer objects, not content;
///   following symlinks outside the repo root would be a path-escape risk)
/// - paths matched by [`DEFAULT_IGNORE_PATTERNS`]
/// - paths matched by patterns in `.atlasignore` at the repo root
/// - uninitialized submodules (skipped with a warning)
///
/// Returned paths are repo-relative, forward-slash separated. Initialized
/// submodules are scanned recursively and prefixed with their submodule path.
/// Recursion is guarded by canonical filesystem repo roots so symlink aliases
/// or malformed nested submodule topology cannot loop forever.
///
/// ## Submodule `.gitignore` semantics
///
/// Each repo is scanned with its own `git ls-files --exclude-standard`, so
/// each submodule's `.gitignore` (and `.git/info/exclude`) is applied
/// independently. The *parent* repo's `.gitignore` is **not** propagated into
/// a submodule, which mirrors git's own behaviour. Atlas-level ignore rules
/// (`.atlasignore` and [`DEFAULT_IGNORE_PATTERNS`]) are checked against the
/// combined repo-relative path that includes the submodule prefix, so a
/// pattern like `node_modules/` will suppress `my-submodule/node_modules/foo`
/// as expected.
///
/// ## Path-identity invariants
///
/// All paths returned by this function go through
/// [`CanonicalRepoPath::from_git_diff_path`], which applies NFC Unicode
/// normalisation, converts backslashes to forward slashes, collapses `.` and
/// `..` components, and lowercases on Windows. This matches the invariants
/// enforced by every other Atlas path boundary.
pub fn collect_files(repo_root: &Utf8Path, max_bytes: Option<u64>) -> Result<Vec<Utf8PathBuf>> {
    let (files, _) = collect_supported_files(repo_root, max_bytes, |_| true)?;
    Ok(files)
}

/// Collect all git-tracked files under `repo_root`, applying the same filters
/// as [`collect_files`] plus a caller-supplied support predicate.
///
/// Returns the accepted repo-relative paths and the number of files skipped by
/// the support predicate (typically unsupported extensions/languages).
pub fn collect_supported_files<F>(
    repo_root: &Utf8Path,
    max_bytes: Option<u64>,
    mut supports_path: F,
) -> Result<(Vec<Utf8PathBuf>, usize)>
where
    F: FnMut(&Utf8Path) -> bool,
{
    let (results, stats) =
        collect_supported_files_with_stats(repo_root, max_bytes, |path| supports_path(path))?;
    Ok((results, stats.skipped_unsupported))
}

pub fn collect_supported_files_with_stats<F>(
    repo_root: &Utf8Path,
    max_bytes: Option<u64>,
    mut supports_path: F,
) -> Result<(Vec<Utf8PathBuf>, CollectFilesStats)>
where
    F: FnMut(&Utf8Path) -> bool,
{
    let threshold = max_bytes.unwrap_or(DEFAULT_MAX_FILE_BYTES);
    let raw = git_ls_files(repo_root)?;
    let default_patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
        .iter()
        .map(|s| format!("{}/", s))
        .collect();
    let ignore_patterns = load_atlasignore(repo_root);
    let mut results = Vec::with_capacity(raw.len());
    let mut stats = CollectFilesStats::default();

    for rel_path in raw {
        if should_ignore(rel_path.as_str(), &default_patterns) {
            tracing::debug!("skipping '{}': matched default ignore", rel_path);
            continue;
        }
        if should_ignore(rel_path.as_str(), &ignore_patterns) {
            tracing::debug!("skipping '{}': matched .atlasignore", rel_path);
            continue;
        }
        if !supports_path(rel_path.as_path()) {
            stats.skipped_unsupported += 1;
            tracing::debug!("skipping '{}': unsupported extension", rel_path);
            continue;
        }
        let abs = repo_root.join(&rel_path);
        match check_file(&abs, threshold) {
            Ok(CheckFileOutcome::Accept) => results.push(rel_path),
            Ok(CheckFileOutcome::SkipBySize) => {
                stats.skipped_by_size += 1;
                tracing::debug!("skipping '{}': size budget", rel_path);
            }
            Ok(CheckFileOutcome::SkipOther) => {
                tracing::debug!("skipping '{}': too large, binary, or symlink", rel_path);
            }
            Err(e) => {
                tracing::warn!("skipping '{}': {}", rel_path, e);
            }
        }
    }

    Ok((results, stats))
}

/// Read `.atlasignore` from the repo root and return non-empty, non-comment
/// lines as ignore patterns.
pub fn load_atlasignore(repo_root: &Utf8Path) -> Vec<String> {
    let path = repo_root.join(ATLASIGNORE_FILE);
    let content = match std::fs::read_to_string(path.as_std_path()) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_owned())
        .collect()
}

/// Return `true` if `path` (repo-relative, forward-slash) matches any of the
/// given glob patterns.
///
/// Pattern semantics (gitignore-like subset):
/// - `*`  matches any sequence of characters that does not contain `/`
/// - `**` matches any sequence including `/`
/// - `?`  matches exactly one character that is not `/`
/// - A trailing `/` anchors the pattern to match directory prefixes
/// - A pattern without `/` (other than trailing) matches any path component;
///   a pattern containing `/` is anchored to the start
pub fn should_ignore(path: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        let pat = pattern.trim_end_matches('/');
        let trailing_slash = pattern.ends_with('/');

        // If pattern contains no '/' (after stripping trailing one), treat it as
        // a filename/basename pattern matched against any path segment or the full path.
        if !pat.contains('/') {
            // Match against the full path or against each component.
            if glob_match(pat, path) {
                return true;
            }
            // Also match any component individually (e.g. "*.pyc" should match "a/b/foo.pyc").
            for component in path.split('/') {
                if glob_match(pat, component) {
                    return true;
                }
            }
        } else {
            // Pattern is anchored to repo root.
            if trailing_slash {
                // Match as directory prefix: path starts with pat + "/"
                if path.starts_with(&format!("{}/", pat)) || glob_match(pat, path) {
                    return true;
                }
            } else if glob_match(pat, path) {
                return true;
            }
        }
    }
    false
}

/// Match `pattern` against `text` using `*`, `**`, and `?` globs.
///
/// `*` does not cross `/` boundaries; `**` matches across them.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_inner(pat: &[u8], text: &[u8]) -> bool {
    match (pat.first(), text.first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some(b'?'), Some(&tc)) if tc != b'/' => glob_match_inner(&pat[1..], &text[1..]),
        (Some(b'?'), _) => false,
        (Some(b'*'), _) => {
            // Check for `**`
            if pat.get(1) == Some(&b'*') {
                let rest_pat = if pat.get(2) == Some(&b'/') {
                    &pat[3..]
                } else {
                    &pat[2..]
                };
                // Try matching `**` against every suffix of text.
                for i in 0..=text.len() {
                    if glob_match_inner(rest_pat, &text[i..]) {
                        return true;
                    }
                }
                false
            } else {
                // Single `*`: match zero or more non-'/' chars.
                let rest_pat = &pat[1..];
                for i in 0..=text.len() {
                    if text[..i].contains(&b'/') {
                        break;
                    }
                    if glob_match_inner(rest_pat, &text[i..]) {
                        return true;
                    }
                }
                false
            }
        }
        (Some(&pc), Some(&tc)) if pc == tc => glob_match_inner(&pat[1..], &text[1..]),
        _ => false,
    }
}

/// Run `git ls-files` and return repo-relative paths.
fn git_ls_files(repo_root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut visited = HashSet::new();
    git_ls_files_with_guard(repo_root, &mut visited)
}

fn git_ls_files_with_guard(
    repo_root: &Utf8Path,
    visited: &mut HashSet<Utf8PathBuf>,
) -> Result<Vec<Utf8PathBuf>> {
    if !register_repo_scan_root(visited, repo_root)? {
        tracing::warn!(
            "skipping repo scan for '{}': canonical root already visited; possible symlink loop",
            repo_root
        );
        return Ok(Vec::new());
    }

    let mut paths = git_ls_files_in_repo(repo_root)?;

    for submodule_path in git_submodule_paths(repo_root)? {
        let submodule_root = repo_root.join(&submodule_path);
        if !submodule_root.is_dir() || !submodule_root.join(".git").exists() {
            tracing::warn!(
                "skipping submodule '{}': working tree is not initialized",
                submodule_path
            );
            continue;
        }

        match git_ls_files_with_guard(&submodule_root, visited) {
            Ok(submodule_files) => {
                paths.extend(
                    submodule_files
                        .into_iter()
                        .map(|path| submodule_path.join(path)),
                );
            }
            Err(error) => {
                tracing::warn!("skipping submodule '{}': {}", submodule_path, error);
            }
        }
    }

    Ok(paths)
}

fn register_repo_scan_root(
    visited: &mut HashSet<Utf8PathBuf>,
    repo_root: &Utf8Path,
) -> Result<bool> {
    let canonical = canonical_filesystem_path(repo_root)
        .with_context(|| format!("cannot canonicalize repo root '{repo_root}'"))?;
    Ok(visited.insert(canonical))
}

fn git_ls_files_in_repo(repo_root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let output = git_cmd()
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .current_dir(repo_root)
        .output()
        .context("failed to run git ls-files")?;

    anyhow::ensure!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Null-separated output from `-z` flag.
    let stdout =
        std::str::from_utf8(&output.stdout).context("git ls-files output is not valid UTF-8")?;

    let paths = stdout
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| CanonicalRepoPath::from_git_diff_path(s).map(CanonicalRepoPath::into_path_buf))
        .collect::<std::result::Result<Vec<Utf8PathBuf>, _>>()?;

    Ok(paths)
}

fn git_submodule_paths(repo_root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let gitmodules_path = repo_root.join(".gitmodules");
    if !gitmodules_path.is_file() {
        return Ok(Vec::new());
    }

    let output = git_cmd()
        .args([
            "config",
            "-z",
            "--file",
            ".gitmodules",
            "--get-regexp",
            r"^submodule\..*\.path$",
        ])
        .current_dir(repo_root)
        .output()
        .context("failed to read submodule paths from .gitmodules")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if output.status.code() == Some(1) && stderr.is_empty() {
            return Ok(Vec::new());
        }
        anyhow::bail!("git config for submodule paths failed: {stderr}");
    }

    let stdout =
        std::str::from_utf8(&output.stdout).context("submodule path output is not valid UTF-8")?;
    let mut submodules = Vec::new();

    for entry in stdout.split('\0').filter(|entry| !entry.is_empty()) {
        let Some((_, path)) = entry.split_once('\n') else {
            anyhow::bail!("malformed submodule path entry: {entry}");
        };
        submodules.push(CanonicalRepoPath::from_git_diff_path(path)?.into_path_buf());
    }

    Ok(submodules)
}

/// Return `true` if the file should be included (exists, small enough, not binary, not a symlink).
///
/// Symlink policy: symlinks are **skipped**. Git tracks symlinks as special pointer
/// objects; reading the target's bytes would produce content that does not match
/// what git indexes, and following symlinks outside the repo root is a security
/// concern. If a symlink target should be analysed, it should be tracked directly.
fn check_file(abs: &Utf8Path, max_bytes: u64) -> Result<CheckFileOutcome> {
    // Use symlink_metadata so we can detect symlinks without following them.
    let sym_meta = abs
        .as_std_path()
        .symlink_metadata()
        .with_context(|| format!("symlink_metadata for '{abs}'"))?;

    if sym_meta.file_type().is_symlink() {
        tracing::debug!("skipping '{}': symlink", abs);
        return Ok(CheckFileOutcome::SkipOther);
    }
    if !sym_meta.is_file() {
        return Ok(CheckFileOutcome::SkipOther);
    }
    if sym_meta.len() > max_bytes {
        return Ok(CheckFileOutcome::SkipBySize);
    }
    if is_binary(abs)? {
        return Ok(CheckFileOutcome::SkipOther);
    }
    Ok(CheckFileOutcome::Accept)
}

/// Sniff first `BINARY_SNIFF_BYTES` for null bytes.
fn is_binary(path: &Utf8Path) -> Result<bool> {
    use std::io::Read;
    let mut f =
        std::fs::File::open(path.as_std_path()).with_context(|| format!("open '{path}'"))?;
    let mut buf = vec![0u8; BINARY_SNIFF_BYTES];
    let n = f.read(&mut buf).with_context(|| format!("read '{path}'"))?;
    Ok(buf[..n].contains(&0u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_rust_sources_in_workspace() {
        let root = Utf8Path::new(
            // Go up 2 levels from packages/atlas-repo to the workspace root.
            concat!(env!("CARGO_MANIFEST_DIR"), "/../.."),
        );
        // Canonicalize to resolve any `..` components.
        let abs = root
            .canonicalize_utf8()
            .expect("canonicalize workspace root");

        let files = collect_files(&abs, None).expect("collect_files");
        assert!(!files.is_empty(), "should find at least one tracked file");
        assert!(
            files.iter().any(|p| p.extension() == Some("rs")),
            "should include .rs files"
        );
    }

    #[test]
    fn collect_supported_files_skips_unsupported_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();

        let status = git_cmd()
            .args(["init", "--quiet"])
            .current_dir(root)
            .status()
            .expect("git init");
        assert!(status.success(), "git init should succeed");

        std::fs::write(root.join("main.rs").as_std_path(), "fn main() {}\n").unwrap();
        std::fs::write(root.join("notes.md").as_std_path(), "# notes\n").unwrap();
        std::fs::write(root.join("config.yaml").as_std_path(), "name: atlas\n").unwrap();

        let (files, skipped_unsupported) = collect_supported_files(root, None, |path| {
            matches!(path.extension(), Some("rs") | Some("py"))
        })
        .expect("collect_supported_files");

        assert_eq!(skipped_unsupported, 2);
        assert_eq!(files, vec![Utf8PathBuf::from("main.rs")]);
    }

    #[test]
    fn collect_supported_files_skips_internal_atlas_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();

        let status = git_cmd()
            .args(["init", "--quiet"])
            .current_dir(root)
            .status()
            .expect("git init");
        assert!(status.success(), "git init should succeed");

        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();
        std::fs::create_dir_all(root.join(".atlas").as_std_path()).unwrap();
        std::fs::write(root.join("src/main.rs").as_std_path(), "fn main() {}\n").unwrap();
        std::fs::write(
            root.join(".atlas/config.toml").as_std_path(),
            "[build]\nparse_batch_size = 8\n",
        )
        .unwrap();

        let status = git_cmd()
            .args(["add", "."])
            .current_dir(root)
            .status()
            .expect("git add");
        assert!(status.success(), "git add should succeed");

        let (files, skipped_unsupported) = collect_supported_files(root, None, |path| {
            matches!(path.extension(), Some("rs") | Some("toml"))
        })
        .expect("collect_supported_files");

        assert_eq!(skipped_unsupported, 0);
        assert_eq!(files, vec![Utf8PathBuf::from("src/main.rs")]);
    }

    #[test]
    fn binary_detection_rejects_null_bytes() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let p = Utf8Path::from_path(dir.path()).unwrap().join("bin.dat");
        let mut f = std::fs::File::create(p.as_std_path()).unwrap();
        f.write_all(&[0x00, 0x01, 0x02]).unwrap();
        assert!(is_binary(&p).unwrap());
    }

    // --- glob_match ----------------------------------------------------------

    #[test]
    fn glob_exact_match() {
        assert!(glob_match("foo.rs", "foo.rs"));
        assert!(!glob_match("foo.rs", "bar.rs"));
    }

    #[test]
    fn glob_star_matches_no_slash() {
        assert!(glob_match("*.rs", "foo.rs"));
        assert!(!glob_match("*.rs", "src/foo.rs"));
    }

    #[test]
    fn glob_double_star_crosses_slash() {
        assert!(glob_match("**/*.rs", "src/foo.rs"));
        assert!(glob_match("**/*.rs", "a/b/c/foo.rs"));
        assert!(!glob_match("**/*.rs", "foo.txt"));
    }

    #[test]
    fn glob_question_mark() {
        assert!(glob_match("fo?.rs", "foo.rs"));
        assert!(!glob_match("fo?.rs", "fooo.rs"));
        assert!(!glob_match("fo?.rs", "fo/.rs"));
    }

    // --- should_ignore -------------------------------------------------------

    #[test]
    fn should_ignore_basename_pattern() {
        let patterns: Vec<String> = vec!["*.pyc".to_string()];
        assert!(should_ignore("src/app.pyc", &patterns));
        assert!(should_ignore("app.pyc", &patterns));
        assert!(!should_ignore("app.py", &patterns));
    }

    #[test]
    fn should_ignore_anchored_dir_pattern() {
        let patterns: Vec<String> = vec!["build/".to_string()];
        assert!(should_ignore("build/main.rs", &patterns));
        assert!(!should_ignore("src/build.rs", &patterns));
    }

    #[test]
    fn should_ignore_anchored_path_pattern() {
        let patterns: Vec<String> = vec!["generated/proto".to_string()];
        assert!(should_ignore("generated/proto", &patterns));
        assert!(!should_ignore("src/generated/proto", &patterns));
    }

    // --- DEFAULT_IGNORE_PATTERNS ---------------------------------------------

    #[test]
    fn default_patterns_block_node_modules() {
        let patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|s| format!("{}/", s))
            .collect();
        assert!(should_ignore("node_modules/lodash/index.js", &patterns));
        assert!(should_ignore(
            "vendor/github.com/pkg/errors/errors.go",
            &patterns
        ));
        assert!(should_ignore("target/debug/atlas", &patterns));
        assert!(should_ignore(".venv/lib/python3.11/site.py", &patterns));
        assert!(should_ignore("__pycache__/main.cpython-311.pyc", &patterns));
        assert!(should_ignore("dist/bundle.js", &patterns));
        assert!(should_ignore("build/output.o", &patterns));
        assert!(should_ignore(".next/server/pages/index.js", &patterns));
    }

    #[test]
    fn default_patterns_allow_normal_src() {
        let patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|s| format!("{}/", s))
            .collect();
        assert!(!should_ignore("src/main.rs", &patterns));
        assert!(!should_ignore("packages/atlas-core/src/lib.rs", &patterns));
    }

    // --- symlink policy ------------------------------------------------------

    #[test]
    fn symlink_is_skipped() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.txt");
        std::fs::write(&target, b"hello").unwrap();
        let link = dir.path().join("link.txt");
        symlink(&target, &link).unwrap();
        let link_utf8 = Utf8Path::from_path(&link).unwrap();
        // Symlinks must be rejected.
        assert!(matches!(
            check_file(link_utf8, DEFAULT_MAX_FILE_BYTES).unwrap(),
            CheckFileOutcome::SkipOther
        ));
        // The real file is accepted.
        let target_utf8 = Utf8Path::from_path(&target).unwrap();
        assert!(matches!(
            check_file(target_utf8, DEFAULT_MAX_FILE_BYTES).unwrap(),
            CheckFileOutcome::Accept
        ));
    }

    #[test]
    fn repo_scan_guard_dedupes_same_physical_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        let alias = dir.path().join("alias");
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(dir.path(), &alias).unwrap();
            let alias = Utf8Path::from_path(&alias).unwrap();
            let mut visited = HashSet::new();
            assert!(register_repo_scan_root(&mut visited, root).unwrap());
            assert!(!register_repo_scan_root(&mut visited, alias).unwrap());
        }
        #[cfg(not(unix))]
        {
            let mut visited = HashSet::new();
            assert!(register_repo_scan_root(&mut visited, root).unwrap());
            assert!(!register_repo_scan_root(&mut visited, root).unwrap());
        }
    }

    #[test]
    fn should_ignore_comments_and_blank_lines_skipped() {
        let patterns: Vec<String> =
            vec!["# comment".to_string(), "".to_string(), "*.log".to_string()];
        assert!(!should_ignore("src/main.rs", &patterns));
        assert!(should_ignore("debug.log", &patterns));
    }

    #[test]
    fn load_atlasignore_reads_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        let ignore_path = root.join(".atlasignore");
        let mut f = std::fs::File::create(ignore_path.as_std_path()).unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f, "*.log").unwrap();
        writeln!(f, "build/").unwrap();

        let patterns = load_atlasignore(root);
        assert_eq!(patterns, vec!["*.log", "build/"]);
    }

    #[test]
    fn load_atlasignore_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        let patterns = load_atlasignore(root);
        assert!(patterns.is_empty());
    }
}
