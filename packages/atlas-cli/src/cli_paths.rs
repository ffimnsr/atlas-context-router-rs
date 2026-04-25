use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use camino::Utf8PathBuf;

pub(crate) fn canonicalize_cli_path(raw: &str) -> Result<String> {
    let absolute = resolve_absolute_path(raw)?;
    let normalized = Utf8PathBuf::from_path_buf(absolute)
        .map_err(|path| anyhow::anyhow!("path '{}' is not valid UTF-8", path.display()))?;
    let canonical =
        atlas_repo::canonical_filesystem_path(normalized.as_path()).map_err(anyhow::Error::from)?;
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
