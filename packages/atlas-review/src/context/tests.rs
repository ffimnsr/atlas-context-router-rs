use super::*;
use atlas_core::{
    BudgetLimitRule, BudgetPolicy, BudgetStatus, EdgeKind, NodeId, NodeKind,
    model::{
        ContextIntent, ContextRequest, ContextTarget, Edge, Node, ParsedFile, SelectionReason,
    },
};
use atlas_store_sqlite::Store;

fn open_store() -> Store {
    let mut s = Store::open(":memory:").unwrap();
    s.migrate().unwrap();
    s
}

fn make_node(qname: &str, name: &str, file: &str, kind: NodeKind, parent: Option<&str>) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        file_path: file.to_string(),
        line_start: 1,
        line_end: 10,
        language: "rust".to_string(),
        parent_name: parent.map(String::from),
        params: None,
        return_type: None,
        modifiers: Some("pub".to_string()),
        is_test: false,
        file_hash: "abc".to_string(),
        extra_json: serde_json::Value::Null,
    }
}

fn make_edge(src: &str, tgt: &str, kind: EdgeKind, file: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: src.to_string(),
        target_qn: tgt.to_string(),
        file_path: file.to_string(),
        line: None,
        confidence: 1.0,
        confidence_tier: None,
        extra_json: serde_json::Value::Null,
    }
}

fn seed_graph(store: &mut Store) {
    let nodes = [
        make_node(
            "src/a.rs::fn_a",
            "fn_a",
            "src/a.rs",
            NodeKind::Function,
            None,
        ),
        make_node(
            "src/a.rs::fn_a_helper",
            "fn_a_helper",
            "src/a.rs",
            NodeKind::Function,
            Some("mod_a"),
        ),
        make_node(
            "src/b.rs::fn_b",
            "fn_b",
            "src/b.rs",
            NodeKind::Function,
            None,
        ),
        make_node(
            "src/b.rs::fn_c",
            "fn_c",
            "src/b.rs",
            NodeKind::Function,
            None,
        ),
        make_node(
            "tests/test_a.rs::test_fn_a",
            "test_fn_a",
            "tests/test_a.rs",
            NodeKind::Test,
            None,
        ),
    ];
    let edges = [
        make_edge(
            "src/a.rs::fn_a",
            "src/b.rs::fn_b",
            EdgeKind::Calls,
            "src/a.rs",
        ),
        make_edge(
            "src/b.rs::fn_b",
            "src/b.rs::fn_c",
            EdgeKind::Calls,
            "src/b.rs",
        ),
        make_edge(
            "tests/test_a.rs::test_fn_a",
            "src/a.rs::fn_a",
            EdgeKind::Tests,
            "tests/test_a.rs",
        ),
    ];
    let files: Vec<ParsedFile> = vec![
        ParsedFile {
            path: "src/a.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h1".to_string(),
            size: None,
            nodes: nodes[0..2].to_vec(),
            edges: edges[0..1].to_vec(),
        },
        ParsedFile {
            path: "src/b.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h2".to_string(),
            size: None,
            nodes: nodes[2..4].to_vec(),
            edges: edges[1..2].to_vec(),
        },
        ParsedFile {
            path: "tests/test_a.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h3".to_string(),
            size: None,
            nodes: nodes[4..5].to_vec(),
            edges: edges[2..3].to_vec(),
        },
    ];
    store.replace_batch(&files).unwrap();
}

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

fn symbol_request(qname: &str) -> ContextRequest {
    ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: qname.to_string(),
        },
        include_tests: false,
        include_imports: false,
        include_neighbors: false,
        ..ContextRequest::default()
    }
}

#[test]
fn symbol_context_contains_seed_and_callee() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = symbol_request("src/a.rs::fn_a");
    let result = build_symbol_context(&store, seed, &req).unwrap();

    let qnames: Vec<&str> = result
        .nodes
        .iter()
        .map(|n| n.node.qualified_name.as_str())
        .collect();
    assert!(qnames.contains(&"src/a.rs::fn_a"), "seed missing");
    assert!(qnames.contains(&"src/b.rs::fn_b"), "callee fn_b missing");
}

