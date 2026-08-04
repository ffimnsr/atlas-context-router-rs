use super::*;

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
    assert_eq!(result.budget.budget_status, BudgetStatus::PartialResult);
    assert_eq!(result.budget.budget_name, "graph_traversal.max_seed_files");
    assert!(result.budget.safe_to_answer);
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
    assert!(meta.budget_hit);
    assert!(!meta.safe_to_answer);
    assert!(!meta.partial);
    assert!(meta.suggested_narrower_query.is_some());
    assert_eq!(result.budget.budget_status, BudgetStatus::Blocked);
    assert_eq!(
        result.budget.budget_name,
        "query_candidates_and_seeds.symbol_resolution"
    );
    assert!(!result.budget.safe_to_answer);
}
