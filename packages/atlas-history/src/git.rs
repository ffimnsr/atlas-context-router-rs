//! Thin, deterministic wrappers around git plumbing commands.
//!
//! All functions take the repo root as a `&std::path::Path` and return
//! `anyhow::Result`.  Output is parsed with no LLM involvement and no
//! branch-name assumptions.  Every caller must validate SHAs before use.
//!
//! Supported sub-commands: rev-parse, log, show, ls-tree, diff-tree, cat-file.

use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

/// Parsed single-commit metadata produced by `git log`.
#[derive(Debug, Clone)]
pub struct GitCommitMeta {
    pub sha: String,
    /// First parent SHA, empty string if root commit.
    pub parent_sha: Option<String>,
    pub author_name: String,
    pub author_email: String,
    /// Unix timestamp seconds.
    pub author_time: i64,
    /// Unix timestamp seconds.
    pub committer_time: i64,
    /// First line of the commit message.
    pub subject: String,
    /// Full raw commit message body (may be empty).
    pub body: String,
}

/// One entry from `git ls-tree`.
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub object_type: String,
    pub object_hash: String,
    pub file_path: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TagRef {
    pub name: String,
    pub commit_sha: String,
}

// ── internal helpers ──────────────────────────────────────────────────────────

fn run(repo: &Path, args: &[&str]) -> Result<Output> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .with_context(|| format!("spawn git {args:?}"))?;
    Ok(out)
}

fn ok_stdout(out: Output, cmd: &str) -> Result<String> {
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("git {cmd} failed: {stderr}");
    }
    String::from_utf8(out.stdout).with_context(|| format!("git {cmd} output not valid UTF-8"))
}

// ── rev-parse ─────────────────────────────────────────────────────────────────

