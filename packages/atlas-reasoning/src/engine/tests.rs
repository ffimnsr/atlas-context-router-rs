use super::*;
use atlas_core::{
    Edge, EdgeKind, ImpactClass, Node, NodeId, NodeKind, PackageOwner, PackageOwnerKind,
    ReferenceScope, SafetyBand,
};
use atlas_store_sqlite::Store;

fn make_store() -> Store {
    let mut store = Store::open(":memory:").unwrap();
    store.migrate().unwrap();
    store
}

fn node(id: i64, name: &str, qname: &str, file: &str, kind: NodeKind) -> Node {
    Node {
        id: NodeId(id),
        kind,
        name: name.to_owned(),
        qualified_name: qname.to_owned(),
        file_path: file.to_owned(),
        line_start: 1,
        line_end: 10,
        language: "rust".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: String::new(),
        extra_json: serde_json::Value::Null,
    }
}

fn edge(src: &str, tgt: &str, kind: EdgeKind, file: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: src.to_owned(),
        target_qn: tgt.to_owned(),
        file_path: file.to_owned(),
        line: None,
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::Value::Null,
    }
}

fn seed_graph(store: &mut Store, nodes: Vec<Node>, edges: Vec<Edge>) {
    let mut files: std::collections::HashMap<String, (Vec<Node>, Vec<Edge>)> = Default::default();
    for node in nodes {
        files
            .entry(node.file_path.clone())
            .or_default()
            .0
            .push(node);
    }
    for edge in edges {
        files
            .entry(edge.file_path.clone())
            .or_default()
            .1
            .push(edge);
    }
    for (path, (nodes, edges)) in files {
        let language = nodes.first().map(|node| node.language.clone());
        store
            .replace_file_graph(&path, "hash", language.as_deref(), None, &nodes, &edges)
            .unwrap();
    }
}

fn attach_owner(store: &mut Store, path: &str, manifest_path: &str) {
    let root = manifest_path
        .rsplit_once('/')
        .map(|(prefix, _)| prefix)
        .unwrap_or("");
    let owner = PackageOwner {
        owner_id: format!("cargo:{manifest_path}"),
        kind: PackageOwnerKind::Cargo,
        root: root.to_owned(),
        manifest_path: manifest_path.to_owned(),
        package_name: manifest_path.split('/').rev().nth(1).map(str::to_owned),
    };
    store.upsert_file_owner(path, Some(&owner)).unwrap();
}

#[test]
fn removal_simple_call_graph() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
        node(0, "fn_b", "src/b.rs::fn_b", "src/b.rs", NodeKind::Function),
    ];
    let edges = vec![edge(
        "src/b.rs::fn_b",
        "src/a.rs::fn_a",
        EdgeKind::Calls,
        "src/b.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .analyze_removal(&["src/a.rs::fn_a"], None, None)
        .unwrap();

    assert!(!result.seed.is_empty(), "seed should resolve");
    assert!(
        result
            .impacted_symbols
            .iter()
            .any(|im| im.node.qualified_name == "src/b.rs::fn_b"),
        "fn_b should be in impacted symbols"
    );
}

#[test]
fn removal_cyclic_graph() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
        node(0, "fn_b", "src/b.rs::fn_b", "src/b.rs", NodeKind::Function),
    ];
    let edges = vec![
        edge(
            "src/a.rs::fn_a",
            "src/b.rs::fn_b",
            EdgeKind::Calls,
            "src/a.rs",
        ),
        edge(
            "src/b.rs::fn_b",
            "src/a.rs::fn_a",
            EdgeKind::Calls,
            "src/b.rs",
        ),
    ];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .analyze_removal(&["src/a.rs::fn_a"], Some(5), Some(100))
        .unwrap();

    assert!(
        result
            .impacted_symbols
            .iter()
            .any(|im| im.node.qualified_name == "src/b.rs::fn_b")
    );
}

