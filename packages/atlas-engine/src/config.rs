use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default parse-worker batch size.  Can be overridden in `.atlas/config.toml`.
pub const DEFAULT_PARSE_BATCH_SIZE: usize = 64;

/// Top-level atlas configuration loaded from `.atlas/config.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub search: SearchConfig,
}

/// Search-phase configuration.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Enable hybrid (FTS + vector) retrieval when an embedding backend is configured.
    /// Falls back to FTS-only when no backend is available regardless of this flag.
    pub hybrid_enabled: bool,
    /// FTS candidate pool size fetched before Reciprocal Rank Fusion merge.
    pub top_k_fts: usize,
    /// Vector candidate pool size fetched before Reciprocal Rank Fusion merge.
    pub top_k_vector: usize,
    /// RRF k constant (higher = less rank-position sensitivity, default 60).
    pub rrf_k: u32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            hybrid_enabled: false,
            top_k_fts: 60,
            top_k_vector: 60,
            rrf_k: 60,
        }
    }
}

/// Build-phase configuration.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Number of files parsed in parallel per batch (clamped to 1–4096).
    pub parse_batch_size: usize,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            parse_batch_size: DEFAULT_PARSE_BATCH_SIZE,
        }
    }
}

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
        toml::from_str(&raw).with_context(|| format!("cannot parse {}", path.display()))
    }

    /// Write the default config to `<atlas_dir>/config.toml`.
    ///
    /// Does not overwrite an existing file.
    pub fn write_default(atlas_dir: &Path) -> Result<bool> {
        let path = atlas_dir.join(crate::paths::ATLAS_CONFIG);
        if path.exists() {
            return Ok(false);
        }
        let default = Self::default();
        let content =
            toml::to_string_pretty(&default).context("cannot serialize default config")?;
        fs::write(&path, content).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(true)
    }

    /// Return the effective parse batch size, clamped to [1, 4096].
    pub fn parse_batch_size(&self) -> usize {
        self.build.parse_batch_size.clamp(1, 4096)
    }
}
