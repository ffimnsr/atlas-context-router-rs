use super::*;
use crate::context::payload::trim_one_payload_unit;

fn parsed_file(path: &str, hash: &str, nodes: Vec<Node>, edges: Vec<Edge>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: Some("rust".to_string()),
        hash: hash.to_string(),
        size: None,
        nodes,
        edges,
    }
}

fn selected(
    qname: &str,
    file: &str,
    reason: SelectionReason,
    distance: u32,
    score: f32,
) -> SelectedNode {
    SelectedNode {
        node: make_node(
            qname,
            qname.rsplit("::").next().unwrap_or(qname),
            file,
            NodeKind::Function,
            None,
        ),
        selection_reason: reason,
        distance,
        relevance_score: score,
        context_ranking_evidence: None,
    }
}

fn empty_result(request: ContextRequest, nodes: Vec<SelectedNode>) -> ContextResult {
    ContextResult {
        request,
        nodes,
        edges: Vec::new(),
        files: Vec::new(),
        truncation: TruncationMeta::none(),
        seed_budgets: Vec::new(),
        traversal_budget: None,
        ambiguity: None,
        workflow: None,
        saved_context_sources: Vec::new(),
        content_assets: Vec::new(),
        overviews: Vec::new(),
        sufficiency: None,
        repo_provenance: None,
        telemetry: None,
        budget: atlas_core::BudgetReport::not_applicable(),
    }
}

/// A caller is impacted by the target; a callee is only a dependency. Under
/// payload pressure the caller must survive even when the callee scores higher.
#[test]
fn payload_trim_keeps_caller_before_higher_scored_callee() {
    let request = ContextRequest {
        intent: ContextIntent::Symbol,
        ..ContextRequest::default()
    };
    let mut result = empty_result(
        request,
        vec![
            selected(
                "src/a.rs::fn::seed",
                "src/a.rs",
                SelectionReason::DirectTarget,
                0,
                100.0,
            ),
            selected(
                "src/a.rs::fn::caller",
                "src/a.rs",
                SelectionReason::Caller,
                1,
                10.0,
            ),
            selected(
                "src/a.rs::fn::callee",
                "src/a.rs",
                SelectionReason::Callee,
                1,
                99.0,
            ),
        ],
    );

    assert!(trim_one_payload_unit(&mut result), "a unit must be dropped");

    let remaining = result
        .nodes
        .iter()
        .map(|node| node.node.qualified_name.as_str())
        .collect::<Vec<_>>();
    assert!(
        remaining.contains(&"src/a.rs::fn::caller"),
        "caller must outrank the callee under payload pressure, got {remaining:?}"
    );
    assert!(
        !remaining.contains(&"src/a.rs::fn::callee"),
        "callee must be dropped first, got {remaining:?}"
    );
}

/// File targets used to spend the whole node budget on same-file symbols and
/// then pull in distant 2-hop references from unrelated packages.
#[test]
fn file_target_reserves_neighbor_slot_and_stops_at_one_hop() {
    let mut store = open_store();
    let files = vec![
        parsed_file(
            "src/a.rs",
            "ha",
            vec![
                make_node(
                    "src/a.rs::fn::seed_one",
                    "seed_one",
                    "src/a.rs",
                    NodeKind::Function,
                    None,
                ),
                make_node(
                    "src/a.rs::fn::seed_two",
                    "seed_two",
                    "src/a.rs",
                    NodeKind::Function,
                    None,
                ),
            ],
            vec![],
        ),
        parsed_file(
            "src/b.rs",
            "hb",
            vec![make_node(
                "src/b.rs::fn::caller",
                "caller",
                "src/b.rs",
                NodeKind::Function,
                None,
            )],
            vec![make_edge(
                "src/b.rs::fn::caller",
                "src/a.rs::fn::seed_one",
                EdgeKind::Calls,
                "src/b.rs",
            )],
        ),
        parsed_file(
            "src/c.rs",
            "hc",
            vec![make_node(
                "src/c.rs::fn::far",
                "far",
                "src/c.rs",
                NodeKind::Function,
                None,
            )],
            vec![make_edge(
                "src/c.rs::fn::far",
                "src/b.rs::fn::caller",
                EdgeKind::Calls,
                "src/c.rs",
            )],
        ),
    ];
    store
        .replace_files_transactional_for_repo("repo_test", &files)
        .unwrap();

    let request = ContextRequest {
        intent: ContextIntent::File,
        target: ContextTarget::FilePath {
            path: "src/a.rs".to_string(),
        },
        max_nodes: Some(2),
        include_tests: false,
        include_imports: false,
        include_neighbors: false,
        ..ContextRequest::default()
    };
    let result = build_context(&store, &request, &BudgetPolicy::default()).unwrap();
    let qnames = result
        .nodes
        .iter()
        .map(|node| node.node.qualified_name.as_str())
        .collect::<Vec<_>>();

    assert!(
        qnames.contains(&"src/b.rs::fn::caller"),
        "direct caller must get the reserved neighbour slot, got {qnames:?}"
    );
    assert!(
        !qnames.contains(&"src/c.rs::fn::far"),
        "two-hop reference must stay out of a default file context, got {qnames:?}"
    );
}
