//! Atlas-managed `.atlas/.gitignore` policy file.
//!
//! Tracking rules for Atlas state live inside the state directory instead of
//! owning root `.gitignore` entries. `atlas init` and `atlas install` write
//! this file idempotently; user edits are preserved unless `--force` restores
//! the managed content, and `atlas install --validate-only` reports a missing
//! or unmanaged file.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// Repo-relative path of the Atlas state-dir git policy.
pub const ATLAS_GITIGNORE_REL: &str = ".atlas/.gitignore";

/// Marker line used by install validation to recognize the managed file.
pub const ATLAS_GITIGNORE_MARKER: &str = "atlas-managed local-state git policy";

/// Managed `.atlas/.gitignore` content.
///
/// Anchored `/*` ignores only direct children of `.atlas/` so the durable
/// markdown continuity directories can be re-included; deeper negations
/// override any root `*.md` rule. `config.toml`, `hooks/`, `sessions/`,
/// `tasks/`, `experience/`, `locks/`, `tmp/`, and `*.db` indexes stay
/// untracked.
pub const ATLAS_GITIGNORE_CONTENT: &str = r#"# atlas-managed local-state git policy v1 — durable markdown records stay committed.
/*
!/.gitignore
!/memories/
!/decisions/
!/feedback/
!/memoir/
!/overview/
!/artifacts/
!/memories/**/*.md
!/decisions/**/*.md
!/feedback/**/*.md
!/memoir/**/*.md
!/overview/**/*.md
!/artifacts/**/*.md
"#;

/// Ensure `.atlas/.gitignore` matches [`ATLAS_GITIGNORE_CONTENT`].
///
/// Returns `Some(ATLAS_GITIGNORE_REL)` when the file was written (or would be
/// written under `dry_run`) and `None` when it already matches or a user-edited
/// file was preserved because `force` is false.
pub fn ensure_atlas_gitignore(
    repo_root: &Path,
    dry_run: bool,
    force: bool,
) -> Result<Option<String>> {
    let path = repo_root.join(ATLAS_GITIGNORE_REL);
    match fs::read_to_string(&path) {
        Ok(existing) if existing == ATLAS_GITIGNORE_CONTENT => Ok(None),
        // Existing user-edited policy is preserved unless --force restores it.
        Ok(_) if !force => Ok(None),
        _ => {
            if dry_run {
                return Ok(Some(ATLAS_GITIGNORE_REL.to_owned()));
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
            }
            fs::write(&path, ATLAS_GITIGNORE_CONTENT)
                .with_context(|| format!("cannot write {}", path.display()))?;
            Ok(Some(ATLAS_GITIGNORE_REL.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn creates_missing_file_with_parent_dir() {
        let repo = TempDir::new().expect("temp dir");
        let written = ensure_atlas_gitignore(repo.path(), false, false).expect("ensure");
        assert_eq!(written.as_deref(), Some(ATLAS_GITIGNORE_REL));
        let content = fs::read_to_string(repo.path().join(ATLAS_GITIGNORE_REL)).expect("read");
        assert_eq!(content, ATLAS_GITIGNORE_CONTENT);
    }

    #[test]
    fn matching_file_is_unchanged() {
        let repo = TempDir::new().expect("temp dir");
        ensure_atlas_gitignore(repo.path(), false, false).expect("create");
        let before = fs::read_to_string(repo.path().join(ATLAS_GITIGNORE_REL)).expect("read");
        let written = ensure_atlas_gitignore(repo.path(), false, false).expect("re-run");
        assert_eq!(written, None);
        let after = fs::read_to_string(repo.path().join(ATLAS_GITIGNORE_REL)).expect("read");
        assert_eq!(before, after);
    }

    #[test]
    fn user_edits_are_preserved_without_force() {
        let repo = TempDir::new().expect("temp dir");
        ensure_atlas_gitignore(repo.path(), false, false).expect("create");
        let path = repo.path().join(ATLAS_GITIGNORE_REL);
        fs::write(&path, "# user policy\n").expect("user edit");
        let written = ensure_atlas_gitignore(repo.path(), false, false).expect("re-run");
        assert_eq!(written, None);
        assert_eq!(fs::read_to_string(&path).expect("read"), "# user policy\n");
    }

    #[test]
    fn force_restores_managed_content() {
        let repo = TempDir::new().expect("temp dir");
        ensure_atlas_gitignore(repo.path(), false, false).expect("create");
        let path = repo.path().join(ATLAS_GITIGNORE_REL);
        fs::write(&path, "# user policy\n").expect("user edit");
        let written = ensure_atlas_gitignore(repo.path(), false, true).expect("force");
        assert_eq!(written.as_deref(), Some(ATLAS_GITIGNORE_REL));
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            ATLAS_GITIGNORE_CONTENT
        );
    }

    #[test]
    fn dry_run_reports_without_writing() {
        let repo = TempDir::new().expect("temp dir");
        let planned = ensure_atlas_gitignore(repo.path(), true, false).expect("dry run");
        assert_eq!(planned.as_deref(), Some(ATLAS_GITIGNORE_REL));
        assert!(!repo.path().join(ATLAS_GITIGNORE_REL).exists());
    }
}
