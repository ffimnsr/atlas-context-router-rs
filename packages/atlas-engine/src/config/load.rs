//! Config loading from `.atlas/config.toml` and default/template writes.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::Config;
use super::template::ConfigTemplateProfile;

impl Config {
    /// Load config from `<atlas_dir>/config.toml`.
    ///
    /// Returns a default `Config` if the file does not exist.
    pub fn load(atlas_dir: &Path) -> Result<Self> {
        let path = atlas_dir.join(crate::paths::ATLAS_CONFIG);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let config: Self =
            toml::from_str(&raw).with_context(|| format!("cannot parse {}", path.display()))?;
        config.insights.validate()?;
        config.insights.validate_layer_rules_file(atlas_dir)?;
        config.sanitization.validate(atlas_dir)?;
        Ok(config)
    }

    /// Write the default config to `<atlas_dir>/config.toml`.
    ///
    /// Does not overwrite an existing file.
    pub fn write_default(atlas_dir: &Path) -> Result<bool> {
        Self::write_template(atlas_dir, ConfigTemplateProfile::Standard)
    }

    /// Write a commented config template to `<atlas_dir>/config.toml`.
    ///
    /// Does not overwrite an existing file.
    pub fn write_template(atlas_dir: &Path, profile: ConfigTemplateProfile) -> Result<bool> {
        let path = atlas_dir.join(crate::paths::ATLAS_CONFIG);
        if path.exists() {
            return Ok(false);
        }
        let content = Self::render_template(profile)?;
        fs::write(&path, content).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(true)
    }
}
