use super::*;
use std::fs;

/// Copy the vendored tree-sitter-rust grammar C sources into a tempdir so
/// the loader compiles it with the system C compiler (same path a user
/// grammar checkout would take).
fn vendored_rust_grammar() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let src_dir = dir.path().join("grammar").join("src");
    fs::create_dir_all(&src_dir).unwrap();

    let registry_dir = std::path::PathBuf::from(env!("CARGO_HOME")).join("registry/src");
    let mut found = None;
    for vendor_dir in fs::read_dir(&registry_dir).unwrap() {
        let vendor_dir = vendor_dir.unwrap().path();
        if !vendor_dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&vendor_dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("tree-sitter-rust-") && entry.path().join("src/parser.c").exists() {
                found = Some(entry.path().join("src"));
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let src = found.unwrap_or_else(|| {
        panic!(
            "vendored tree-sitter-rust grammar source not found under {}",
            registry_dir.display()
        )
    });
    for file in ["parser.c", "scanner.c", "grammar.json", "node-types.json"] {
        fs::copy(src.join(file), src_dir.join(file)).unwrap();
    }
    // parser.c/scanner.c include tree_sitter/*.h shipped next to them.
    fs::create_dir_all(src_dir.join("tree_sitter")).unwrap();
    for entry in fs::read_dir(src.join("tree_sitter")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(
            entry.path(),
            src_dir.join("tree_sitter").join(entry.file_name()),
        )
        .unwrap();
    }
    let grammar_path = dir.path().join("grammar");
    let grammar_dir = grammar_path.to_string_lossy().to_string();
    (dir, grammar_dir)
}

fn rust_external_config(grammar_dir: &str, lib_cache: &str) -> ExternalParserConfig {
    ExternalParserConfig {
        language_name: "rust".to_owned(),
        extensions: vec!["myrs".to_owned()],
        grammar_dir: Some(grammar_dir.to_owned()),
        grammar_lib_dir: Some(lib_cache.to_owned()),
        symbols: vec![
            ExternalSymbolRule {
                tree_kind: "function_item".to_owned(),
                node_kind: atlas_core::NodeKind::Function,
                name_field: "name".to_owned(),
                params_field: None,
                return_type_field: None,
                modifier_node_kinds: Vec::new(),
            },
            ExternalSymbolRule {
                tree_kind: "struct_item".to_owned(),
                node_kind: atlas_core::NodeKind::Struct,
                name_field: "name".to_owned(),
                params_field: None,
                return_type_field: None,
                modifier_node_kinds: Vec::new(),
            },
            ExternalSymbolRule {
                tree_kind: "impl_item".to_owned(),
                node_kind: atlas_core::NodeKind::Trait,
                name_field: "type".to_owned(),
                params_field: None,
                return_type_field: None,
                modifier_node_kinds: Vec::new(),
            },
        ],
        call_node_kinds: vec!["call_expression".to_owned()],
        call_target_field: Some("function".to_owned()),
        ..Default::default()
    }
}

#[test]
fn query_mode_parser_loads_grammar_and_extracts() {
    let (_dir, grammar_dir) = vendored_rust_grammar();
    let cache_dir = tempfile::tempdir().unwrap();
    let config = ExternalParserConfig {
        language_name: "rust".to_owned(),
        extensions: vec!["qrs".to_owned()],
        grammar_dir: Some(grammar_dir),
        grammar_lib_dir: Some(cache_dir.path().to_string_lossy().into_owned()),
        query: Some(
            "(function_item name: (identifier) @atlas.name) @atlas.definition.function".to_owned(),
        ),
        ..Default::default()
    };
    let parser = ExternalLangParser::new(&config).expect("query-mode parser loads grammar");
    assert!(parser.supports("src/main.qrs"));

    let source = b"fn greet() {}\n";
    let context = ParseContext {
        rel_path: "src/main.qrs",
        file_hash: "hash-query",
        source,
        old_tree: None,
    };
    let (parsed, tree) = parser.parse(&context);
    assert!(tree.is_some());
    assert!(
        parsed
            .nodes
            .iter()
            .any(|node| node.qualified_name == "src/main.qrs::function::greet")
    );
}

#[test]
fn external_parser_loads_grammar_and_extracts_symbols() {
    let (_dir, grammar_dir) = vendored_rust_grammar();
    let cache_dir = tempfile::tempdir().unwrap();
    let parser = ExternalLangParser::new(&rust_external_config(
        &grammar_dir,
        cache_dir.path().to_string_lossy().as_ref(),
    ))
    .unwrap();
    assert_eq!(parser.language_name(), "rust");
    assert!(parser.supports("src/main.myrs"));
    assert!(!parser.supports("src/main.rs"));

    let source =
        b"struct Greeter;\nfn greet() -> i32 {\n    Greeter\n}\nfn call() {\n    greet();\n}\n";
    let ctx = ParseContext {
        rel_path: "src/main.myrs",
        file_hash: "hash-1",
        source,
        old_tree: None,
    };
    let (parsed, tree) = parser.parse(&ctx);
    assert!(tree.is_some());
    let functions: Vec<&str> = parsed
        .nodes
        .iter()
        .filter(|node| node.kind == atlas_core::NodeKind::Function)
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(functions, ["greet", "call"]);
    assert!(
        parsed
            .nodes
            .iter()
            .any(|node| { node.kind == atlas_core::NodeKind::Struct && node.name == "Greeter" })
    );
    // Struct nests under the file qname.
    assert!(parsed.nodes.iter().any(|node| {
        node.kind == atlas_core::NodeKind::Struct
            && node.qualified_name == "src/main.myrs::struct::Greeter"
    }));
    // call edge greet() -> fn greet
    let call_edge = parsed
        .edges
        .iter()
        .find(|edge| edge.kind == atlas_core::EdgeKind::Calls)
        .expect("call edge");
    assert!(call_edge.target_qn.ends_with("::function::greet"));
    assert_eq!(call_edge.source_qn, "src/main.myrs::function::call");
}

#[test]
fn external_parser_config_validation() {
    let (_dir, grammar_dir) = vendored_rust_grammar();
    let cache_dir = tempfile::tempdir().unwrap();
    let mut config =
        rust_external_config(&grammar_dir, cache_dir.path().to_string_lossy().as_ref());
    config.extensions.clear();
    assert!(ExternalLangParser::new(&config).is_err());
    config.extensions = vec!["myrs".to_owned()];
    config.grammar_dir = None;
    assert!(ExternalLangParser::new(&config).is_err());
}

#[test]
fn profile_defaults_apply_and_validate() {
    let mut config = ExternalParserConfig {
        extractor: Some(ExternalExtractor::Php),
        grammar_dir: Some("grammars/tree-sitter-php".to_owned()),
        ..Default::default()
    };
    config.apply_defaults();
    assert_eq!(config.language_name, "php");
    assert_eq!(config.extensions, ["php"]);
    assert_eq!(config.lib_function.as_deref(), Some("tree_sitter_php"));
    config.validate().unwrap();
}

#[test]
fn csharp_and_java_profiles_use_expected_lib_functions() {
    assert_eq!(
        ExternalExtractor::CSharp.default_lib_function(),
        "tree_sitter_c_sharp"
    );
    assert_eq!(
        ExternalExtractor::Java.default_lib_function(),
        "tree_sitter_java"
    );
    assert_eq!(ExternalExtractor::CSharp.default_extensions(), &["cs"]);
    assert_eq!(ExternalExtractor::Java.default_extensions(), &["java"]);
}

#[test]
fn profile_language_name_must_match_canonical_name() {
    let mut config = ExternalParserConfig {
        extractor: Some(ExternalExtractor::Java),
        language_name: "jvm".to_owned(),
        grammar_dir: Some("grammars/tree-sitter-java".to_owned()),
        ..Default::default()
    };
    config.apply_defaults();
    let error = config.validate().expect_err("mismatch should fail");
    assert!(error.to_string().contains("canonical language_name"));
}

#[test]
fn profile_rejects_symbol_rules() {
    let mut config = ExternalParserConfig {
        extractor: Some(ExternalExtractor::Ruby),
        grammar_dir: Some("grammars/tree-sitter-ruby".to_owned()),
        symbols: vec![ExternalSymbolRule {
            tree_kind: "method".to_owned(),
            node_kind: atlas_core::NodeKind::Method,
            name_field: "name".to_owned(),
            params_field: None,
            return_type_field: None,
            modifier_node_kinds: Vec::new(),
        }],
        ..Default::default()
    };
    config.apply_defaults();
    let error = config.validate().expect_err("symbols should fail");
    assert!(error.to_string().contains("must not declare symbol rules"));
}

#[test]
fn profile_dispatch_preserves_language_and_overrides_extensions() {
    let mut config = ExternalParserConfig {
        extractor: Some(ExternalExtractor::Php),
        extensions: vec!["phtml".to_owned()],
        grammar_dir: Some("grammars/tree-sitter-php".to_owned()),
        ..Default::default()
    };
    config.apply_defaults();
    let parser = ExternalExtractor::Php
        .build(
            tree_sitter_php::LANGUAGE_PHP.into(),
            config.extensions.clone(),
        )
        .unwrap();

    assert_eq!(parser.language_name().as_ref(), "php");
    assert!(parser.supports("src/index.phtml"));
    assert!(!parser.supports("src/index.php"));

    let source = b"<?php\nclass Runner {}\n";
    let ctx = ParseContext {
        rel_path: "src/index.phtml",
        file_hash: "hash-1",
        source,
        old_tree: None,
    };
    let (parsed, _) = parser.parse(&ctx);
    assert_eq!(parsed.language.as_deref(), Some("php"));
    assert!(parsed.nodes.iter().any(|node| {
        node.kind == atlas_core::NodeKind::Class
            && node.qualified_name == "src/index.phtml::class::Runner"
    }));
}

#[test]
fn from_language_dispatches_profile_and_overrides_extensions() {
    let config = ExternalParserConfig {
        extractor: Some(ExternalExtractor::Php),
        extensions: vec!["phtml".to_owned()],
        grammar_dir: Some("grammars/tree-sitter-php".to_owned()),
        ..Default::default()
    };
    let parser =
        ExternalLangParser::from_language(&config, tree_sitter_php::LANGUAGE_PHP.into()).unwrap();

    assert_eq!(parser.language_name(), "php");
    assert!(parser.supports("src/index.phtml"));
    assert!(!parser.supports("src/index.php"));
}

#[test]
fn profile_rejects_generic_extraction_fields() {
    let mut config = ExternalParserConfig {
        extractor: Some(ExternalExtractor::Ruby),
        grammar_dir: Some("grammars/tree-sitter-ruby".to_owned()),
        imports: vec![ExternalImportRule {
            tree_kind: "call".to_owned(),
            name_field: None,
        }],
        ..Default::default()
    };
    config.apply_defaults();
    let error = config.validate().expect_err("generic fields should fail");
    assert!(error.to_string().contains("generic extraction fields"));
}

#[test]
fn profile_query_validation_failure_is_reported() {
    let error = ExternalExtractor::Php
        .build(tree_sitter_c::LANGUAGE.into(), vec!["php".to_owned()])
        .err()
        .expect("php query should not compile against the C grammar");
    assert!(error.to_string().contains("php query compile failed"));
}

fn rust_fidelity_config(grammar_dir: &str, lib_cache: &str) -> ExternalParserConfig {
    ExternalParserConfig {
        language_name: "rust".to_owned(),
        extensions: vec!["myrs".to_owned()],
        grammar_dir: Some(grammar_dir.to_owned()),
        grammar_lib_dir: Some(lib_cache.to_owned()),
        symbols: vec![
            ExternalSymbolRule {
                tree_kind: "function_item".to_owned(),
                node_kind: atlas_core::NodeKind::Function,
                name_field: "name".to_owned(),
                params_field: Some("parameters".to_owned()),
                return_type_field: Some("return_type".to_owned()),
                modifier_node_kinds: vec!["visibility_modifier".to_owned()],
            },
            ExternalSymbolRule {
                tree_kind: "mod_item".to_owned(),
                node_kind: atlas_core::NodeKind::Module,
                name_field: "name".to_owned(),
                params_field: None,
                return_type_field: None,
                modifier_node_kinds: Vec::new(),
            },
        ],
        imports: vec![ExternalImportRule {
            tree_kind: "use_declaration".to_owned(),
            name_field: None,
        }],
        attributes: vec![ExternalAttributeRule {
            tree_kind: "attribute_item".to_owned(),
            name_field: "name".to_owned(),
            attach_to: ExternalAttributeTarget::NextSymbol,
        }],
        qn_style: ExternalQnStyle::FlatOwner,
        owner_node_kinds: vec![atlas_core::NodeKind::Module],
        test_name_patterns: vec!["^test_".to_owned()],
        call_node_kinds: vec!["call_expression".to_owned()],
        call_target_field: Some("function".to_owned()),
        ..Default::default()
    }
}

#[test]
fn generic_external_fidelity_rules_cover_imports_attributes_metadata_qn_and_calls() {
    let (_dir, grammar_dir) = vendored_rust_grammar();
    let cache_dir = tempfile::tempdir().unwrap();
    let parser = ExternalLangParser::new(&rust_fidelity_config(
        &grammar_dir,
        cache_dir.path().to_string_lossy().as_ref(),
    ))
    .unwrap();
    let source = br#"use std::io;
#[inline]
pub fn test_alpha(x: i32) -> i32 { x }
mod helpers {
fn inner() {}
}
pub fn caller() {
test_alpha(1);
missing();
}
"#;
    let ctx = ParseContext {
        rel_path: "src/fidelity.myrs",
        file_hash: "hash-fidelity",
        source,
        old_tree: None,
    };
    let (parsed, _) = parser.parse(&ctx);

    let import = parsed
        .nodes
        .iter()
        .find(|node| node.kind == atlas_core::NodeKind::Import && node.name == "std::io")
        .expect("import node");
    assert!(parsed.edges.iter().any(|edge| {
        edge.kind == atlas_core::EdgeKind::Imports
            && edge.source_qn == "src/fidelity.myrs"
            && edge.target_qn == import.qualified_name
    }));

    let test_alpha = parsed
        .nodes
        .iter()
        .find(|node| node.name == "test_alpha")
        .expect("test_alpha node");
    let attribute = parsed
        .nodes
        .iter()
        .find(|node| node.kind == atlas_core::NodeKind::Variable && node.name.contains("inline"))
        .expect("attribute node");
    assert_eq!(
        attribute.parent_name.as_deref(),
        Some(test_alpha.qualified_name.as_str())
    );
    assert!(parsed.edges.iter().any(|edge| {
        edge.kind == atlas_core::EdgeKind::Contains
            && edge.source_qn == test_alpha.qualified_name
            && edge.target_qn == attribute.qualified_name
    }));

    assert!(
        test_alpha
            .params
            .as_deref()
            .is_some_and(|params| params.contains("i32"))
    );
    assert_eq!(test_alpha.return_type.as_deref(), Some("i32"));
    assert!(test_alpha.is_test);

    let caller = parsed
        .nodes
        .iter()
        .find(|node| node.name == "caller")
        .expect("caller node");
    assert_eq!(caller.modifiers.as_deref(), Some("pub"));
    assert!(!caller.is_test);

    let inner = parsed
        .nodes
        .iter()
        .find(|node| node.name == "inner")
        .expect("inner node");
    assert_eq!(
        inner.qualified_name,
        "src/fidelity.myrs::function::helpers.inner"
    );
    assert_eq!(
        inner.parent_name.as_deref(),
        Some("src/fidelity.myrs::module::helpers")
    );

    let resolved = parsed
        .edges
        .iter()
        .find(|edge| {
            edge.kind == atlas_core::EdgeKind::Calls && edge.target_qn == test_alpha.qualified_name
        })
        .expect("resolved call edge");
    assert_eq!(resolved.confidence, 0.8);
    assert_eq!(resolved.confidence_tier.as_deref(), Some("same_file"));

    let unresolved = parsed
        .edges
        .iter()
        .find(|edge| {
            edge.kind == atlas_core::EdgeKind::Calls
                && edge.target_qn == "src/fidelity.myrs::missing"
        })
        .expect("unresolved call edge");
    assert_eq!(unresolved.confidence, 0.3);
    assert_eq!(unresolved.confidence_tier.as_deref(), Some("text"));
}

#[test]
fn generic_external_scope_attributes_attach_to_current_scope() {
    let mut config = rust_fidelity_config("unused", "unused");
    config.attributes[0].attach_to = ExternalAttributeTarget::Scope;
    let parser =
        ExternalLangParser::from_language(&config, tree_sitter_rust::LANGUAGE.into()).unwrap();
    let source = b"#[inline]\nfn plain() {}\n";
    let ctx = ParseContext {
        rel_path: "src/scope.myrs",
        file_hash: "hash-scope",
        source,
        old_tree: None,
    };
    let (parsed, _) = parser.parse(&ctx);

    let attribute = parsed
        .nodes
        .iter()
        .find(|node| node.kind == atlas_core::NodeKind::Variable && node.name.contains("inline"))
        .expect("scope attribute node");
    assert_eq!(attribute.parent_name.as_deref(), Some("src/scope.myrs"));
    assert!(parsed.edges.iter().any(|edge| {
        edge.kind == atlas_core::EdgeKind::Contains
            && edge.source_qn == "src/scope.myrs"
            && edge.target_qn == attribute.qualified_name
    }));
}

#[test]
fn generic_external_rejects_invalid_test_regex() {
    let mut config = rust_fidelity_config("unused", "unused");
    config.test_name_patterns = vec!["(".to_owned()];
    let error = config.validate().expect_err("invalid regex should fail");
    assert!(error.to_string().contains("invalid test regex"));
}
