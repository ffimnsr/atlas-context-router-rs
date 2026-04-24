use super::*;
use crate::budget::BudgetReport;
use crate::kinds::{EdgeKind, NodeKind};

// -------------------------------------------------------------------------
// NodeId
// -------------------------------------------------------------------------

#[test]
fn node_id_serde_round_trip() {
    let id = NodeId(42);
    let json = serde_json::to_string(&id).unwrap();
    // transparent: serialises as a plain number
    assert_eq!(json, "42");
    let back: NodeId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, id);
}

#[test]
fn node_id_unset_sentinel() {
    assert_eq!(NodeId::UNSET.0, 0);
}

#[test]
fn node_id_from_i64() {
    assert_eq!(NodeId::from(7_i64), NodeId(7));
    let raw: i64 = NodeId(99).into();
    assert_eq!(raw, 99);
}

// -------------------------------------------------------------------------
// Node serialization
// -------------------------------------------------------------------------

fn sample_node() -> Node {
    Node {
        id: NodeId(1),
        kind: NodeKind::Function,
        name: "my_func".to_string(),
        qualified_name: "src/lib.rs::fn::my_func".to_string(),
        file_path: "src/lib.rs".to_string(),
        line_start: 10,
        line_end: 20,
        language: "rust".to_string(),
        parent_name: None,
        params: Some("(x: i32)".to_string()),
        return_type: Some("i32".to_string()),
        modifiers: None,
        is_test: false,
        file_hash: "abc123".to_string(),
        extra_json: serde_json::Value::Null,
    }
}

#[test]
fn node_serde_round_trip() {
    let n = sample_node();
    let json = serde_json::to_string(&n).unwrap();
    let back: Node = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, n.id);
    assert_eq!(back.kind, n.kind);
    assert_eq!(back.name, n.name);
    assert_eq!(back.qualified_name, n.qualified_name);
    assert_eq!(back.file_path, n.file_path);
    assert_eq!(back.line_start, n.line_start);
    assert_eq!(back.line_end, n.line_end);
    assert_eq!(back.language, n.language);
    assert_eq!(back.params, n.params);
    assert_eq!(back.return_type, n.return_type);
    assert_eq!(back.is_test, n.is_test);
    assert_eq!(back.file_hash, n.file_hash);
}

#[test]
fn node_optional_fields_null_round_trip() {
    let mut n = sample_node();
    n.parent_name = None;
    n.params = None;
    n.return_type = None;
    n.modifiers = None;
    let json = serde_json::to_string(&n).unwrap();
    let back: Node = serde_json::from_str(&json).unwrap();
    assert!(back.parent_name.is_none());
    assert!(back.params.is_none());
    assert!(back.return_type.is_none());
    assert!(back.modifiers.is_none());
}

#[test]
fn node_is_test_flag_preserved() {
    let mut n = sample_node();
    n.is_test = true;
    n.kind = NodeKind::Test;
    let json = serde_json::to_string(&n).unwrap();
    let back: Node = serde_json::from_str(&json).unwrap();
    assert!(back.is_test);
    assert_eq!(back.kind, NodeKind::Test);
}

// -------------------------------------------------------------------------
// Edge serialization
// -------------------------------------------------------------------------

fn sample_edge() -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: "src/a.rs::fn::caller".to_string(),
        target_qn: "src/b.rs::fn::callee".to_string(),
        file_path: "src/a.rs".to_string(),
        line: Some(15),
        confidence: 1.0,
        confidence_tier: Some("high".to_string()),
        extra_json: serde_json::Value::Null,
    }
}

#[test]
fn edge_serde_round_trip() {
    let e = sample_edge();
    let json = serde_json::to_string(&e).unwrap();
    let back: Edge = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, e.id);
    assert_eq!(back.kind, e.kind);
    assert_eq!(back.source_qn, e.source_qn);
    assert_eq!(back.target_qn, e.target_qn);
    assert_eq!(back.file_path, e.file_path);
    assert_eq!(back.line, e.line);
    assert_eq!(back.confidence, e.confidence);
    assert_eq!(back.confidence_tier, e.confidence_tier);
}

