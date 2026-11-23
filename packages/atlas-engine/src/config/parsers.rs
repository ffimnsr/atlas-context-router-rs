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
    /// Fill profile defaults for every extractor entry before validation.
    pub fn apply_defaults(&mut self) {
        for config in &mut self.external {
            config.apply_defaults();
        }
    }

    /// Resolve relative grammar paths against `atlas_dir` (`.atlas/`),
    /// matching the tokenizer/redaction-file resolution convention so configs
    /// stay portable across machines.
    pub fn resolve_from(&mut self, atlas_dir: &std::path::Path) {
        for config in &mut self.external {
            for value in [
                &mut config.grammar_dir,
                &mut config.lib_path,
                &mut config.grammar_lib_dir,
                &mut config.query_path,
                &mut config.command,
                &mut config.working_dir,
            ]
            .into_iter()
            .flatten()
            {
                let candidate = std::path::Path::new(value);
                if candidate.is_relative() {
                    *value = atlas_dir.join(candidate).to_string_lossy().into_owned();
                }
            }
            // Plugin working directory defaults to the repo root (the parent of
            // `.atlas/`), matching the protocol contract.
            if config.command.is_some() && config.working_dir.is_none() {
                let repo_root = atlas_dir.parent().unwrap_or(atlas_dir);
                config.working_dir = Some(repo_root.to_string_lossy().into_owned());
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        let mut seen_extensions: Vec<&str> = Vec::new();
        for config in &self.external {
            config.validate()?;
            for extension in &config.extensions {
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

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_parser::{ExternalExtractor, ExternalSymbolRule};

    fn profile(extractor: ExternalExtractor) -> ExternalParserConfig {
        ExternalParserConfig {
            extractor: Some(extractor),
            grammar_dir: Some("grammars/test".to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn profile_defaults_are_applied_before_validation() {
        let mut config = ParsersConfig {
            external: vec![profile(ExternalExtractor::Php)],
        };
        config.apply_defaults();
        config.validate().unwrap();

        let entry = &config.external[0];
        assert_eq!(entry.language_name, "php");
        assert_eq!(entry.extensions, ["php"]);
        assert_eq!(entry.lib_function.as_deref(), Some("tree_sitter_php"));
    }

    #[test]
    fn profile_language_name_mismatch_is_rejected() {
        let mut config = ParsersConfig {
            external: vec![ExternalParserConfig {
                language_name: "jvm".to_owned(),
                ..profile(ExternalExtractor::Java)
            }],
        };
        config.apply_defaults();
        let error = config.validate().expect_err("mismatch should fail");
        assert!(error.to_string().contains("canonical language_name"));
    }

    #[test]
    fn profile_symbol_rules_are_rejected() {
        let mut entry = profile(ExternalExtractor::Ruby);
        entry.symbols.push(ExternalSymbolRule {
            tree_kind: "method".to_owned(),
            node_kind: atlas_core::NodeKind::Method,
            name_field: "name".to_owned(),
            params_field: None,
            return_type_field: None,
            modifier_node_kinds: Vec::new(),
        });
        let mut config = ParsersConfig {
            external: vec![entry],
        };
        config.apply_defaults();
        let error = config.validate().expect_err("symbols should fail");
        assert!(error.to_string().contains("must not declare symbol rules"));
    }

    #[test]
    fn generic_config_still_requires_a_mode() {
        let config = ParsersConfig {
            external: vec![ExternalParserConfig {
                language_name: "zig".to_owned(),
                extensions: vec!["zig".to_owned()],
                grammar_dir: Some("grammars/tree-sitter-zig".to_owned()),
                ..Default::default()
            }],
        };
        let error = config
            .validate()
            .expect_err("an extraction mode is required");
        assert!(
            error
                .to_string()
                .contains("must declare an extractor, symbol rules, query, or command")
        );
    }

    #[test]
    fn query_mode_validates_and_rejects_rule_fields() {
        let mut entry = ExternalParserConfig {
            language_name: "zig".to_owned(),
            extensions: vec!["zig".to_owned()],
            grammar_dir: Some("grammars/tree-sitter-zig".to_owned()),
            query: Some("(function_declaration) @atlas.definition.function".to_owned()),
            ..Default::default()
        };
        entry.validate().expect("query mode should validate");

        entry.symbols.push(ExternalSymbolRule {
            tree_kind: "function_declaration".to_owned(),
            node_kind: atlas_core::NodeKind::Function,
            name_field: "name".to_owned(),
            params_field: None,
            return_type_field: None,
            modifier_node_kinds: Vec::new(),
        });
        let error = entry.validate().expect_err("symbols and query conflict");
        assert!(error.to_string().contains("must not declare"));
    }

    fn plugin_entry() -> ExternalParserConfig {
        ExternalParserConfig {
            language_name: "custom".to_owned(),
            extensions: vec!["cust".to_owned()],
            command: Some("bin/custom-parser".to_owned()),
            args: vec!["--stdio".to_owned()],
            ..Default::default()
        }
    }

    #[test]
    fn plugin_mode_applies_defaults_and_validates() {
        let mut entry = plugin_entry();
        entry.apply_defaults();
        entry.validate().expect("plugin mode should validate");
        assert_eq!(entry.protocol.as_deref(), Some("atlas-parser.v1"));
        assert_eq!(entry.timeout_ms, Some(30_000));
        assert_eq!(entry.max_response_bytes, Some(16_777_216));
    }

    #[test]
    fn plugin_mode_rejects_conflicting_fields() {
        let mut grammar = plugin_entry();
        grammar.grammar_dir = Some("grammars/tree-sitter-zig".to_owned());
        let error = grammar.validate().expect_err("grammar conflicts");
        assert!(
            error.to_string().contains("must not declare grammar_dir"),
            "unexpected error: {error}"
        );

        let mut rules = plugin_entry();
        rules.symbols.push(ExternalSymbolRule {
            tree_kind: "function_declaration".to_owned(),
            node_kind: atlas_core::NodeKind::Function,
            name_field: "name".to_owned(),
            params_field: None,
            return_type_field: None,
            modifier_node_kinds: Vec::new(),
        });
        let error = rules.validate().expect_err("rule fields conflict");
        assert!(
            error.to_string().contains("must not declare"),
            "unexpected error: {error}"
        );

        let mut protocol = plugin_entry();
        protocol.protocol = Some("atlas-parser.v2".to_owned());
        let error = protocol.validate().expect_err("unsupported protocol");
        assert!(
            error.to_string().contains("unsupported protocol"),
            "unexpected error: {error}"
        );

        let mut timeout = plugin_entry();
        timeout.timeout_ms = Some(0);
        let error = timeout.validate().expect_err("zero timeout");
        assert!(
            error.to_string().contains("timeout_ms"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn profile_and_rule_modes_reject_plugin_fields() {
        let mut profile_entry = profile(ExternalExtractor::Php);
        profile_entry.command = Some("bin/parser".to_owned());
        let mut config = ParsersConfig {
            external: vec![profile_entry],
        };
        config.apply_defaults();
        let error = config
            .validate()
            .expect_err("profile rejects plugin fields");
        assert!(
            error.to_string().contains("must not declare plugin fields"),
            "unexpected error: {error}"
        );

        let rule_entry = ExternalParserConfig {
            language_name: "zig".to_owned(),
            extensions: vec!["zig".to_owned()],
            grammar_dir: Some("grammars/tree-sitter-zig".to_owned()),
            symbols: vec![ExternalSymbolRule {
                tree_kind: "function_declaration".to_owned(),
                node_kind: atlas_core::NodeKind::Function,
                name_field: "name".to_owned(),
                params_field: None,
                return_type_field: None,
                modifier_node_kinds: Vec::new(),
            }],
            args: vec!["--stdio".to_owned()],
            ..Default::default()
        };
        let error = rule_entry
            .validate()
            .expect_err("rules reject plugin fields");
        assert!(
            error.to_string().contains("must not declare plugin fields"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn query_mode_requires_exactly_one_query_source() {
        let mut entry = ExternalParserConfig {
            language_name: "zig".to_owned(),
            extensions: vec!["zig".to_owned()],
            grammar_dir: Some("grammars/tree-sitter-zig".to_owned()),
            query: Some("(function_declaration) @atlas.definition.function".to_owned()),
            query_path: Some("queries/zig.scm".to_owned()),
            ..Default::default()
        };
        let error = entry.validate().expect_err("two query sources should fail");
        assert!(
            error
                .to_string()
                .contains("exactly one of query_path or query")
        );

        entry.query = None;
        entry.query_path = None;
        let error = entry.validate().expect_err("no mode should fail");
        assert!(error.to_string().contains("must declare an extractor"));
    }

    #[test]
    fn query_fields_are_rejected_by_profile_and_rule_modes() {
        let mut profile_entry = profile(ExternalExtractor::Php);
        profile_entry.query = Some("(function_declaration) @atlas.definition.function".to_owned());
        let mut config = ParsersConfig {
            external: vec![profile_entry],
        };
        config.apply_defaults();
        let error = config.validate().expect_err("profile rejects query fields");
        assert!(error.to_string().contains("must not declare query fields"));

        let mut rule_entry = ExternalParserConfig {
            language_name: "zig".to_owned(),
            extensions: vec!["zig".to_owned()],
            grammar_dir: Some("grammars/tree-sitter-zig".to_owned()),
            symbols: vec![ExternalSymbolRule {
                tree_kind: "function_declaration".to_owned(),
                node_kind: atlas_core::NodeKind::Function,
                name_field: "name".to_owned(),
                params_field: None,
                return_type_field: None,
                modifier_node_kinds: Vec::new(),
            }],
            ..Default::default()
        };
        rule_entry.query_path = Some("queries/zig.scm".to_owned());
        let error = rule_entry
            .validate()
            .expect_err("rules reject query fields");
        assert!(error.to_string().contains("rule mode must not declare"));
    }

    #[test]
    fn duplicate_profile_extensions_are_rejected() {
        let mut config = ParsersConfig {
            external: vec![
                profile(ExternalExtractor::Php),
                profile(ExternalExtractor::Php),
            ],
        };
        config.apply_defaults();
        let error = config
            .validate()
            .expect_err("duplicate extensions should fail");
        assert!(error.to_string().contains("registered more than once"));
    }

    #[test]
    fn profile_requires_exactly_one_grammar_source() {
        let mut entry = profile(ExternalExtractor::Bash);
        entry.lib_path = Some("grammars/libtree-sitter-bash.so".to_owned());
        let mut config = ParsersConfig {
            external: vec![entry],
        };
        config.apply_defaults();
        let error = config
            .validate()
            .expect_err("both grammar sources should fail");
        assert!(
            error
                .to_string()
                .contains("exactly one of grammar_dir or lib_path")
        );
    }
}
