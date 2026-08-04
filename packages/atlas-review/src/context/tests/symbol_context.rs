use super::*;

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
    let evidence = seed_node
        .context_ranking_evidence
        .as_ref()
        .expect("direct target evidence");
    assert!(evidence.direct_target);
    assert_eq!(evidence.base_score, Some(seed_node.relevance_score));
    assert_eq!(evidence.final_score, Some(seed_node.relevance_score));
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
    let test_node = result
        .nodes
        .iter()
        .find(|n| n.node.qualified_name == "tests/test_a.rs::test_fn_a")
        .expect("test node");
    assert!(
        test_node
            .context_ranking_evidence
            .as_ref()
            .is_some_and(|e| e.test_adjacent),
        "test-adjacent node must record context ranking evidence"
    );
}

#[test]
fn review_context_records_changed_symbol_and_impact_evidence() {
    let mut store = open_store();
    seed_graph(&mut store);

    let req = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: vec!["src/a.rs".to_string()],
        },
        ..ContextRequest::default()
    };

    let result =
        super::build::build_review_context(&store, &req, &BudgetPolicy::default()).unwrap();
    let changed = result
        .nodes
        .iter()
        .find(|node| node.node.qualified_name == "src/a.rs::fn_a")
        .expect("changed symbol in review context");
    let changed_evidence = changed
        .context_ranking_evidence
        .as_ref()
        .expect("changed symbol evidence");
    assert!(changed_evidence.direct_target);
    assert!(changed_evidence.changed_symbol);
    assert!(
        changed_evidence
            .impact_score_contribution
            .unwrap_or_default()
            > 0.0,
        "changed symbol must record impact contribution"
    );
    assert!(
        changed_evidence.final_score.unwrap_or_default()
            >= changed_evidence.base_score.unwrap_or_default(),
        "impact contribution must not decrease final score"
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

#[test]
fn symbol_context_blocks_cross_repo_callers_unless_enabled() {
    let mut store = open_store();
    let local = with_repo(
        make_node(
            "src/local.rs::fn::target",
            "target",
            "src/local.rs",
            NodeKind::Function,
            None,
        ),
        "repo-local",
    );
    let remote = with_repo(
        make_node(
            "vendor/remote.rs::fn::caller",
            "caller",
            "vendor/remote.rs",
            NodeKind::Function,
            None,
        ),
        "repo-remote",
    );
    let files = vec![
        ParsedFile {
            path: "src/local.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "local".to_string(),
            size: None,
            nodes: vec![local.clone()],
            edges: vec![],
        },
        ParsedFile {
            path: "vendor/remote.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "remote".to_string(),
            size: None,
            nodes: vec![remote.clone()],
            edges: vec![make_edge(
                "vendor/remote.rs::fn::caller",
                "src/local.rs::fn::target",
                EdgeKind::Calls,
                "vendor/remote.rs",
            )],
        },
    ];
    store.replace_batch(&files).unwrap();

    let seed = store
        .node_by_qname("src/local.rs::fn::target")
        .unwrap()
        .unwrap();
    let blocked = build_symbol_context(
        &store,
        seed.clone(),
        &symbol_request("src/local.rs::fn::target"),
    )
    .unwrap();
    assert!(
        !blocked
            .nodes
            .iter()
            .any(|node| node.node.qualified_name == "vendor/remote.rs::fn::caller")
    );

    let mut req = symbol_request("src/local.rs::fn::target");
    req.allow_cross_repo_edges = true;
    let allowed = build_symbol_context(&store, seed, &req).unwrap();
    assert!(
        allowed
            .nodes
            .iter()
            .any(|node| node.node.qualified_name == "vendor/remote.rs::fn::caller")
    );
}