/// Resolve any ref (branch, tag, `HEAD`, SHA) to a full 40-char SHA.
///
/// Returns an error when the ref is missing, which handles shallow clones
/// and detached HEAD correctly via the git error message.
pub fn rev_parse(repo: &Path, r#ref: &str) -> Result<String> {
    let out = run(repo, &["rev-parse", "--verify", r#ref])?;
    let sha = ok_stdout(out, "rev-parse")?.trim().to_owned();
    validate_sha(&sha).context("rev-parse returned invalid SHA")?;
    Ok(sha)
}

/// Resolve merge base of two refs to a full 40-char SHA.
pub fn merge_base(repo: &Path, left: &str, right: &str) -> Result<String> {
    let out = run(repo, &["merge-base", left, right])?;
    let sha = ok_stdout(out, "merge-base")?.trim().to_owned();
    validate_sha(&sha).context("merge-base returned invalid SHA")?;
    Ok(sha)
}

/// Check if the repo is a shallow clone.
pub fn is_shallow(repo: &Path) -> Result<bool> {
    let out = run(repo, &["rev-parse", "--is-shallow-repository"])?;
    let text = ok_stdout(out, "rev-parse --is-shallow-repository")?;
    Ok(text.trim() == "true")
}

// ── git log ───────────────────────────────────────────────────────────────────

/// Enumerate commits reachable from `start_ref` (most-recent first).
///
/// `max_count` limits the number of commits; `None` returns the full
/// reachable history.  The record separator `\x1e` (ASCII RS) is used so
/// multi-line commit messages parse cleanly.
pub fn log_commits(
    repo: &Path,
    start_ref: &str,
    max_count: Option<usize>,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<Vec<GitCommitMeta>> {
    // Format: RS-delimited records, each field separated by NUL.
    // %H  %P  %an  %ae  %at  %ct  %s  %b
    let format = "--format=%x1e%H%x00%P%x00%an%x00%ae%x00%at%x00%ct%x00%s%x00%b";
    let mut args: Vec<String> = vec!["log".into(), format.into()];

    if let Some(n) = max_count {
        args.push(format!("--max-count={n}"));
    }
    if let Some(s) = since {
        args.push(format!("--after={s}"));
    }
    if let Some(u) = until {
        args.push(format!("--before={u}"));
    }
    args.push(start_ref.to_owned());

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run(repo, &arg_refs)?;
    let raw = ok_stdout(out, "log")?;
    parse_log_output(&raw)
}

/// Enumerate an explicit list of commit SHAs.  Each SHA is resolved
/// individually with `git log -1`.
pub fn log_commits_explicit(repo: &Path, shas: &[String]) -> Result<Vec<GitCommitMeta>> {
    let mut result = Vec::with_capacity(shas.len());
    for sha in shas {
        validate_sha(sha).with_context(|| format!("invalid SHA in explicit list: {sha}"))?;
        let format = "--format=%x1e%H%x00%P%x00%an%x00%ae%x00%at%x00%ct%x00%s%x00%b";
        let out = run(repo, &["log", "-1", format, sha])?;
        let raw = ok_stdout(out, "log -1")?;
        let mut commits = parse_log_output(&raw)?;
        if commits.is_empty() {
            bail!("commit not found or not reachable: {sha}");
        }
        result.push(commits.remove(0));
    }
    Ok(result)
}

fn parse_log_output(raw: &str) -> Result<Vec<GitCommitMeta>> {
    let mut commits = Vec::new();
    for record in raw.split('\x1e') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.splitn(8, '\x00').collect();
        if fields.len() < 7 {
            continue; // incomplete record, skip
        }
        let sha = fields[0].trim().to_owned();
        if sha.is_empty() {
            continue;
        }
        validate_sha(&sha).with_context(|| format!("invalid SHA in log output: {sha}"))?;

        let parents_raw = fields[1].trim();
        let parent_sha = parents_raw
            .split_whitespace()
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let author_time: i64 = fields[4]
            .trim()
            .parse()
            .with_context(|| format!("bad author_time for {sha}"))?;
        let committer_time: i64 = fields[5]
            .trim()
            .parse()
            .with_context(|| format!("bad committer_time for {sha}"))?;

        commits.push(GitCommitMeta {
            sha,
            parent_sha,
            author_name: fields[2].trim().to_owned(),
            author_email: fields[3].trim().to_owned(),
            author_time,
            committer_time,
            subject: fields[6].trim().to_owned(),
            body: if fields.len() > 7 {
                fields[7].trim_end().to_owned()
            } else {
                String::new()
            },
        });
    }
    Ok(commits)
}

// ── git show ──────────────────────────────────────────────────────────────────

/// Read the raw content of `path` at `commit_sha` from the object store.
///
/// Returns `None` when the path does not exist at that commit (deleted or
/// never added).
pub fn show_file(repo: &Path, commit_sha: &str, path: &str) -> Result<Option<Vec<u8>>> {
    validate_sha(commit_sha)?;
    let spec = format!("{commit_sha}:{path}");
    let out = run(repo, &["show", &spec])?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // "does not exist in" or "exists on disk, but not in" → file absent
        if stderr.contains("does not exist") || stderr.contains("path not in") {
            return Ok(None);
        }
        bail!("git show {spec} failed: {stderr}");
    }
    Ok(Some(out.stdout))
}

// ── git ls-tree ───────────────────────────────────────────────────────────────

/// List all blobs (files) tracked at `commit_sha`, recursively.
pub fn ls_tree(repo: &Path, commit_sha: &str) -> Result<Vec<TreeEntry>> {
    validate_sha(commit_sha)?;
    let out = run(repo, &["ls-tree", "-r", "--full-tree", commit_sha])?;
    let raw = ok_stdout(out, "ls-tree")?;
    parse_ls_tree(&raw)
}

/// List tracked entries for selected repo-relative paths at `commit_sha`.
pub fn ls_tree_paths(repo: &Path, commit_sha: &str, paths: &[String]) -> Result<Vec<TreeEntry>> {
    validate_sha(commit_sha)?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec![
        "ls-tree".to_owned(),
        "-r".to_owned(),
        "--full-tree".to_owned(),
        commit_sha.to_owned(),
        "--".to_owned(),
    ];
    args.extend(paths.iter().cloned());
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let out = run(repo, &arg_refs)?;
    let raw = ok_stdout(out, "ls-tree")?;
    parse_ls_tree(&raw)
}

fn parse_ls_tree(raw: &str) -> Result<Vec<TreeEntry>> {
    let mut entries = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Format: "<mode> <type> <hash>\t<path>"
        let (meta, path) = line
            .split_once('\t')
            .ok_or_else(|| anyhow::anyhow!("malformed ls-tree line: {line}"))?;
        let parts: Vec<&str> = meta.split_whitespace().collect();
        if parts.len() < 3 {
            bail!("malformed ls-tree meta: {meta}");
        }
        entries.push(TreeEntry {
            mode: parts[0].to_owned(),
            object_type: parts[1].to_owned(),
            object_hash: parts[2].to_owned(),
            file_path: path.to_owned(),
        });
    }
    Ok(entries)
}

// ── git diff-tree ─────────────────────────────────────────────────────────────

/// Return paths changed between two commits (or between a commit and its
/// parent when `parent_sha` is `None`).
///
/// Returns `(old_path, new_path, status_char)` tuples where `status_char` is
/// the single letter git uses: A, M, D, R, C, etc.
pub fn diff_tree_files(
    repo: &Path,
    commit_sha: &str,
    parent_sha: Option<&str>,
) -> Result<Vec<(String, String, char)>> {
    validate_sha(commit_sha)?;
    if let Some(p) = parent_sha {
        validate_sha(p)?;
    }

    let mut args = vec![
        "diff-tree",
        "--no-commit-id",
        "-r",
        "--name-status",
        "-M", // detect renames
    ];
    let refs: String;
    if let Some(p) = parent_sha {
        refs = format!("{p} {commit_sha}");
        args.extend(refs.split_whitespace());
    } else {
        args.push(commit_sha);
    }

    let out = run(repo, &args)?;
    let raw = ok_stdout(out, "diff-tree")?;
    let mut result = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.splitn(3, '\t').collect();
        if cols.is_empty() {
            continue;
        }
        let status = cols[0].chars().next().unwrap_or('?');
        let old_path = cols.get(1).copied().unwrap_or("").to_owned();
        let new_path = cols.get(2).copied().unwrap_or(&old_path).to_owned();
        result.push((old_path, new_path, status));
    }
    Ok(result)
}

