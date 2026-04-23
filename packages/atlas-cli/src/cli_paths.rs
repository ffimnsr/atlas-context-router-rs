use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;

pub(crate) fn canonicalize_cli_path(raw: &str) -> Result<String> {
    let absolute = resolve_absolute_path(raw)?;
    let normalized = Utf8PathBuf::from_path_buf(absolute)
        .map_err(|path| anyhow::anyhow!("path '{}' is not valid UTF-8", path.display()))?;
    let normalized =
        atlas_repo::canonical_absolute_path(normalized.as_path()).map_err(anyhow::Error::from)?;
    let physical = canonicalize_existing_ancestor(normalized.as_std_path())?;
    let physical = Utf8PathBuf::from_path_buf(physical)
        .map_err(|path| anyhow::anyhow!("path '{}' is not valid UTF-8", path.display()))?;
    let canonical =
        atlas_repo::canonical_absolute_path(physical.as_path()).map_err(anyhow::Error::from)?;
    Ok(canonical.into_string())
}

fn resolve_absolute_path(raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("cannot determine cwd")?
            .join(path))
    }
}

fn canonicalize_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut existing = path;
    let mut suffix = Vec::<OsString>::new();

    while !existing.exists() {
        let name = existing.file_name().ok_or_else(|| {
            anyhow::anyhow!(
                "cannot canonicalize '{}' because no existing ancestor was found",
                path.display()
            )
        })?;
        suffix.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "cannot canonicalize '{}' because no existing ancestor was found",
                path.display()
            )
        })?;
    }

    let mut canonical = std::fs::canonicalize(existing)
        .with_context(|| format!("cannot canonicalize {}", existing.display()))?;
    for component in suffix.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}
