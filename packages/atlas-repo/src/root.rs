use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use std::process::Command;

use crate::path::to_forward_slashes;

/// Locate the git repository root by first running `git rev-parse --show-toplevel`
/// and falling back to walking parent directories for a `.git` entry.
pub fn find_repo_root(start: &Utf8Path) -> Result<Utf8PathBuf> {
    if let Ok(root) = git_toplevel(start) {
        return Ok(root);
    }
    walk_for_git(start).context("no git repository found from this directory")
}

fn git_toplevel(cwd: &Utf8Path) -> Result<Utf8PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .context("failed to run git")?;

    anyhow::ensure!(output.status.success(), "git exited non-zero");

    let raw = std::str::from_utf8(&output.stdout)
        .context("git output is not valid UTF-8")?
        .trim();

    // On Windows git may return backslash-separated or mixed paths — normalise.
    let normalised = to_forward_slashes(raw);
    Ok(Utf8PathBuf::from(&normalised))
}

fn walk_for_git(start: &Utf8Path) -> Result<Utf8PathBuf> {
    let abs = start
        .canonicalize_utf8()
        .with_context(|| format!("cannot canonicalize '{start}'"))?;

    let mut current = abs.as_path();
    loop {
        if current.join(".git").exists() {
            return Ok(current.to_owned());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => anyhow::bail!("reached filesystem root without finding .git"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8Path;

    #[test]
    fn finds_workspace_root() {
        // The workspace itself is a git repo; start from a nested dir.
        let here = Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = find_repo_root(here).expect("should find repo root");
        assert!(root.join(".git").exists(), ".git must exist at repo root");
    }
}
