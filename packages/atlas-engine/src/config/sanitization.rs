//! Sanitization configuration (redaction rules file resolution).

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::validate_nonempty_string;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SanitizationConfig {
    pub redaction_rules_file: Option<String>,
}

impl SanitizationConfig {
    pub fn resolve_redaction_rules_file(
        &self,
        atlas_dir: &Path,
    ) -> Result<Option<std::path::PathBuf>> {
        let Some(raw_path) = self.redaction_rules_file.as_deref() else {
            return Ok(None);
        };
        let trimmed = validate_nonempty_string("sanitization.redaction_rules_file", raw_path)?;
        let candidate = Path::new(&trimmed);
        let resolved = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            atlas_dir.join(candidate)
        };
        Ok(Some(resolved))
    }

    pub fn validate(&self, atlas_dir: &Path) -> Result<()> {
        let Some(path) = self.resolve_redaction_rules_file(atlas_dir)? else {
            return Ok(());
        };
        if !path.exists() {
            anyhow::bail!(
                "invalid config: sanitization.redaction_rules_file points to missing file {}",
                path.display()
            );
        }
        if !path.is_file() {
            anyhow::bail!(
                "invalid config: sanitization.redaction_rules_file must point to a readable file, got {}",
                path.display()
            );
        }
        atlas_adapters::load_redaction_rules_file(&path).with_context(|| {
            format!(
                "invalid config: sanitization.redaction_rules_file={} failed validation",
                path.display()
            )
        })?;
        Ok(())
    }
}
