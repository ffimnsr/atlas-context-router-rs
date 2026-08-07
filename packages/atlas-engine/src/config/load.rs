//! Config loading from `.atlas/config.toml`, default/template writes, and
//! runtime token counter building.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use camino::Utf8Path;

use super::Config;
use super::context::{TokenizerFallbackMode, TokenizerProvider};
use super::template::ConfigTemplateProfile;

/// Result of building a runtime token counter from config.
#[derive(Debug)]
pub struct TokenCounterLoadResult {
    /// Ready-to-use counter honoring `context.tokenizer`.
    pub counter: atlas_token_count::TokenCounter,
    /// True when the configured tokenizer failed to load and the byte
    /// heuristic was used instead.
    pub fallback_used: bool,
    /// Load-error summary when `fallback_used`; no payload content.
    pub fallback_reason: Option<String>,
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
        let config: Self =
            toml::from_str(&raw).with_context(|| format!("cannot parse {}", path.display()))?;
        config.insights.validate()?;
        config.insights.validate_layer_rules_file(atlas_dir)?;
        config.sanitization.validate(atlas_dir)?;
        config.context.tokenizer.validate()?;
        Ok(config)
    }

    /// Build a runtime token counter from `context.tokenizer`.
    ///
    /// Relative `tokenizer_file` values resolve from `atlas_dir` (`.atlas/`),
    /// matching external config file resolution. When the configured
    /// tokenizer cannot be loaded, `fallback` decides between a heuristic
    /// counter with fallback metadata and a hard error.
    pub fn token_counter(&self, atlas_dir: &Utf8Path) -> Result<TokenCounterLoadResult> {
        self.context.tokenizer.validate()?;
        let tokenizer = &self.context.tokenizer;
        match tokenizer.provider {
            TokenizerProvider::Heuristic => Ok(TokenCounterLoadResult {
                counter: atlas_token_count::TokenCounter::heuristic(tokenizer.bytes_per_token)?,
                fallback_used: false,
                fallback_reason: None,
            }),
            TokenizerProvider::Tokenizers => {
                let Some(file) = tokenizer.tokenizer_file.as_deref() else {
                    anyhow::bail!(
                        "invalid config: context.tokenizer.tokenizer_file is required when context.tokenizer.provider = \"tokenizers\""
                    );
                };
                let path = resolve_tokenizer_file(atlas_dir, file);
                match atlas_token_count::TokenCounter::from_file(
                    &path,
                    "tokenizers",
                    tokenizer.model.clone(),
                ) {
                    Ok(counter) => Ok(TokenCounterLoadResult {
                        counter,
                        fallback_used: false,
                        fallback_reason: None,
                    }),
                    Err(err) => match tokenizer.fallback {
                        TokenizerFallbackMode::Heuristic => Ok(TokenCounterLoadResult {
                            counter: atlas_token_count::TokenCounter::heuristic(
                                tokenizer.bytes_per_token,
                            )?,
                            fallback_used: true,
                            fallback_reason: Some(err.to_string()),
                        }),
                        TokenizerFallbackMode::FailClosed => Err(err).with_context(|| {
                            format!(
                                "invalid config: context.tokenizer.tokenizer_file={file} failed to load from {}",
                                path.display()
                            )
                        }),
                    },
                }
            }
        }
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

/// Resolve a configured tokenizer file path: relative paths resolve from
/// `atlas_dir` (`.atlas/`), absolute paths are kept as-is.
fn resolve_tokenizer_file(atlas_dir: &Utf8Path, file: &str) -> PathBuf {
    let candidate = Path::new(file);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        atlas_dir.as_std_path().join(candidate)
    }
}
