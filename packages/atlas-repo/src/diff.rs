use anyhow::{Context, Result};
use atlas_core::model::{ChangeType, ChangedFile};
use camino::Utf8Path;
use std::process::Command;

use crate::path::to_forward_slashes;

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
    let args = target.git_args();
    let output = Command::new("git")
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

    parse_name_status_z(stdout)
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
                let path = to_forward_slashes(consume(&mut i, &parts)?);
                results.push(ChangedFile {
                    path,
                    change_type: ChangeType::Added,
                    old_path: None,
                });
            }
            'M' => {
                let path = to_forward_slashes(consume(&mut i, &parts)?);
                results.push(ChangedFile {
                    path,
                    change_type: ChangeType::Modified,
                    old_path: None,
                });
            }
            'D' => {
                let path = to_forward_slashes(consume(&mut i, &parts)?);
                results.push(ChangedFile {
                    path,
                    change_type: ChangeType::Deleted,
                    old_path: None,
                });
            }
            'R' => {
                let old_path = to_forward_slashes(consume(&mut i, &parts)?);
                let new_path = to_forward_slashes(consume(&mut i, &parts)?);
                results.push(ChangedFile {
                    path: new_path,
                    change_type: ChangeType::Renamed,
                    old_path: Some(old_path),
                });
            }
            'C' => {
                let old_path = to_forward_slashes(consume(&mut i, &parts)?);
                let new_path = to_forward_slashes(consume(&mut i, &parts)?);
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
    fn empty_output() {
        assert!(parse("").is_empty());
    }
}