#[test]
fn removal_normalizes_function_alias_qname() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "fn_a",
            "src/a.rs::fn::fn_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "fn_b",
            "src/b.rs::fn::fn_b",
            "src/b.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/b.rs::fn::fn_b",
        "src/a.rs::fn::fn_a",
        EdgeKind::Calls,
        "src/b.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .analyze_removal(&["src/a.rs::function::fn_a"], None, None)
        .unwrap();

    assert_eq!(result.seed[0].qualified_name, "src/a.rs::fn::fn_a");
    assert!(
        result
            .impacted_symbols
            .iter()
            .any(|im| im.node.qualified_name == "src/b.rs::fn::fn_b")
    );
}

#[test]
fn removal_containment_only_edges_are_weak_not_probable() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
        node(0, "mod_a", "src/a.rs", "src/a.rs", NodeKind::File),
    ];
    let edges = vec![edge(
        "src/a.rs",
        "src/a.rs::fn_a",
        EdgeKind::Contains,
        "src/a.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .analyze_removal(&["src/a.rs::fn_a"], None, None)
        .unwrap();

    for impacted in &result.impacted_symbols {
        if impacted.node.qualified_name == "src/a.rs" {
            assert_ne!(
                impacted.impact_class,
                ImpactClass::Definite,
                "containment parent must not be Definite impact"
            );
            assert_ne!(
                impacted.impact_class,
                ImpactClass::Probable,
                "containment parent must not be Probable impact — got inflated result"
            );
        }
    }
}

#[test]
fn dead_code_private_function_flagged() {
    let mut store = make_store();
    let mut priv_node = node(
        0,
        "unused_fn",
        "src/a.rs::unused_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    priv_node.modifiers = None;
    seed_graph(&mut store, vec![priv_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let candidates = engine.detect_dead_code(&[], None, None, &[]).unwrap();
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.node.qualified_name == "src/a.rs::unused_fn"),
        "private unused_fn should be dead-code candidate"
    );
}

#[test]
fn dead_code_exclude_kinds_removes_matching_candidates() {
    let mut store = make_store();
    let mut fn_node = node(
        0,
        "unused_fn",
        "src/a.rs::unused_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    fn_node.modifiers = None;
    let mut const_node = node(
        1,
        "UNUSED_CONST",
        "src/a.rs::UNUSED_CONST",
        "src/a.rs",
        NodeKind::Constant,
    );
    const_node.modifiers = None;
    seed_graph(&mut store, vec![fn_node, const_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let candidates = engine
        .detect_dead_code(&[], None, None, &[NodeKind::Constant])
        .unwrap();
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.node.kind == NodeKind::Constant),
        "constants should be filtered out by exclude_kinds"
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.node.kind == NodeKind::Function),
        "functions should still appear when only constants are excluded"
    );
}

#[test]
fn dead_code_exported_function_not_flagged() {
    let mut store = make_store();
    let mut pub_node = node(
        0,
        "pub_fn",
        "src/a.rs::pub_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    pub_node.modifiers = Some("pub".to_owned());
    seed_graph(&mut store, vec![pub_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let candidates = engine.detect_dead_code(&[], None, None, &[]).unwrap();
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.node.qualified_name == "src/a.rs::pub_fn"),
        "pub function should not be flagged"
    );
}

#[test]
fn dead_code_entrypoint_suppressed() {
    let mut store = make_store();
    let main_node = node(
        0,
        "main",
        "src/main.rs::main",
        "src/main.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![main_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let candidates = engine.detect_dead_code(&[], None, None, &[]).unwrap();
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.node.name == "main"),
        "main entrypoint should be suppressed"
    );
}

#[test]
fn rename_same_file_radius() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
        node(
            0,
            "fn_caller",
            "src/a.rs::fn_caller",
            "src/a.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/a.rs::fn_caller",
        "src/a.rs::fn_a",
        EdgeKind::Calls,
        "src/a.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .preview_rename_radius("src/a.rs::fn_a", "fn_a_renamed")
        .unwrap();
    assert!(
        result
            .affected_references
            .iter()
            .any(|reference| reference.scope == ReferenceScope::SameFile),
        "caller in same file should appear as SameFile reference"
    );
}

#[test]
fn rename_cross_module_radius() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "fn_a",
            "module_a/lib.rs::fn_a",
            "module_a/lib.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "fn_b",
            "module_b/lib.rs::fn_b",
            "module_b/lib.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "module_b/lib.rs::fn_b",
        "module_a/lib.rs::fn_a",
        EdgeKind::Calls,
        "module_b/lib.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .preview_rename_radius("module_a/lib.rs::fn_a", "fn_a_v2")
        .unwrap();
    assert!(
        result
            .affected_references
            .iter()
            .any(|reference| reference.scope == ReferenceScope::CrossModule),
        "caller in different module dir should be CrossModule"
    );
}

#[test]
fn rename_radius_normalizes_function_alias_qname() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "fn_a",
            "src/a.rs::fn::fn_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "fn_caller",
            "src/a.rs::fn::fn_caller",
            "src/a.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/a.rs::fn::fn_caller",
        "src/a.rs::fn::fn_a",
        EdgeKind::Calls,
        "src/a.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .preview_rename_radius("src/a.rs::function::fn_a", "fn_a_renamed")
        .unwrap();

    assert_eq!(result.target.qualified_name, "src/a.rs::fn::fn_a");
    assert!(
        result
            .affected_references
            .iter()
            .any(|reference| reference.node.qualified_name == "src/a.rs::fn::fn_caller")
    );
}

