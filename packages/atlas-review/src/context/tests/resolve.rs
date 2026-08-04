use super::*;

#[test]
fn resolve_exact_qname_hit() {
    let mut store = open_store();
    seed_graph(&mut store);
    let target = ContextTarget::QualifiedName {
        qname: "src/a.rs::fn_a".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(matches!(resolved, ResolvedTarget::Node(n) if n.qualified_name == "src/a.rs::fn_a"));
}

#[test]
fn resolve_exact_qname_miss_returns_not_found_or_ambiguous() {
    let mut store = open_store();
    seed_graph(&mut store);
    let target = ContextTarget::QualifiedName {
        qname: "nonexistent::qname".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(matches!(
        resolved,
        ResolvedTarget::NotFound { .. } | ResolvedTarget::Ambiguous(..)
    ));
}

#[test]
fn resolve_unique_symbol_name() {
    let mut store = open_store();
    seed_graph(&mut store);
    let target = ContextTarget::SymbolName {
        name: "fn_a".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(matches!(resolved, ResolvedTarget::Node(n) if n.name == "fn_a"));
}

#[test]
fn resolve_ambiguous_symbol_name() {
    let mut store = open_store();
    let dupe = ParsedFile {
        path: "src/c.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "h4".to_string(),
        size: None,
        nodes: vec![make_node(
            "src/c.rs::fn_a",
            "fn_a",
            "src/c.rs",
            NodeKind::Function,
            None,
        )],
        edges: vec![],
    };
    store.replace_batch(&[dupe]).unwrap();
    seed_graph(&mut store);

    let target = ContextTarget::SymbolName {
        name: "fn_a".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(matches!(resolved, ResolvedTarget::Ambiguous(ref m) if m.candidates.len() >= 2));
}

#[test]
fn resolve_file_path_hit() {
    let mut store = open_store();
    seed_graph(&mut store);
    let target = ContextTarget::FilePath {
        path: "src/a.rs".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(matches!(resolved, ResolvedTarget::File(p) if p == "src/a.rs"));
}

#[test]
fn resolve_file_path_miss_returns_not_found() {
    let mut store = open_store();
    seed_graph(&mut store);
    let target = ContextTarget::FilePath {
        path: "src/missing.rs".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(matches!(resolved, ResolvedTarget::NotFound { .. }));
}

#[test]
fn resolve_missing_symbol_returns_not_found() {
    let mut store = open_store();
    seed_graph(&mut store);
    let target = ContextTarget::SymbolName {
        name: "zzz_totally_absent".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(matches!(
        resolved,
        ResolvedTarget::NotFound { .. } | ResolvedTarget::Ambiguous(..)
    ));
}

#[test]
fn normalize_qn_kind_tokens_function_alias() {
    assert_eq!(
        normalize_qn_kind_tokens("src/lib.rs::function::foo"),
        "src/lib.rs::fn::foo"
    );
    assert_eq!(
        normalize_qn_kind_tokens("src/lib.rs::func::foo"),
        "src/lib.rs::fn::foo"
    );
    assert_eq!(
        normalize_qn_kind_tokens("src/lib.rs::fn::foo"),
        "src/lib.rs::fn::foo"
    );
}

#[test]
fn normalize_qn_kind_tokens_other_aliases() {
    assert_eq!(
        normalize_qn_kind_tokens("pkg/a.go::meth::T.Run"),
        "pkg/a.go::method::T.Run"
    );
    assert_eq!(
        normalize_qn_kind_tokens("src/a.rs::constant::MAX"),
        "src/a.rs::const::MAX"
    );
    assert_eq!(
        normalize_qn_kind_tokens("src/a.rs::struct::Foo"),
        "src/a.rs::struct::Foo"
    );
    assert_eq!(normalize_qn_kind_tokens("just_a_name"), "just_a_name");
}

#[test]
fn resolve_qname_with_function_alias_resolves_via_normalisation() {
    let mut store = open_store();
    let canonical_qn = "src/x.rs::fn::my_fn";
    let file = ParsedFile {
        path: "src/x.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "hx".to_string(),
        size: None,
        nodes: vec![make_node(
            canonical_qn,
            "my_fn",
            "src/x.rs",
            NodeKind::Function,
            None,
        )],
        edges: vec![],
    };
    store.replace_batch(&[file]).unwrap();

    let target = ContextTarget::QualifiedName {
        qname: "src/x.rs::function::my_fn".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(
        matches!(resolved, ResolvedTarget::Node(ref n) if n.qualified_name == canonical_qn),
        "expected canonical node, got: {resolved:?}"
    );
}

#[test]
fn resolve_qname_alias_miss_returns_not_found_or_suggestions() {
    let mut store = open_store();
    store.migrate().unwrap();
    let target = ContextTarget::QualifiedName {
        qname: "no/such/file.rs::function::missing".to_string(),
    };
    let resolved = resolve_target(&store, &target).unwrap();
    assert!(matches!(
        resolved,
        ResolvedTarget::NotFound { .. } | ResolvedTarget::Ambiguous(..)
    ));
}
