//! `[parsers]` configuration: external (config-driven) language parsers.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use atlas_parser::ExternalParserConfig;

/// Parser configuration surface.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ParsersConfig {
    /// External languages registered on top of the built-in handlers.
    #[serde(default)]
    pub external: Vec<ExternalParserConfig>,
}

impl ParsersConfig {
    /// Resolve relative grammar paths against `atlas_dir` (`.atlas/`),
    /// matching the tokenizer/redaction-file resolution convention so configs
    /// stay portable across machines.
    pub fn resolve_from(&mut self, atlas_dir: &std::path::Path) {
        for config in &mut self.external {
            for value in [
                &mut config.grammar_dir,
                &mut config.lib_path,
                &mut config.grammar_lib_dir,
            ]
            .into_iter()
            .flatten()
            {
                let candidate = std::path::Path::new(value);
                if candidate.is_relative() {
                    *value = atlas_dir.join(candidate).to_string_lossy().into_owned();
                }
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        let mut seen_extensions: Vec<&str> = Vec::new();
        for config in &self.external {
            ensure!(
                !config.language_name.is_empty(),
                "parsers.external entry must declare a language_name"
            );
            ensure!(
                !config.extensions.is_empty(),
                "parsers.external '{}' must declare at least one extension",
                config.language_name
            );
            ensure!(
                config.grammar_dir.is_some() != config.lib_path.is_some(),
                "parsers.external '{}' must configure exactly one of grammar_dir or lib_path",
                config.language_name
            );
            ensure!(
                !config.symbols.is_empty(),
                "parsers.external '{}' must declare at least one symbol rule",
                config.language_name
            );
            if let Some(function) = &config.lib_function {
                ensure!(
                    !function.is_empty(),
                    "parsers.external '{}' has an empty lib_function",
                    config.language_name
                );
            }
            for rule in &config.symbols {
                ensure!(
                    !rule.tree_kind.is_empty() && !rule.name_field.is_empty(),
                    "parsers.external '{}' symbol rules need non-empty tree_kind and name_field",
                    config.language_name
                );
            }
            for extension in &config.extensions {
                ensure!(
                    !extension.is_empty() && !extension.starts_with('.'),
                    "parsers.external '{}' has invalid extension '{extension}' (no leading dot)",
                    config.language_name
                );
                ensure!(
                    !seen_extensions.contains(&extension.as_str()),
                    "parsers.external extension '{extension}' registered more than once"
                );
                seen_extensions.push(extension);
            }
        }
        Ok(())
    }
}