#[test]
fn symbol_context_seed_is_direct_target() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = symbol_request("src/a.rs::fn_a");
    let result = build_symbol_context(&store, seed, &req).unwrap();

    let seed_node = result
        .nodes
        .iter()
        .find(|n| n.node.qualified_name == "src/a.rs::fn_a")
        .unwrap();
    assert_eq!(seed_node.selection_reason, SelectionReason::DirectTarget);
    assert_eq!(seed_node.distance, 0);
}

#[test]
fn symbol_context_include_tests_flag() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let mut req = symbol_request("src/a.rs::fn_a");
    req.include_tests = true;
    let result = build_symbol_context(&store, seed, &req).unwrap();

    let qnames: Vec<&str> = result
        .nodes
        .iter()
        .map(|n| n.node.qualified_name.as_str())
        .collect();
    assert!(
        qnames.contains(&"tests/test_a.rs::test_fn_a"),
        "test node missing"
    );
}

#[test]
fn symbol_context_files_bounded() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let mut req = symbol_request("src/a.rs::fn_a");
    req.max_files = Some(1);
    let result = build_symbol_context(&store, seed, &req).unwrap();
    assert!(result.files.len() <= 1);
}

#[test]
fn rank_puts_direct_target_first() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = symbol_request("src/a.rs::fn_a");
    let result = build_symbol_context(&store, seed, &req).unwrap();
    assert_eq!(
        result.nodes[0].selection_reason,
        SelectionReason::DirectTarget
    );
}

#[test]
fn callers_and_callees_survive_trimming_over_distant_nodes() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/b.rs::fn_b").unwrap().unwrap();
    let mut req = symbol_request("src/b.rs::fn_b");
    req.max_nodes = Some(2);
    req.include_tests = true;
    let result = build_symbol_context(&store, seed, &req).unwrap();

    assert!(result.nodes.len() <= 2);
    assert!(
        result
            .nodes
            .iter()
            .any(|n| n.selection_reason == SelectionReason::DirectTarget)
    );
    assert!(result.truncation.truncated || result.nodes.len() == 2);
}

#[test]
fn trim_records_dropped_counts() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let mut req = symbol_request("src/a.rs::fn_a");
    req.max_nodes = Some(1);
    let result = build_symbol_context(&store, seed, &req).unwrap();
    assert_eq!(result.nodes.len(), 1);
    for edge in &result.edges {
        let src_present = result
            .nodes
            .iter()
            .any(|n| n.node.qualified_name == edge.edge.source_qn);
        let tgt_present = result
            .nodes
            .iter()
            .any(|n| n.node.qualified_name == edge.edge.target_qn);
        assert!(
            src_present || tgt_present,
            "edge references both-dropped nodes"
        );
    }
}

#[test]
fn trim_caps_deterministic_under_ties() {
    let mut store = open_store();
    seed_graph(&mut store);

    let run = |s: &Store| {
        let seed = s.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let mut req = symbol_request("src/a.rs::fn_a");
        req.max_nodes = Some(2);
        build_symbol_context(s, seed, &req).unwrap()
    };

    let r1 = run(&store);
    let r2 = run(&store);
    let qns1: Vec<&str> = r1
        .nodes
        .iter()
        .map(|n| n.node.qualified_name.as_str())
        .collect();
    let qns2: Vec<&str> = r2
        .nodes
        .iter()
        .map(|n| n.node.qualified_name.as_str())
        .collect();
    assert_eq!(qns1, qns2, "trim output non-deterministic");
}

#[test]
fn context_engine_clamps_oversized_request_limits() {
    let mut store = open_store();
    seed_graph(&mut store);

    let mut req = symbol_request("src/a.rs::fn_a");
    req.max_nodes = Some(10_000);
    req.max_edges = Some(10_000);
    req.max_files = Some(10_000);
    req.depth = Some(99);

    let result = ContextEngine::new(&store).build(&req).unwrap();

    assert_eq!(result.request.max_nodes, Some(200));
    assert_eq!(result.request.max_edges, Some(400));
    assert_eq!(result.request.max_files, Some(100));
    assert_eq!(result.request.depth, Some(10));
    assert_eq!(result.budget.budget_status, BudgetStatus::OverrideClamped);
    assert!(result.budget.budget_hit);
}