#[test]
fn edge_optional_line_none_round_trip() {
    let mut e = sample_edge();
    e.line = None;
    let json = serde_json::to_string(&e).unwrap();
    let back: Edge = serde_json::from_str(&json).unwrap();
    assert!(back.line.is_none());
}

#[test]
fn edge_optional_confidence_tier_none_round_trip() {
    let mut e = sample_edge();
    e.confidence_tier = None;
    let json = serde_json::to_string(&e).unwrap();
    let back: Edge = serde_json::from_str(&json).unwrap();
    assert!(back.confidence_tier.is_none());
}

// -------------------------------------------------------------------------
// Phase 22 Slice 1 — ContextRequest / ContextResult serde round-trips
// -------------------------------------------------------------------------

fn sample_context_request_symbol() -> ContextRequest {
    ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: "crate::module::my_fn".to_string(),
        },
        max_nodes: Some(50),
        max_edges: Some(100),
        max_files: Some(10),
        depth: Some(1),
        include_tests: true,
        include_imports: true,
        include_neighbors: false,
        include_code_spans: false,
        include_callers: true,
        include_callees: true,
        include_saved_context: false,
        session_id: None,
        token_budget: None,
    }
}

#[test]
fn context_intent_serde_variants() {
    for (intent, expected) in [
        (ContextIntent::Symbol, "\"symbol\""),
        (ContextIntent::File, "\"file\""),
        (ContextIntent::Review, "\"review\""),
        (ContextIntent::Impact, "\"impact\""),
    ] {
        let json = serde_json::to_string(&intent).unwrap();
        assert_eq!(json, expected);
        let back: ContextIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, intent);
    }
}

#[test]
fn context_target_qualified_name_round_trip() {
    let t = ContextTarget::QualifiedName {
        qname: "crate::foo::bar".to_string(),
    };
    let json = serde_json::to_string(&t).unwrap();
    let back: ContextTarget = serde_json::from_str(&json).unwrap();
    assert_eq!(back, t);
    assert!(json.contains("\"kind\":\"qualified_name\""));
}

#[test]
fn context_target_symbol_name_round_trip() {
    let t = ContextTarget::SymbolName {
        name: "my_func".to_string(),
    };
    let json = serde_json::to_string(&t).unwrap();
    let back: ContextTarget = serde_json::from_str(&json).unwrap();
    assert_eq!(back, t);
    assert!(json.contains("\"kind\":\"symbol_name\""));
}

#[test]
fn context_target_file_path_round_trip() {
    let t = ContextTarget::FilePath {
        path: "src/lib.rs".to_string(),
    };
    let json = serde_json::to_string(&t).unwrap();
    let back: ContextTarget = serde_json::from_str(&json).unwrap();
    assert_eq!(back, t);
    assert!(json.contains("\"kind\":\"file_path\""));
}

#[test]
fn context_target_changed_files_round_trip() {
    let t = ContextTarget::ChangedFiles {
        paths: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
    };
    let json = serde_json::to_string(&t).unwrap();
    let back: ContextTarget = serde_json::from_str(&json).unwrap();
    assert_eq!(back, t);
    assert!(json.contains("\"kind\":\"changed_files\""));
}

#[test]
fn context_request_round_trip() {
    let req = sample_context_request_symbol();
    let json = serde_json::to_string(&req).unwrap();
    let back: ContextRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.intent, req.intent);
    assert_eq!(back.target, req.target);
    assert_eq!(back.max_nodes, req.max_nodes);
    assert_eq!(back.max_edges, req.max_edges);
    assert_eq!(back.max_files, req.max_files);
    assert_eq!(back.depth, req.depth);
    assert_eq!(back.include_tests, req.include_tests);
    assert_eq!(back.include_imports, req.include_imports);
    assert_eq!(back.include_neighbors, req.include_neighbors);
}

#[test]
fn context_request_default_round_trip() {
    let req = ContextRequest::default();
    let json = serde_json::to_string(&req).unwrap();
    let back: ContextRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.intent, ContextIntent::Symbol);
    assert!(back.max_nodes.is_none());
    assert!(back.depth.is_none());
    assert!(!back.include_tests);
    assert!(back.include_imports);
    assert!(!back.include_neighbors);
}

