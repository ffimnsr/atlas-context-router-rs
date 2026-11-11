use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// Tool-agnostic hook launcher installed at `.atlas/hooks/atlas-hook`.
///
/// Any AI coding tool that supports command hooks can invoke this script as
/// `atlas-hook <frontend> <event>`. Atlas never writes tool-specific hook
/// configuration; the generic runner keeps host wiring entirely in user hands.
const ATLAS_HOOK_RUNNER_SCRIPT: &str = r#"#!/bin/sh
# Atlas hook runner — thin launcher for Rust `atlas hook`.
# Installed by `atlas install` at .atlas/hooks/atlas-hook.
# All failures are non-blocking.

if [ "$#" -ge 2 ]; then
    ATLAS_FRONTEND="$1"
    ATLAS_EVENT="$2"
else
    ATLAS_FRONTEND="hook"
    ATLAS_EVENT="${1:-unknown}"
fi

# Skip all work when atlas binary is not on PATH.
command -v atlas >/dev/null 2>&1 || exit 0

if [ -n "$ATLAS_EVENT" ] && [ "$ATLAS_EVENT" != "unknown" ]; then
    ATLAS_HOOK_FRONTEND="$ATLAS_FRONTEND" \
    ATLAS_HOOK_SCRIPT_PATH="$0" \
    atlas hook "$ATLAS_EVENT" >/dev/null 2>&1 || true
fi
"#;

pub(super) const ATLAS_HOOK_RUNNER_REL: &str = ".atlas/hooks/atlas-hook";
const ATLAS_HOOK_LIB_REL: &str = ".atlas/hooks/lib/";

/// Install the generic Atlas hook runner. Returns the repo-relative path when a
/// file was created or updated, or `None` when it was already current.
pub fn install_hook_runner(repo_root: &Path, dry_run: bool) -> Result<Option<String>> {
    let hooks_dir = repo_root.join(".atlas").join("hooks");
    let hook_lib_dir = hooks_dir.join("lib");
    let runner_path = hooks_dir.join("atlas-hook");

    let existing = if runner_path.exists() {
        fs::read_to_string(&runner_path)
            .with_context(|| format!("cannot read {}", runner_path.display()))?
    } else {
        String::new()
    };

    if !hook_lib_dir.is_dir() {
        if dry_run {
            println!("  [dry-run] would create {ATLAS_HOOK_LIB_REL}");
        } else {
            fs::create_dir_all(&hook_lib_dir)
                .with_context(|| format!("cannot create {}", hook_lib_dir.display()))?;
        }
    }

    if existing == ATLAS_HOOK_RUNNER_SCRIPT {
        return Ok(None);
    }

    if dry_run {
        if existing.is_empty() {
            println!("  [dry-run] would create {ATLAS_HOOK_RUNNER_REL}");
        } else {
            println!("  [dry-run] would update {ATLAS_HOOK_RUNNER_REL}");
        }
        return Ok(Some(ATLAS_HOOK_RUNNER_REL.to_owned()));
    }

    fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("cannot create {}", hooks_dir.display()))?;
    fs::write(&runner_path, ATLAS_HOOK_RUNNER_SCRIPT)
        .with_context(|| format!("cannot write {}", runner_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runner_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("cannot chmod {}", runner_path.display()))?;
    }

    Ok(Some(ATLAS_HOOK_RUNNER_REL.to_owned()))
}
