use anyhow::{Context, Result};
use atlas_core::model::{ChangeType, ChangedFile};
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::HashSet;

use crate::path::{CanonicalRepoPath, git_cmd, to_forward_slashes};

/// Specification of what to diff against.
#[derive(Debug, Clone)]
pub enum DiffTarget {
    /// Compare a base ref to HEAD, e.g. `origin/main...HEAD` or just `origin/main`.
    BaseRef(String),
    /// Changes staged in the index (`git diff --cached`).
    Staged,
    /// Unstaged working-tree changes (`git diff`).
    WorkingTree,
}

impl DiffTarget {
    /// Build the git arguments for `git diff --name-status -z`.
    fn git_args(&self) -> Vec<&str> {
        match self {
            DiffTarget::BaseRef(r) => vec!["diff", "--name-status", "-z", r],
            DiffTarget::Staged => vec!["diff", "--cached", "--name-status", "-z"],
            DiffTarget::WorkingTree => vec!["diff", "--name-status", "-z"],
        }
    }
}

/// Return the list of changed files according to `target`.
///
/// Returned paths are the values reported by git (relative to `repo_root`).
/// Deleted files have `ChangeType::Deleted`; renamed/copied files carry the
/// new path in `path` and the old path in `old_path`.
pub fn changed_files(repo_root: &Utf8Path, target: &DiffTarget) -> Result<Vec<ChangedFile>> {
    let recursive_changes = collect_recursive_submodule_changes(repo_root, target)?;
    let known_submodule_paths = known_submodule_paths(repo_root, target)?;
    let recursive_prefixes: HashSet<&str> = known_submodule_paths
        .iter()
        .map(String::as_str)
        .filter(|submodule_path| {
            recursive_changes.iter().any(|change| {
                change.path == *submodule_path
                    || change
                        .path
                        .strip_prefix(submodule_path)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
        })
        .collect();
    let known_submodule_set: HashSet<&str> =
        known_submodule_paths.iter().map(String::as_str).collect();

    let args = target.git_args();
    let output = git_cmd()
        .args(&args)
        .current_dir(repo_root)
        .output()
        .context("failed to run git diff")?;

    anyhow::ensure!(
        output.status.success(),
        "git diff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout =
        std::str::from_utf8(&output.stdout).context("git diff output is not valid UTF-8")?;

    let root_changes = parse_name_status_z(stdout)?;
    let mut results = Vec::new();
    let mut expanded = Vec::new();

    for change in root_changes {
        if !is_submodule_marker(&change, &known_submodule_set) {
            results.push(change);
            continue;
        }

        let submodule_expanded = expand_submodule_marker(repo_root, target, &change)?;
        if !submodule_expanded.is_empty() {
            expanded.extend(submodule_expanded);
            continue;
        }

        if submodule_path_for_change(&change, &known_submodule_set)
            .is_some_and(|path| recursive_prefixes.contains(path))
        {
            continue;
        }

        results.push(change);
    }

    results.extend(recursive_changes);
    results.extend(expanded);
    Ok(dedup_changes(results))
}

fn collect_recursive_submodule_changes(
    repo_root: &Utf8Path,
    target: &DiffTarget,
) -> Result<Vec<ChangedFile>> {
    let mut results = Vec::new();
    let recursive_target = match target {
        DiffTarget::BaseRef(_) => DiffTarget::BaseRef("HEAD".to_owned()),
        DiffTarget::Staged => DiffTarget::Staged,
        DiffTarget::WorkingTree => DiffTarget::WorkingTree,
    };

    for submodule_path in known_submodule_paths(repo_root, target)? {
        let submodule_root = repo_root.join(&submodule_path);
        if !submodule_root.is_dir() || !submodule_root.join(".git").exists() {
            continue;
        }

        match changed_files(&submodule_root, &recursive_target) {
            Ok(changes) => results.extend(changes.into_iter().map(|change| {
                let path = prefix_rel_path(&submodule_path, &change.path);
                let old_path = change
                    .old_path
                    .as_deref()
                    .map(|old| prefix_rel_path(&submodule_path, old));
                ChangedFile {
                    path,
                    change_type: change.change_type,
                    old_path,
                }
            })),
            Err(error) => {
                tracing::warn!(
                    "skipping recursive submodule diff '{}': {}",
                    submodule_path,
                    error
                );
            }
        }
    }

    Ok(results)
}

fn known_submodule_paths(repo_root: &Utf8Path, target: &DiffTarget) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    for path in submodule_paths_from_worktree(repo_root)? {
        if seen.insert(path.clone()) {
            paths.push(path);
        }
    }

    match target {
        DiffTarget::BaseRef(spec) => {
            let (old_treeish, new_treeish) = diff_treeish_pair(repo_root, spec)?;
            for path in submodule_paths_at_treeish(repo_root, &old_treeish)? {
                if seen.insert(path.clone()) {
                    paths.push(path);
                }
            }
            for path in submodule_paths_at_treeish(repo_root, &new_treeish)? {
                if seen.insert(path.clone()) {
                    paths.push(path);
                }
            }
        }
        DiffTarget::Staged | DiffTarget::WorkingTree => {
            for path in submodule_paths_at_treeish(repo_root, "HEAD")? {
                if seen.insert(path.clone()) {
                    paths.push(path);
                }
            }
        }
    }

    Ok(paths)
}

fn submodule_paths_from_worktree(repo_root: &Utf8Path) -> Result<Vec<String>> {
    submodule_paths_from_config(repo_root, ConfigSource::Worktree)
}

fn submodule_paths_at_treeish(repo_root: &Utf8Path, treeish: &str) -> Result<Vec<String>> {
    submodule_paths_from_config(repo_root, ConfigSource::Treeish(treeish.to_owned()))
}

enum ConfigSource {
    Worktree,
    Treeish(String),
}

fn submodule_paths_from_config(repo_root: &Utf8Path, source: ConfigSource) -> Result<Vec<String>> {
    let mut command = git_cmd();
    command.arg("config").arg("-z");

    match source {
        ConfigSource::Worktree => {
            let gitmodules_path = repo_root.join(".gitmodules");
            if !gitmodules_path.is_file() {
                return Ok(Vec::new());
            }
            command.args(["--file", ".gitmodules"]);
        }
        ConfigSource::Treeish(treeish) => {
            command.args(["--blob", &format!("{treeish}:.gitmodules")]);
        }
    }

    command
        .args(["--get-regexp", r"^submodule\..*\.path$"])
        .current_dir(repo_root);
    let output = command.output().context("failed to read submodule paths")?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout =
        std::str::from_utf8(&output.stdout).context("submodule path output is not valid UTF-8")?;

    let mut submodules = Vec::new();
    for entry in stdout.split('\0').filter(|entry| !entry.is_empty()) {
        let Some((_, path)) = entry.split_once('\n') else {
            anyhow::bail!("malformed submodule path entry: {entry}");
        };
        submodules.push(to_forward_slashes(path));
    }

    Ok(submodules)
}

fn diff_treeish_pair(repo_root: &Utf8Path, spec: &str) -> Result<(String, String)> {
    if let Some((left, right)) = spec.split_once("...") {
        let output = git_cmd()
            .args(["merge-base", left, right])
            .current_dir(repo_root)
            .output()
            .context("failed to resolve merge-base for diff spec")?;
        anyhow::ensure!(
            output.status.success(),
            "git merge-base failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let base = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Ok((base, right.to_owned()));
    }

    if let Some((left, right)) = spec.split_once("..") {
        return Ok((left.to_owned(), right.to_owned()));
    }

    Ok((spec.to_owned(), "HEAD".to_owned()))
}

fn is_submodule_marker(change: &ChangedFile, known_submodule_paths: &HashSet<&str>) -> bool {
    submodule_path_for_change(change, known_submodule_paths).is_some()
}

fn submodule_path_for_change<'a>(
    change: &'a ChangedFile,
    known_submodule_paths: &HashSet<&'a str>,
) -> Option<&'a str> {
    if known_submodule_paths.contains(change.path.as_str()) {
        return Some(change.path.as_str());
    }
    change
        .old_path
        .as_deref()
        .filter(|old| known_submodule_paths.contains(old))
}

fn expand_submodule_marker(
    repo_root: &Utf8Path,
    target: &DiffTarget,
    change: &ChangedFile,
) -> Result<Vec<ChangedFile>> {
    match change.change_type {
        ChangeType::Renamed | ChangeType::Copied => {
            let Some(old_prefix) = change.old_path.as_deref() else {
                return Ok(Vec::new());
            };
            let old_sha = gitlink_old_sha(repo_root, target, old_prefix)?;
            let new_sha = gitlink_new_sha(repo_root, target, &change.path)?;
            expand_submodule_path_change(
                repo_root,
                change.change_type,
                old_prefix,
                &change.path,
                old_sha.as_deref(),
                new_sha.as_deref(),
            )
        }
        _ => {
            let old_sha = gitlink_old_sha(repo_root, target, &change.path)?;
            let new_sha = gitlink_new_sha(repo_root, target, &change.path)?;
            expand_gitlink_delta(
                repo_root,
                &change.path,
                old_sha.as_deref(),
                new_sha.as_deref(),
            )
        }
    }
}

fn gitlink_old_sha(
    repo_root: &Utf8Path,
    target: &DiffTarget,
    submodule_path: &str,
) -> Result<Option<String>> {
    match target {
        DiffTarget::BaseRef(spec) => {
            let (old_treeish, _) = diff_treeish_pair(repo_root, spec)?;
            gitlink_sha_at_treeish(repo_root, &old_treeish, submodule_path)
        }
        DiffTarget::Staged => gitlink_sha_at_treeish(repo_root, "HEAD", submodule_path),
        DiffTarget::WorkingTree => gitlink_sha_in_index(repo_root, submodule_path),
    }
}

fn gitlink_new_sha(
    repo_root: &Utf8Path,
    target: &DiffTarget,
    submodule_path: &str,
) -> Result<Option<String>> {
    match target {
        DiffTarget::BaseRef(spec) => {
            let (_, new_treeish) = diff_treeish_pair(repo_root, spec)?;
            gitlink_sha_at_treeish(repo_root, &new_treeish, submodule_path)
        }
        DiffTarget::Staged => gitlink_sha_in_index(repo_root, submodule_path),
        DiffTarget::WorkingTree => gitlink_sha_in_worktree(repo_root, submodule_path),
    }
}

fn gitlink_sha_at_treeish(
    repo_root: &Utf8Path,
    treeish: &str,
    submodule_path: &str,
) -> Result<Option<String>> {
    let output = git_cmd()
        .arg("rev-parse")
        .arg(format!("{treeish}:{submodule_path}"))
        .current_dir(repo_root)
        .output()
        .context("failed to resolve submodule gitlink from treeish")?;

    if !output.status.success() {
        return Ok(None);
    }

    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if sha.is_empty() {
        Ok(None)
    } else {
        Ok(Some(sha))
    }
}

fn gitlink_sha_in_index(repo_root: &Utf8Path, submodule_path: &str) -> Result<Option<String>> {
    let output = git_cmd()
        .arg("rev-parse")
        .arg(format!(":{submodule_path}"))
        .current_dir(repo_root)
        .output()
        .context("failed to resolve staged submodule gitlink")?;

    if !output.status.success() {
        return Ok(None);
    }

    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if sha.is_empty() {
        Ok(None)
    } else {
        Ok(Some(sha))
    }
}

fn gitlink_sha_in_worktree(repo_root: &Utf8Path, submodule_path: &str) -> Result<Option<String>> {
    let submodule_root = repo_root.join(submodule_path);
    if !submodule_root.is_dir() || !submodule_root.join(".git").exists() {
        return Ok(None);
    }

    let output = git_cmd()
        .args(["rev-parse", "HEAD"])
        .current_dir(&submodule_root)
        .output()
        .context("failed to resolve submodule HEAD")?;

    if !output.status.success() {
        return Ok(None);
    }

    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if sha.is_empty() {
        Ok(None)
    } else {
        Ok(Some(sha))
    }
}

fn expand_submodule_path_change(
    repo_root: &Utf8Path,
    change_type: ChangeType,
    old_prefix: &str,
    new_prefix: &str,
    old_sha: Option<&str>,
    new_sha: Option<&str>,
) -> Result<Vec<ChangedFile>> {
    match (old_sha, new_sha) {
        (Some(old_sha), Some(new_sha)) if old_sha == new_sha => {
            list_tree_paths(repo_root, old_prefix, old_sha)?
                .into_iter()
                .map(|rel_path| {
                    Ok(ChangedFile {
                        path: prefix_rel_path(new_prefix, &rel_path),
                        old_path: Some(prefix_rel_path(old_prefix, &rel_path)),
                        change_type,
                    })
                })
                .collect()
        }
        (Some(old_sha), Some(new_sha)) => {
            let mut expanded =
                list_tree_changes(repo_root, old_prefix, old_sha, ChangeType::Deleted)?;
            expanded.extend(list_tree_changes(
                repo_root,
                new_prefix,
                new_sha,
                ChangeType::Added,
            )?);
            Ok(expanded)
        }
        (Some(old_sha), None) => {
            list_tree_changes(repo_root, old_prefix, old_sha, ChangeType::Deleted)
        }
        (None, Some(new_sha)) => {
            list_tree_changes(repo_root, new_prefix, new_sha, ChangeType::Added)
        }
        (None, None) => Ok(Vec::new()),
    }
}

fn expand_gitlink_delta(
    repo_root: &Utf8Path,
    prefix: &str,
    old_sha: Option<&str>,
    new_sha: Option<&str>,
) -> Result<Vec<ChangedFile>> {
    match (old_sha, new_sha) {
        (Some(old_sha), Some(new_sha)) if old_sha != new_sha => {
            diff_submodule_commits(repo_root, prefix, old_sha, new_sha)
        }
        (None, Some(new_sha)) => list_tree_changes(repo_root, prefix, new_sha, ChangeType::Added),
        (Some(old_sha), None) => list_tree_changes(repo_root, prefix, old_sha, ChangeType::Deleted),
        _ => Ok(Vec::new()),
    }
}

fn diff_submodule_commits(
    repo_root: &Utf8Path,
    prefix: &str,
    old_sha: &str,
    new_sha: &str,
) -> Result<Vec<ChangedFile>> {
    let output = submodule_git_output(
        repo_root,
        prefix,
        &["diff", "--name-status", "-z", old_sha, new_sha],
    )
    .context("failed to diff submodule commits")?;
    let Some(output) = output else {
        return Ok(Vec::new());
    };

    anyhow::ensure!(
        output.status.success(),
        "git diff for submodule failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout =
        std::str::from_utf8(&output.stdout).context("submodule diff output is not valid UTF-8")?;
    let changes = parse_name_status_z(stdout)?;
    Ok(changes
        .into_iter()
        .map(|change| ChangedFile {
            path: prefix_rel_path(prefix, &change.path),
            old_path: change
                .old_path
                .as_deref()
                .map(|old| prefix_rel_path(prefix, old)),
            change_type: change.change_type,
        })
        .collect())
}

fn list_tree_changes(
    repo_root: &Utf8Path,
    prefix: &str,
    treeish: &str,
    change_type: ChangeType,
) -> Result<Vec<ChangedFile>> {
    Ok(list_tree_paths(repo_root, prefix, treeish)?
        .into_iter()
        .map(|entry| ChangedFile {
            path: prefix_rel_path(prefix, &entry),
            old_path: None,
            change_type,
        })
        .collect())
}

fn list_tree_paths(repo_root: &Utf8Path, prefix: &str, treeish: &str) -> Result<Vec<String>> {
    let output = submodule_git_output(
        repo_root,
        prefix,
        &["ls-tree", "-r", "--name-only", "-z", treeish],
    )
    .context("failed to list submodule tree")?;
    let Some(output) = output else {
        return Ok(Vec::new());
    };

    anyhow::ensure!(
        output.status.success(),
        "git ls-tree for submodule failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout =
        std::str::from_utf8(&output.stdout).context("submodule tree output is not valid UTF-8")?;
    Ok(stdout
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(to_forward_slashes)
        .collect())
}

fn submodule_git_output(
    repo_root: &Utf8Path,
    prefix: &str,
    args: &[&str],
) -> Result<Option<std::process::Output>> {
    let submodule_root = repo_root.join(prefix);
    if submodule_root.is_dir() && submodule_root.join(".git").exists() {
        return git_cmd()
            .args(args)
            .current_dir(&submodule_root)
            .output()
            .context("failed to run git in submodule worktree")
            .map(Some);
    }

    let Some(git_dir) = submodule_git_dir(repo_root, prefix)? else {
        return Ok(None);
    };

    git_cmd()
        .arg("-c")
        .arg("core.worktree=.")
        .arg(format!("--git-dir={}", git_dir))
        .arg("--work-tree=.")
        .args(args)
        .current_dir(repo_root)
        .output()
        .context("failed to run git against submodule git dir")
        .map(Some)
}

fn submodule_git_dir(repo_root: &Utf8Path, prefix: &str) -> Result<Option<String>> {
    let output = git_cmd()
        .args(["rev-parse", "--git-path", &format!("modules/{prefix}")])
        .current_dir(repo_root)
        .output()
        .context("failed to resolve submodule git dir")?;

    if !output.status.success() {
        return Ok(None);
    }

    let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if git_dir.is_empty() {
        return Ok(None);
    }

    let git_dir_path = Utf8PathBuf::from(to_forward_slashes(&git_dir));
    let resolved_git_dir = if git_dir_path.is_absolute() {
        git_dir_path
    } else {
        repo_root.join(git_dir_path)
    };
    if resolved_git_dir.exists() {
        Ok(Some(resolved_git_dir.to_string()))
    } else {
        Ok(None)
    }
}

fn prefix_rel_path(prefix: &str, rel_path: &str) -> String {
    Utf8PathBuf::from(prefix).join(rel_path).to_string()
}

fn dedup_changes(changes: Vec<ChangedFile>) -> Vec<ChangedFile> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for change in changes {
        let key = format!(
            "{:?}\0{}\0{}",
            change.change_type,
            change.path,
            change.old_path.as_deref().unwrap_or("")
        );
        if seen.insert(key) {
            deduped.push(change);
        }
    }

    deduped
}

/// Parse the NUL-separated `--name-status -z` output from git.
///
/// Format per record:
/// - Non-rename: `<status>\0<path>\0`
/// - Rename/copy: `<status><score>\0<old-path>\0<new-path>\0`
fn parse_name_status_z(raw: &str) -> Result<Vec<ChangedFile>> {
    let parts: Vec<&str> = raw.split('\0').filter(|s| !s.is_empty()).collect();
    let mut results = Vec::new();
    let mut i = 0;

    while i < parts.len() {
        let status_field = parts[i];
        i += 1;

        let status_char = status_field.chars().next().unwrap_or(' ');

        match status_char {
            'A' => {
                let path = canonical_git_diff_path(consume(&mut i, &parts)?)?;
                results.push(ChangedFile {
                    path,
                    change_type: ChangeType::Added,
                    old_path: None,
                });
            }
            'M' => {
                let path = canonical_git_diff_path(consume(&mut i, &parts)?)?;
                results.push(ChangedFile {
                    path,
                    change_type: ChangeType::Modified,
                    old_path: None,
                });
            }
            'D' => {
                let path = canonical_git_diff_path(consume(&mut i, &parts)?)?;
                results.push(ChangedFile {
                    path,
                    change_type: ChangeType::Deleted,
                    old_path: None,
                });
            }
            'R' => {
                let old_path = canonical_git_diff_path(consume(&mut i, &parts)?)?;
                let new_path = canonical_git_diff_path(consume(&mut i, &parts)?)?;
                results.push(ChangedFile {
                    path: new_path,
                    change_type: ChangeType::Renamed,
                    old_path: Some(old_path),
                });
            }
            'C' => {
                let old_path = canonical_git_diff_path(consume(&mut i, &parts)?)?;
                let new_path = canonical_git_diff_path(consume(&mut i, &parts)?)?;
                results.push(ChangedFile {
                    path: new_path,
                    change_type: ChangeType::Copied,
                    old_path: Some(old_path),
                });
            }
            other => {
                tracing::warn!("unknown git diff status '{other}', skipping");
                // Consume one path token to stay in sync.
                if i < parts.len() {
                    i += 1;
                }
            }
        }
    }

    Ok(results)
}

fn canonical_git_diff_path(path: &str) -> Result<String> {
    Ok(CanonicalRepoPath::from_git_diff_path(path)?
        .as_str()
        .to_owned())
}

fn consume<'a>(i: &mut usize, parts: &[&'a str]) -> Result<&'a str> {
    anyhow::ensure!(*i < parts.len(), "unexpected end of git diff output");
    let v = parts[*i];
    *i += 1;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Vec<ChangedFile> {
        parse_name_status_z(raw).unwrap()
    }

    #[test]
    fn added_file() {
        // status\0path\0
        let raw = "A\0src/new.rs\0";
        let files = parse(raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/new.rs");
        assert_eq!(files[0].change_type, ChangeType::Added);
        assert!(files[0].old_path.is_none());
    }

    #[test]
    fn modified_and_deleted() {
        let raw = "M\0src/lib.rs\0D\0src/old.rs\0";
        let files = parse(raw);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].change_type, ChangeType::Modified);
        assert_eq!(files[1].change_type, ChangeType::Deleted);
        assert_eq!(files[1].path, "src/old.rs");
    }

    #[test]
    fn renamed_file() {
        // R<score>\0old\0new\0
        let raw = "R100\0src/old.rs\0src/new.rs\0";
        let files = parse(raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].change_type, ChangeType::Renamed);
        assert_eq!(files[0].path, "src/new.rs");
        assert_eq!(files[0].old_path.as_deref(), Some("src/old.rs"));
    }

    #[test]
    fn copied_file() {
        let raw = "C80\0src/base.rs\0src/copy.rs\0";
        let files = parse(raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].change_type, ChangeType::Copied);
        assert_eq!(files[0].old_path.as_deref(), Some("src/base.rs"));
    }

    #[test]
    fn windows_style_paths_are_normalized_when_parsing_git_output() {
        let raw = "M\0src\\lib.rs\0R100\0old\\name.rs\0new\\name.rs\0";
        let files = parse(raw);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].change_type, ChangeType::Modified);
        assert_eq!(files[0].path, "src/lib.rs");
        assert_eq!(files[1].change_type, ChangeType::Renamed);
        assert_eq!(files[1].path, "new/name.rs");
        assert_eq!(files[1].old_path.as_deref(), Some("old/name.rs"));
    }

    #[test]
    fn empty_output() {
        assert!(parse("").is_empty());
    }
}
