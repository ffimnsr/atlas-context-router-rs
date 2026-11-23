//! Parser configuration fingerprint used to keep incremental updates aligned
//! with the parser set that produced the graph.

use atlas_parser::{ExternalParserConfig, ParserRegistry};
use sha2::{Digest, Sha256};

/// Bump when parser extraction behavior changes without a config shape change.
pub const PARSER_FINGERPRINT_REVISION: u32 = 1;

/// Compute a stable SHA-256 fingerprint over the default registry identity and
/// the normalized external parser configuration.
pub fn compute(external_parsers: &[ExternalParserConfig]) -> String {
    let mut normalized = external_parsers.to_vec();
    for config in &mut normalized {
        config.apply_defaults();
    }

    let mut default_languages = ParserRegistry::with_defaults()
        .supported_languages()
        .into_iter()
        .map(|language| language.into_owned())
        .collect::<Vec<_>>();
    default_languages.sort();
    default_languages.dedup();

    let entries = normalized
        .iter()
        .map(|config| {
            serde_json::json!({
                "mode": extraction_mode(config),
                "config": config,
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::json!({
        "revision": PARSER_FINGERPRINT_REVISION,
        "default_languages": default_languages,
        "external_parsers": entries,
    });
    let bytes = serde_json::to_vec(&payload).expect("parser fingerprint payload is serializable");
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

fn extraction_mode(config: &ExternalParserConfig) -> &'static str {
    config
        .extraction_mode()
        .map_or("unknown", |mode| mode.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::NodeKind;
    use atlas_parser::{
        ExternalAttributeRule, ExternalExtractor, ExternalImportRule, ExternalQnStyle,
        ExternalSymbolRule,
    };

    fn generic_config() -> ExternalParserConfig {
        ExternalParserConfig {
            language_name: "zig".to_owned(),
            extensions: vec!["zig".to_owned()],
            grammar_dir: Some("grammars/tree-sitter-zig".to_owned()),
            symbols: vec![ExternalSymbolRule {
                tree_kind: "function_declaration".to_owned(),
                node_kind: NodeKind::Function,
                name_field: "name".to_owned(),
                params_field: Some("parameters".to_owned()),
                return_type_field: None,
                modifier_node_kinds: Vec::new(),
            }],
            imports: vec![ExternalImportRule {
                tree_kind: "import_declaration".to_owned(),
                name_field: None,
            }],
            attributes: vec![ExternalAttributeRule {
                tree_kind: "attribute".to_owned(),
                name_field: "name".to_owned(),
                attach_to: Default::default(),
            }],
            qn_style: ExternalQnStyle::Nested,
            ..Default::default()
        }
    }

    #[test]
    fn fingerprint_is_stable() {
        let config = generic_config();
        let first = config.clone();
        assert_eq!(compute(&[first]), compute(&[config]));
    }

    #[test]
    fn fingerprint_normalizes_profile_defaults() {
        let raw = ExternalParserConfig {
            extractor: Some(ExternalExtractor::Php),
            grammar_dir: Some("grammars/tree-sitter-php".to_owned()),
            ..Default::default()
        };
        let mut normalized = raw.clone();
        normalized.apply_defaults();
        assert_eq!(compute(&[raw]), compute(&[normalized]));
    }

    #[test]
    fn fingerprint_normalizes_plugin_defaults() {
        let raw = raw_plugin();
        let mut normalized = raw.clone();
        normalized.apply_defaults();
        assert_eq!(
            compute(std::slice::from_ref(&raw)),
            compute(std::slice::from_ref(&normalized))
        );

        let mut changed = raw_plugin();
        changed.command = Some("bin/other-parser".to_owned());
        assert_ne!(compute(&[raw_plugin()]), compute(&[changed]));
    }

    fn raw_plugin() -> ExternalParserConfig {
        ExternalParserConfig {
            language_name: "custom".to_owned(),
            extensions: vec!["cust".to_owned()],
            command: Some("bin/custom-parser".to_owned()),
            args: vec!["--stdio".to_owned()],
            ..Default::default()
        }
    }

    #[test]
    fn fingerprint_includes_query_mode() {
        let rules = generic_config();
        let mut query = generic_config();
        query.symbols.clear();
        query.imports.clear();
        query.attributes.clear();
        query.query = Some("(function_declaration) @atlas.definition.function".to_owned());
        assert_ne!(compute(&[rules]), compute(&[query]));
    }

    #[test]
    fn fingerprint_changes_with_parser_config() {
        let mut changed_grammar = generic_config();
        changed_grammar.grammar_dir = Some("grammars/tree-sitter-other".to_owned());
        assert_ne!(compute(&[generic_config()]), compute(&[changed_grammar]));

        let mut changed_rules = generic_config();
        changed_rules.symbols[0].name_field = "identifier".to_owned();
        assert_ne!(compute(&[generic_config()]), compute(&[changed_rules]));

        let mut changed_tests = generic_config();
        changed_tests.test_name_patterns = vec!["^test_".to_owned()];
        assert_ne!(compute(&[generic_config()]), compute(&[changed_tests]));
    }
}