#[test]
fn build_context_convenience_wrapper() {
    let mut store = open_store();
    seed_graph(&mut store);
    let req = ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: "src/b.rs::fn_b".to_string(),
        },
        ..ContextRequest::default()
    };
    let result = build_context(&store, &req, &BudgetPolicy::default()).unwrap();
    assert!(
        result
            .nodes
            .iter()
            .any(|n| n.node.qualified_name == "src/b.rs::fn_b")
    );
}

fn review_request(paths: Vec<String>) -> ContextRequest {
    ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles { paths },
        ..ContextRequest::default()
    }
}

#[test]
fn review_context_changed_files_become_direct_targets() {
    let mut store = open_store();
    seed_graph(&mut store);
    let req = review_request(vec!["src/a.rs".to_string()]);
    let result = build_context(&store, &req, &BudgetPolicy::default()).unwrap();

    assert!(
        result
            .nodes
            .iter()
            .filter(|n| n.node.file_path == "src/a.rs")
            .all(|n| n.selection_reason == SelectionReason::DirectTarget),
        "src/a.rs nodes not tagged DirectTarget"
    );
}

#[test]
fn review_context_impacted_nodes_tagged_impact_neighbor() {
    let mut store = open_store();
    seed_graph(&mut store);
    let req = review_request(vec!["src/a.rs".to_string()]);
    let result = build_context(&store, &req, &BudgetPolicy::default()).unwrap();

    let has_neighbor = result
        .nodes
        .iter()
        .any(|n| n.selection_reason == SelectionReason::ImpactNeighbor);
    assert!(
        has_neighbor,
        "expected ImpactNeighbor nodes from impact traversal"
    );
}

#[test]
fn review_context_result_is_bounded() {
    let mut store = open_store();
    seed_graph(&mut store);
    let mut req = review_request(vec!["src/a.rs".to_string()]);
    req.max_nodes = Some(3);
    let result = build_context(&store, &req, &BudgetPolicy::default()).unwrap();
    assert!(result.nodes.len() <= 3, "node cap exceeded");
}

#[test]
fn review_context_tight_cap_keeps_impacted_neighbor() {
    let mut store = open_store();
    seed_graph(&mut store);
    let mut req = review_request(vec!["src/a.rs".to_string()]);
    req.max_nodes = Some(2);
    let result = build_context(&store, &req, &BudgetPolicy::default()).unwrap();

    assert_eq!(result.nodes.len(), 2);
    assert!(
        result
            .nodes
            .iter()
            .any(|node| node.selection_reason == SelectionReason::DirectTarget)
    );
    assert!(
        result
            .nodes
            .iter()
            .any(|node| node.selection_reason == SelectionReason::ImpactNeighbor),
        "expected impacted neighbor to survive tight review cap"
    );
}

#[test]
fn impact_context_file_seed_returns_neighbors() {
    let mut store = open_store();
    seed_graph(&mut store);
    let req = ContextRequest {
        intent: ContextIntent::Impact,
        target: ContextTarget::FilePath {
            path: "src/a.rs".to_string(),
        },
        ..ContextRequest::default()
    };
    let result = build_context(&store, &req, &BudgetPolicy::default()).unwrap();
    assert!(!result.nodes.is_empty(), "impact result must have nodes");
}

#[test]
fn impact_context_qname_seed_returns_neighbors() {
    let mut store = open_store();
    seed_graph(&mut store);
    let req = ContextRequest {
        intent: ContextIntent::Impact,
        target: ContextTarget::QualifiedName {
            qname: "src/a.rs::fn_a".to_string(),
        },
        ..ContextRequest::default()
    };
    let result = build_context(&store, &req, &BudgetPolicy::default()).unwrap();
    let has_fn_b = result
        .nodes
        .iter()
        .any(|n| n.node.qualified_name == "src/b.rs::fn_b");
    assert!(has_fn_b, "fn_b should appear as impact neighbor of fn_a");
}

