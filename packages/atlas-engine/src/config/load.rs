//! Config loading from `.atlas/config.toml`, default/template writes, and
//! runtime token counter building.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use atlas_tokenizer::EncodingName;
use camino::Utf8Path;

use super::Config;
use super::context::TokenizerFallbackMode;
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
        let mut config: Self =
            toml::from_str(&raw).with_context(|| format!("cannot parse {}", path.display()))?;
        config.parsers.resolve_from(atlas_dir);
        config.parsers.apply_defaults();
        config.insights.validate()?;
        config.insights.validate_layer_rules_file(atlas_dir)?;
        config.parsers.validate()?;
        config.retrieval.validate()?;
        config.sanitization.validate(atlas_dir)?;
        config.context.tokenizer.validate()?;
        config.context.escalation.validate()?;
        config.memory.validate()?;
        config.memory_embedding_backend()?;
        config.overview.validate()?;
        config.reachability.validate()?;
        Ok(config)
    }

    /// Build a runtime token counter from `context.tokenizer`.
    ///
    /// Relative `tokenizer_file` values resolve from `atlas_dir` (`.atlas/`),
    /// matching external config file resolution. Counting is always
    /// `tiktoken`; when the configured encoding cannot load, `fallback`
    /// decides between a heuristic counter with fallback metadata and a hard
    /// error.
    pub fn token_counter(&self, atlas_dir: &Utf8Path) -> Result<TokenCounterLoadResult> {
        let tokenizer = &self.context.tokenizer;
        tokenizer.validate()?;
        let encoding_name = resolve_tiktoken_encoding(tokenizer)?;
        let loaded: Result<atlas_token_count::TokenCounter> = if let Some(file) =
            tokenizer.tokenizer_file.as_deref()
        {
            let path = resolve_tokenizer_file(atlas_dir, file);
            atlas_token_count::TokenCounter::from_tiktoken_file_with_encoding(
                &path,
                encoding_name,
                tokenizer.model.clone(),
            )
            .with_context(|| {
                format!(
                    "invalid config: context.tokenizer.tokenizer_file={file} failed to load tiktoken ranks from {}",
                    path.display()
                )
            })
        } else if tokenizer.encoding.is_some() {
            atlas_token_count::TokenCounter::from_encoding(encoding_name, tokenizer.model.clone())
        } else if let Some(model) = tokenizer.model.as_deref() {
            atlas_token_count::TokenCounter::from_model(model)
        } else {
            atlas_token_count::TokenCounter::from_encoding(encoding_name, None)
        };
        match loaded {
            Ok(counter) => Ok(TokenCounterLoadResult {
                counter,
                fallback_used: false,
                fallback_reason: None,
            }),
            Err(err) => match tokenizer.fallback {
                TokenizerFallbackMode::Heuristic => Ok(TokenCounterLoadResult {
                    counter: atlas_token_count::TokenCounter::heuristic(tokenizer.bytes_per_token)?,
                    fallback_used: true,
                    fallback_reason: Some(err.to_string()),
                }),
                TokenizerFallbackMode::FailClosed => {
                    let detail = tiktoken_config_detail(tokenizer);
                    Err(err).with_context(|| {
                        format!("invalid config: {detail} failed to load a tiktoken encoding")
                    })
                }
            },
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
        if profile == ConfigTemplateProfile::Auto {
            anyhow::bail!(
                "auto profile requires hardware and repo probes; use Config::write_auto_template"
            );
        }
        let path = atlas_dir.join(crate::paths::ATLAS_CONFIG);
        if path.exists() {
            return Ok(false);
        }
        let content = Self::render_template(profile)?;
        fs::write(&path, content).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(true)
    }

    /// Write the tuned auto-profile config to `<atlas_dir>/config.toml`.
    ///
    /// Does not overwrite an existing file.
    pub fn write_auto_template(
        atlas_dir: &Path,
        system: &super::auto::SystemSnapshot,
        repo: &super::auto::RepoEstimate,
    ) -> Result<bool> {
        let path = atlas_dir.join(crate::paths::ATLAS_CONFIG);
        if path.exists() {
            return Ok(false);
        }
        let content = super::auto::render_auto_template(system, repo)?;
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

fn resolve_tiktoken_encoding(
    tokenizer: &super::context::ContextTokenizerConfig,
) -> Result<EncodingName> {
    if let Some(encoding) = tokenizer.encoding.as_deref() {
        return EncodingName::from_str(encoding).map_err(|_| {
            anyhow::anyhow!(
                "invalid config: context.tokenizer.encoding = \"{encoding}\" is not a known tiktoken encoding"
            )
        });
    }
    if let Some(model) = tokenizer.model.as_deref() {
        return atlas_tokenizer::encoding_name_for_model(model).with_context(|| {
            format!(
                "invalid config: context.tokenizer.model = \"{model}\" does not map to a tiktoken encoding"
            )
        });
    }
    Ok(EncodingName::default())
}

fn tiktoken_config_detail(tokenizer: &super::context::ContextTokenizerConfig) -> String {
    if let Some(file) = tokenizer.tokenizer_file.as_deref() {
        format!("context.tokenizer.tokenizer_file={file}")
    } else if let Some(encoding) = tokenizer.encoding.as_deref() {
        format!("context.tokenizer.encoding={encoding}")
    } else if let Some(model) = tokenizer.model.as_deref() {
        format!("context.tokenizer.model={model}")
    } else {
        "context.tokenizer".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_normalizes_profile_defaults_before_validation() {
        let dir = tempfile::tempdir().unwrap();
        let atlas_dir = dir.path();
        std::fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            "[[parsers.external]]\nextractor = \"php\"\ngrammar_dir = \"grammars/tree-sitter-php\"\n",
        )
        .unwrap();

        let config = Config::load(atlas_dir).unwrap();
        let entry = &config.parsers.external[0];
        assert_eq!(entry.language_name, "php");
        assert_eq!(entry.extensions, ["php"]);
        assert_eq!(entry.lib_function.as_deref(), Some("tree_sitter_php"));
        assert_eq!(
            entry.grammar_dir.as_deref(),
            Some(atlas_dir.join("grammars/tree-sitter-php").to_str().unwrap())
        );
    }

    #[test]
    fn load_resolves_query_path_from_atlas_dir() {
        let dir = tempfile::tempdir().unwrap();
        let atlas_dir = dir.path();
        std::fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            "[[parsers.external]]\nlanguage_name = \"zig\"\nextensions = [\"zig\"]\ngrammar_dir = \"grammars/tree-sitter-zig\"\nquery_path = \"queries/zig.scm\"\n",
        )
        .unwrap();

        let config = Config::load(atlas_dir).unwrap();
        let entry = &config.parsers.external[0];
        assert_eq!(
            entry.query_path.as_deref(),
            Some(atlas_dir.join("queries/zig.scm").to_str().unwrap())
        );
    }

    #[test]
    fn load_resolves_plugin_paths_and_defaults_working_dir() {
        let dir = tempfile::tempdir().unwrap();
        let atlas_dir = dir.path();
        std::fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            "[[parsers.external]]\nlanguage_name = \"custom\"\nextensions = [\"cust\"]\ncommand = \"bin/custom-parser\"\nargs = [\"--stdio\"]\n",
        )
        .unwrap();

        let config = Config::load(atlas_dir).unwrap();
        let entry = &config.parsers.external[0];
        assert_eq!(
            entry.command.as_deref(),
            Some(atlas_dir.join("bin/custom-parser").to_str().unwrap())
        );
        assert_eq!(
            entry.working_dir.as_deref(),
            atlas_dir.parent().map(|root| root.to_str().unwrap())
        );
        assert_eq!(entry.protocol.as_deref(), Some("atlas-parser.v1"));
        assert_eq!(entry.timeout_ms, Some(30_000));
        assert_eq!(entry.max_response_bytes, Some(16_777_216));
    }
}