#[test]
fn dependency_removal_blocked_by_reference() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "dep_a",
            "src/a.rs::dep_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "consumer",
            "src/b.rs::consumer",
            "src/b.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/b.rs::consumer",
        "src/a.rs::dep_a",
        EdgeKind::Calls,
        "src/b.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine.check_dependency_removal("src/a.rs::dep_a").unwrap();
    assert!(
        !result.removable,
        "dep_a is still referenced — not removable"
    );
    assert!(!result.blocking_references.is_empty());
}

#[test]
fn dependency_removal_normalizes_function_alias_qname() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "dep_a",
            "src/a.rs::fn::dep_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "consumer",
            "src/b.rs::fn::consumer",
            "src/b.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/b.rs::fn::consumer",
        "src/a.rs::fn::dep_a",
        EdgeKind::Calls,
        "src/b.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .check_dependency_removal("src/a.rs::function::dep_a")
        .unwrap();

    assert_eq!(result.target_qname, "src/a.rs::fn::dep_a");
    assert!(!result.blocking_references.is_empty());
}

#[test]
fn test_adjacency_missing_for_changed_symbol() {
    let mut store = make_store();
    let no_test_node = node(
        0,
        "fn_x",
        "src/lib.rs::fn_x",
        "src/lib.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![no_test_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let result = engine.find_test_adjacency("src/lib.rs::fn_x").unwrap();
    assert_eq!(result.coverage_strength, atlas_core::CoverageStrength::None);
    assert!(result.recommendation.is_some());
}

#[test]
fn test_adjacency_normalizes_function_alias_qname() {
    let mut store = make_store();
    let target = node(
        0,
        "fn_x",
        "src/lib.rs::fn::fn_x",
        "src/lib.rs",
        NodeKind::Function,
    );
    let test = node(
        0,
        "fn_x_test",
        "tests/lib.rs::test::fn_x_test",
        "tests/lib.rs",
        NodeKind::Test,
    );
    let edges = vec![edge(
        "tests/lib.rs::test::fn_x_test",
        "src/lib.rs::fn::fn_x",
        EdgeKind::Tests,
        "tests/lib.rs",
    )];
    seed_graph(&mut store, vec![target, test], edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .find_test_adjacency("src/lib.rs::function::fn_x")
        .unwrap();

    assert_eq!(result.symbol.qualified_name, "src/lib.rs::fn::fn_x");
    assert_eq!(
        result.coverage_strength,
        atlas_core::CoverageStrength::Direct
    );
}

#[test]
fn test_adjacency_indirect_through_caller_tests() {
    let mut store = make_store();
    let target = node(
        0,
        "inner",
        "src/lib.rs::fn::inner",
        "src/lib.rs",
        NodeKind::Function,
    );
    let caller = node(
        0,
        "outer",
        "src/lib.rs::fn::outer",
        "src/lib.rs",
        NodeKind::Function,
    );
    let test_fn = node(
        0,
        "test_outer",
        "tests/lib.rs::test::test_outer",
        "tests/lib.rs",
        NodeKind::Test,
    );
    let edges = vec![
        edge(
            "src/lib.rs::fn::outer",
            "src/lib.rs::fn::inner",
            EdgeKind::Calls,
            "src/lib.rs",
        ),
        edge(
            "tests/lib.rs::test::test_outer",
            "src/lib.rs::fn::outer",
            EdgeKind::Tests,
            "tests/lib.rs",
        ),
    ];
    seed_graph(&mut store, vec![target, caller, test_fn], edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine.find_test_adjacency("src/lib.rs::fn::inner").unwrap();
    assert_eq!(
        result.coverage_strength,
        atlas_core::CoverageStrength::IndirectThroughCallers
    );
    assert!(result.recommendation.is_some());
}

#[test]
fn refactor_safety_sanity_checks() {
    let mut store = make_store();
    let solo = node(
        0,
        "solo_fn",
        "src/a.rs::solo_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![solo], vec![]);

    let engine = ReasoningEngine::new(&store);
    let result = engine.score_refactor_safety("src/a.rs::solo_fn").unwrap();
    assert_eq!(result.safety.band, SafetyBand::Safe);
    assert_eq!(result.coverage_strength, atlas_core::CoverageStrength::None);
}

#[test]
fn refactor_safety_normalizes_function_alias_qname() {
    let mut store = make_store();
    let solo = node(
        0,
        "solo_fn",
        "src/a.rs::fn::solo_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![solo], vec![]);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .score_refactor_safety("src/a.rs::function::solo_fn")
        .unwrap();

    assert_eq!(result.node.qualified_name, "src/a.rs::fn::solo_fn");
}

#[test]
fn classify_change_risk_uses_owner_identity_for_cross_package() {
    let mut store = make_store();
    let target = node(
        0,
        "helper",
        "packages/foo/src/lib.rs::fn::helper",
        "packages/foo/src/lib.rs",
        NodeKind::Function,
    );
    let caller = node(
        0,
        "caller",
        "packages/bar/src/lib.rs::fn::caller",
        "packages/bar/src/lib.rs",
        NodeKind::Function,
    );
    let edges = vec![edge(
        "packages/bar/src/lib.rs::fn::caller",
        "packages/foo/src/lib.rs::fn::helper",
        EdgeKind::Calls,
        "packages/bar/src/lib.rs",
    )];
    seed_graph(&mut store, vec![target.clone(), caller], edges);
    attach_owner(&mut store, &target.file_path, "packages/foo/Cargo.toml");
    attach_owner(
        &mut store,
        "packages/bar/src/lib.rs",
        "packages/bar/Cargo.toml",
    );

    let engine = ReasoningEngine::new(&store);
    let result = engine.classify_change_risk(&target.qualified_name).unwrap();

    assert!(
        result
            .contributing_factors
            .iter()
            .any(|factor| factor.contains("cross-package")),
        "expected cross-package factor, got {:?}",
        result.contributing_factors
    );
}

#[test]
fn classify_change_risk_normalizes_function_alias_qname() {
    let mut store = make_store();
    let target = node(
        0,
        "helper",
        "src/lib.rs::fn::helper",
        "src/lib.rs",
        NodeKind::Function,
    );
    let caller = node(
        0,
        "caller",
        "src/other.rs::fn::caller",
        "src/other.rs",
        NodeKind::Function,
    );
    let edges = vec![edge(
        "src/other.rs::fn::caller",
        "src/lib.rs::fn::helper",
        EdgeKind::Calls,
        "src/other.rs",
    )];
    seed_graph(&mut store, vec![target, caller], edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .classify_change_risk("src/lib.rs::function::helper")
        .unwrap();

    assert!(!result.contributing_factors.is_empty());
}