#[test]
fn impact_context_missing_qname_returns_empty() {
    let mut store = open_store();
    seed_graph(&mut store);
    let req = ContextRequest {
        intent: ContextIntent::Impact,
        target: ContextTarget::QualifiedName {
            qname: "no::such::symbol".to_string(),
        },
        ..ContextRequest::default()
    };
    let result = build_context(&store, &req, &BudgetPolicy::default()).unwrap();
    assert!(
        result.nodes.is_empty(),
        "missing symbol should yield empty result"
    );
}

#[test]
fn review_context_reports_explicit_file_seed_truncation() {
    let mut store = open_store();
    seed_graph(&mut store);

    let mut policy = BudgetPolicy::default();
    policy.graph_traversal.seed_files =
        BudgetLimitRule::new(1, 1, atlas_core::BudgetHitBehavior::Partial, true);

    let req = review_request(vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
    let result = ContextEngine::new(&store)
        .with_budget_policy(policy)
        .build(&req)
        .unwrap();

    assert_eq!(result.seed_budgets.len(), 1);
    let meta = &result.seed_budgets[0];
    assert_eq!(meta.seed_kind, "changed_files");
    assert_eq!(meta.requested_seed_count, 2);
    assert_eq!(meta.accepted_seed_count, 1);
    assert_eq!(meta.omitted_seed_count, 1);
    assert!(meta.budget_hit);
    assert!(meta.partial);
    assert!(meta.safe_to_answer);
    assert!(meta.suggested_narrower_query.is_some());
}

#[test]
fn impact_context_fails_closed_for_ambiguous_symbol_seed() {
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

    let req = ContextRequest {
        intent: ContextIntent::Impact,
        target: ContextTarget::SymbolName {
            name: "fn_a".to_string(),
        },
        ..ContextRequest::default()
    };

    let result = ContextEngine::new(&store).build(&req).unwrap();

    assert!(result.nodes.is_empty());
    assert!(result.ambiguity.is_some());
    assert_eq!(result.seed_budgets.len(), 1);
    let meta = &result.seed_budgets[0];
    assert_eq!(meta.seed_kind, "symbol_resolution");
    assert_eq!(meta.requested_seed_count, 0);
    assert_eq!(meta.accepted_seed_count, 0);
    assert!(!meta.safe_to_answer);
    assert!(!meta.partial);
    assert!(meta.suggested_narrower_query.is_some());
}

#[test]
fn code_spans_populated_for_target() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let mut req = symbol_request("src/a.rs::fn_a");
    req.include_code_spans = true;
    let result = build_symbol_context(&store, seed, &req).unwrap();

    let target_file = result
        .files
        .iter()
        .find(|f| f.path == "src/a.rs")
        .expect("src/a.rs must be in files");
    assert!(
        !target_file.line_ranges.is_empty(),
        "target file must have line ranges"
    );
}

#[test]
fn code_spans_not_populated_when_disabled() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let mut req = symbol_request("src/a.rs::fn_a");
    req.include_code_spans = false;
    let result = build_symbol_context(&store, seed, &req).unwrap();

    for sf in &result.files {
        assert!(
            sf.line_ranges.is_empty(),
            "line_ranges should be empty when spans disabled"
        );
    }
}

#[test]
fn code_spans_merge_overlapping_ranges() {
    let spans = vec![(1u32, 5u32), (3, 8), (15, 20)];
    let merged = super::spans::merge_spans(&spans);
    assert_eq!(merged, vec![(1, 8), (15, 20)]);
}

#[test]
fn code_spans_merge_adjacent_ranges() {
    let spans = vec![(1u32, 5u32), (6, 10)];
    let merged = super::spans::merge_spans(&spans);
    assert_eq!(merged, vec![(1, 10)]);
}

#[test]
fn code_spans_single_range_unchanged() {
    let spans = vec![(10u32, 20u32)];
    let merged = super::spans::merge_spans(&spans);
    assert_eq!(merged, vec![(10, 20)]);
}