// ── git cat-file ──────────────────────────────────────────────────────────────

/// Return the raw content of a git object by hash.
pub fn cat_file_blob(repo: &Path, object_hash: &str) -> Result<Vec<u8>> {
    let out = run(repo, &["cat-file", "blob", object_hash])?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("git cat-file {object_hash} failed: {stderr}");
    }
    Ok(out.stdout)
}

/// Return the root tree hash for a commit SHA.
pub fn commit_tree_hash(repo: &Path, commit_sha: &str) -> Result<String> {
    validate_sha(commit_sha)?;
    let out = run(repo, &["cat-file", "-p", commit_sha])?;
    let raw = ok_stdout(out, "cat-file -p")?;
    for line in raw.lines() {
        if let Some(hash) = line.strip_prefix("tree ") {
            return Ok(hash.trim().to_owned());
        }
    }
    bail!("no tree line in cat-file output for {commit_sha}");
}

/// List all tag refs resolved to commit SHAs.
///
/// Annotated tags are dereferenced via `show-ref --tags -d`; lightweight tags
/// are included as-is. Duplicate commit/name pairs are removed.
pub fn list_tag_refs(repo: &Path) -> Result<Vec<TagRef>> {
    let out = run(repo, &["show-ref", "--tags", "-d"])?;
    let raw = ok_stdout(out, "show-ref --tags -d")?;
    let mut seen = std::collections::BTreeSet::new();
    let mut tags = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(sha) = parts.next() else {
            continue;
        };
        validate_sha(sha).with_context(|| format!("invalid tag SHA in show-ref output: {sha}"))?;
        let Some(raw_ref) = parts.next() else {
            continue;
        };
        let name = raw_ref
            .trim_start_matches("refs/tags/")
            .trim_end_matches("^{}")
            .to_owned();
        if seen.insert((name.clone(), sha.to_owned())) {
            tags.push(TagRef {
                name,
                commit_sha: sha.to_owned(),
            });
        }
    }
    Ok(tags)
}

// ── SHA validation ─────────────────────────────────────────────────────────────

/// Reject anything that is not a 40-char hex string.
///
/// This is a security boundary: git commands built from caller-supplied SHAs
/// must never execute arbitrary shell tokens.  We validate before passing to
/// `std::process::Command`, which does not use a shell, but an explicit
/// safeguard is still required to prevent accidental misuse.
pub fn validate_sha(sha: &str) -> Result<()> {
    let sha = sha.trim();
    if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(());
    }
    bail!("invalid git SHA (must be 40 hex chars): {:?}", sha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_sha_accepts_valid() {
        let sha = "a".repeat(40);
        assert!(validate_sha(&sha).is_ok());
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert!(validate_sha(sha).is_ok());
    }

    #[test]
    fn validate_sha_rejects_short() {
        assert!(validate_sha("deadbeef").is_err());
    }

    #[test]
    fn validate_sha_rejects_non_hex() {
        let bad = "z".repeat(40);
        assert!(validate_sha(&bad).is_err());
    }

    #[test]
    fn validate_sha_rejects_shell_injection() {
        let bad = format!("{} && rm -rf /", "a".repeat(40));
        assert!(validate_sha(&bad).is_err());
    }

    #[test]
    fn parse_ls_tree_basic() {
        let raw = "100644 blob abc123def456abc123def456abc123def456abc123\tsrc/main.rs\n";
        let entries = parse_ls_tree(raw).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_path, "src/main.rs");
        assert_eq!(entries[0].object_type, "blob");
    }

    #[test]
    fn parse_log_output_basic() {
        // Craft a minimal log record using RS/NUL separators.
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let parent = "fedcba9876543210fedcba9876543210fedcba98";
        let raw = format!(
            "\x1e{sha}\x00{parent}\x00Alice\x00alice@example.com\x001700000000\x001700000001\x00initial commit\x00body line\n"
        );
        let commits = parse_log_output(&raw).unwrap();
        assert_eq!(commits.len(), 1);
        let c = &commits[0];
        assert_eq!(c.sha, sha);
        assert_eq!(c.parent_sha.as_deref(), Some(parent));
        assert_eq!(c.author_name, "Alice");
        assert_eq!(c.author_time, 1_700_000_000);
        assert_eq!(c.subject, "initial commit");
    }
}
