use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub(super) const LEGACY_HOOK_MARKER: &str = "atlas update # atlas-hook";
pub(super) const HOOK_START_MARKER: &str = "# atlas-hook start";
pub(super) const HOOK_END_MARKER: &str = "# atlas-hook end";
pub(super) const HOOK_VERSION_MARKER: &str = "# atlas-hook version: 1";

const PRE_COMMIT_HOOK_SCRIPT: &str = r#"
# Installed by atlas. Remove these lines to disable atlas graph updates.
if command -v atlas >/dev/null 2>&1; then
    atlas update || true
    atlas detect-changes || true
fi
echo "[pre-commit] complete"
"#;

const QUIET_HOOK_SCRIPT: &str = r#"
# Installed by atlas. Remove these lines to disable atlas graph updates.
if command -v atlas >/dev/null 2>&1; then
    atlas update || true
    atlas detect-changes || true
fi
"#;

pub fn install_git_hooks(repo_root: &Path, dry_run: bool, force: bool) -> Result<Vec<PathBuf>> {
    let git_dir = repo_root.join(".git");
    if !git_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for hook_name in ["pre-commit", "post-checkout", "post-merge", "post-rewrite"] {
        paths.push(install_git_hook(
            git_dir.join("hooks").join(hook_name),
            dry_run,
            force,
        )?);
    }

    Ok(paths)
}

fn install_git_hook(hook_path: PathBuf, dry_run: bool, force: bool) -> Result<PathBuf> {
    let hook_name = hook_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("git hook");
    let hook_script = if hook_name == "pre-commit" {
        PRE_COMMIT_HOOK_SCRIPT
    } else {
        QUIET_HOOK_SCRIPT
    };

    let existing = if hook_path.exists() {
        fs::read_to_string(&hook_path)
            .with_context(|| format!("cannot read {}", hook_path.display()))?
    } else {
        String::new()
    };

    if !force && has_unmanaged_hook(&existing) {
        bail!(
            "refusing to overwrite non-Atlas hook at {}. Re-run `atlas install --force` to replace it",
            hook_path.display()
        );
    }

    let next_content = upsert_hook_block(&existing, hook_script, force);
    if next_content == existing {
        return Ok(hook_path);
    }

    if dry_run {
        println!(
            "  [dry-run] {hook_name}: would write {}",
            hook_path.display()
        );
        return Ok(hook_path);
    }

    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    fs::write(&hook_path, &next_content)
        .with_context(|| format!("cannot write {}", hook_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("cannot chmod {}", hook_path.display()))?;
    }

    Ok(hook_path)
}

fn upsert_hook_block(existing: &str, hook_script: &str, force: bool) -> String {
    let managed_block =
        format!("{HOOK_START_MARKER}\n{HOOK_VERSION_MARKER}\n{hook_script}{HOOK_END_MARKER}\n");

    if let Some((start, end)) = managed_hook_range(existing) {
        let mut updated = String::new();
        updated.push_str(&existing[..start]);
        updated.push_str(&managed_block);
        updated.push_str(&existing[end..]);
        return updated;
    }

    if let Some(start) = legacy_hook_start(existing) {
        let mut updated = String::new();
        updated.push_str(&existing[..start]);
        updated.push_str(&managed_block);
        return updated;
    }

    if existing.trim().is_empty() || force {
        return format!("#!/bin/sh\n{managed_block}");
    }

    let prefix = if existing.ends_with('\n') {
        existing.to_owned()
    } else {
        format!("{existing}\n")
    };
    format!("{prefix}{managed_block}")
}

fn managed_hook_range(existing: &str) -> Option<(usize, usize)> {
    let start = existing.find(HOOK_START_MARKER)?;
    let mut end = existing[start..]
        .find(HOOK_END_MARKER)
        .map(|offset| start + offset + HOOK_END_MARKER.len())?;
    if existing[end..].starts_with("\r\n") {
        end += 2;
    } else if existing[end..].starts_with('\n') {
        end += 1;
    }
    Some((start, end))
}

fn legacy_hook_start(existing: &str) -> Option<usize> {
    existing.find(LEGACY_HOOK_MARKER).map(|offset| {
        existing[..offset]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0)
    })
}

fn has_unmanaged_hook(existing: &str) -> bool {
    !existing.trim().is_empty()
        && managed_hook_range(existing).is_none()
        && legacy_hook_start(existing).is_none()
}