#[test]
fn selection_reason_serde_variants() {
    let reasons = [
        (SelectionReason::DirectTarget, "\"direct_target\""),
        (SelectionReason::Caller, "\"caller\""),
        (SelectionReason::Callee, "\"callee\""),
        (SelectionReason::Importer, "\"importer\""),
        (SelectionReason::Importee, "\"importee\""),
        (
            SelectionReason::ContainmentSibling,
            "\"containment_sibling\"",
        ),
        (SelectionReason::TestAdjacent, "\"test_adjacent\""),
        (SelectionReason::ImpactNeighbor, "\"impact_neighbor\""),
    ];
    for (reason, expected) in reasons {
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, expected);
        let back: SelectionReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, reason);
    }
}

#[test]
fn selected_node_round_trip() {
    let sn = SelectedNode {
        node: sample_node(),
        selection_reason: SelectionReason::Caller,
        distance: 1,
        relevance_score: 0.0,
    };
    let json = serde_json::to_string(&sn).unwrap();
    let back: SelectedNode = serde_json::from_str(&json).unwrap();
    assert_eq!(back.selection_reason, sn.selection_reason);
    assert_eq!(back.distance, sn.distance);
    assert_eq!(back.node.qualified_name, sn.node.qualified_name);
}

#[test]
fn selected_edge_round_trip() {
    let se = SelectedEdge {
        edge: sample_edge(),
        selection_reason: SelectionReason::Callee,
        depth: None,
        relevance_score: 0.0,
    };
    let json = serde_json::to_string(&se).unwrap();
    let back: SelectedEdge = serde_json::from_str(&json).unwrap();
    assert_eq!(back.selection_reason, se.selection_reason);
    assert_eq!(back.edge.source_qn, se.edge.source_qn);
}

#[test]
fn selected_file_round_trip() {
    let sf = SelectedFile {
        path: "src/main.rs".to_string(),
        selection_reason: SelectionReason::DirectTarget,
        line_ranges: vec![(10, 20), (35, 50)],
        language: Some("rust".to_string()),
        node_count_included: 2,
    };
    let json = serde_json::to_string(&sf).unwrap();
    let back: SelectedFile = serde_json::from_str(&json).unwrap();
    assert_eq!(back.path, sf.path);
    assert_eq!(back.selection_reason, sf.selection_reason);
    assert_eq!(back.line_ranges, sf.line_ranges);
}

#[test]
fn selected_file_empty_ranges_round_trip() {
    let sf = SelectedFile {
        path: "src/lib.rs".to_string(),
        selection_reason: SelectionReason::ImpactNeighbor,
        line_ranges: vec![],
        language: None,
        node_count_included: 0,
    };
    let json = serde_json::to_string(&sf).unwrap();
    let back: SelectedFile = serde_json::from_str(&json).unwrap();
    assert!(back.line_ranges.is_empty());
}

#[test]
fn truncation_meta_none_round_trip() {
    let tm = TruncationMeta::none();
    let json = serde_json::to_string(&tm).unwrap();
    let back: TruncationMeta = serde_json::from_str(&json).unwrap();
    assert!(!back.truncated);
    assert_eq!(back.nodes_dropped, 0);
    assert_eq!(back.edges_dropped, 0);
    assert_eq!(back.files_dropped, 0);
}

#[test]
fn truncation_meta_with_drops_round_trip() {
    let tm = TruncationMeta {
        nodes_dropped: 5,
        edges_dropped: 3,
        files_dropped: 1,
        truncated: true,
        payload: None,
    };
    let json = serde_json::to_string(&tm).unwrap();
    let back: TruncationMeta = serde_json::from_str(&json).unwrap();
    assert!(back.truncated);
    assert_eq!(back.nodes_dropped, 5);
    assert_eq!(back.edges_dropped, 3);
    assert_eq!(back.files_dropped, 1);
}

#[test]
fn ambiguity_meta_round_trip() {
    let am = AmbiguityMeta {
        query: "my_fn".to_string(),
        candidates: vec!["crate::a::my_fn".to_string(), "crate::b::my_fn".to_string()],
        resolved: false,
    };
    let json = serde_json::to_string(&am).unwrap();
    let back: AmbiguityMeta = serde_json::from_str(&json).unwrap();
    assert_eq!(back.query, am.query);
    assert_eq!(back.candidates, am.candidates);
    assert!(!back.resolved);
}

