//! Discovery skill stub written into the user's workspace by `atlas install`.
//!
//! The stub is intentionally thin. It only redirects agents to the guide served
//! by the installed binary, so it does not change between Atlas releases.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// Repo-relative path of the installed discovery skill.
pub const SKILL_REL: &str = ".agents/skills/atlas/SKILL.md";

/// Stub body shipped with this Atlas version.
pub const SKILL_STUB: &str = include_str!("../../skill-data/discovery/SKILL.md");

/// Write the discovery skill when it is missing.
///
/// An existing file is treated as user-owned and left untouched unless `force`
/// is set, so local edits are never silently discarded.
pub fn install_skill(repo_root: &Path, dry_run: bool, force: bool) -> Result<Option<String>> {
    let path = repo_root.join(SKILL_REL);
    if path.exists() && !force {
        return Ok(None);
    }
    if dry_run {
        return Ok(Some(SKILL_REL.to_owned()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(&path, SKILL_STUB).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(Some(SKILL_REL.to_owned()))
}