#[test]
fn ambiguity_meta_resolved_round_trip() {
    let am = AmbiguityMeta {
        query: "my_fn".to_string(),
        candidates: vec![],
        resolved: true,
    };
    let json = serde_json::to_string(&am).unwrap();
    let back: AmbiguityMeta = serde_json::from_str(&json).unwrap();
    assert!(back.resolved);
    assert!(back.candidates.is_empty());
}

#[test]
fn context_result_round_trip() {
    let result = ContextResult {
        request: sample_context_request_symbol(),
        nodes: vec![SelectedNode {
            node: sample_node(),
            selection_reason: SelectionReason::DirectTarget,
            distance: 0,
            relevance_score: 0.0,
        }],
        edges: vec![SelectedEdge {
            edge: sample_edge(),
            selection_reason: SelectionReason::Caller,
            depth: None,
            relevance_score: 0.0,
        }],
        files: vec![SelectedFile {
            path: "src/lib.rs".to_string(),
            selection_reason: SelectionReason::DirectTarget,
            line_ranges: vec![(10, 20)],
            language: Some("rust".to_string()),
            node_count_included: 1,
        }],
        truncation: TruncationMeta::none(),
        seed_budgets: vec![],
        traversal_budget: None,
        ambiguity: None,
        workflow: Some(WorkflowSummary {
            headline: Some("Focus on helper callers".to_string()),
            high_impact_nodes: vec![WorkflowFocusNode {
                qualified_name: "src/lib.rs::fn::helper".to_string(),
                kind: "function".to_string(),
                file_path: "src/lib.rs".to_string(),
                relevance_score: 42.0,
                selection_reason: "direct_target".to_string(),
            }],
            impacted_components: vec![WorkflowComponent {
                label: "src".to_string(),
                kind: "directory".to_string(),
                changed_node_count: 1,
                impacted_node_count: 1,
                file_count: 1,
                summary: "1 changed, 1 impacted".to_string(),
            }],
            call_chains: vec![WorkflowCallChain {
                summary: "caller -> helper".to_string(),
                steps: vec![
                    "src/main.rs::fn::caller".to_string(),
                    "src/lib.rs::fn::helper".to_string(),
                ],
                edge_kinds: vec!["calls".to_string()],
            }],
            ripple_effects: vec!["Change reaches one dependent component.".to_string()],
            noise_reduction: NoiseReductionSummary {
                retained_nodes: 1,
                retained_edges: 1,
                retained_files: 1,
                dropped_nodes: 0,
                dropped_edges: 0,
                dropped_files: 0,
                rules_applied: vec!["omitted containment siblings".to_string()],
            },
        }),
        saved_context_sources: vec![],
        budget: BudgetReport::within_budget("review_context_extraction.max_nodes", 50, 1),
    };
    let json = serde_json::to_string(&result).unwrap();
    let back: ContextResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.nodes.len(), 1);
    assert_eq!(back.edges.len(), 1);
    assert_eq!(back.files.len(), 1);
    assert!(back.ambiguity.is_none());
    assert!(!back.truncation.truncated);
    assert!(back.workflow.is_some());
}

#[test]
fn context_result_with_ambiguity_round_trip() {
    let result = ContextResult {
        request: ContextRequest {
            intent: ContextIntent::Symbol,
            target: ContextTarget::SymbolName {
                name: "parse".to_string(),
            },
            ..ContextRequest::default()
        },
        nodes: vec![],
        edges: vec![],
        files: vec![],
        truncation: TruncationMeta::none(),
        seed_budgets: vec![],
        traversal_budget: None,
        ambiguity: Some(AmbiguityMeta {
            query: "parse".to_string(),
            candidates: vec!["crate::a::parse".to_string(), "crate::b::parse".to_string()],
            resolved: false,
        }),
        workflow: None,
        saved_context_sources: vec![],
        budget: BudgetReport::within_budget("review_context_extraction.max_nodes", 50, 0),
    };
    let json = serde_json::to_string(&result).unwrap();
    let back: ContextResult = serde_json::from_str(&json).unwrap();
    let amb = back.ambiguity.unwrap();
    assert_eq!(amb.candidates.len(), 2);
    assert!(!amb.resolved);
}
