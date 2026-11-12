use super::*;

use crate::store::{TraceDirection, TraceOptions};

fn node(name: &str, qn: &str, file: &str, kind: NodeKind) -> Node {
    make_node(kind, name, qn, file, "rust")
}

/// a -> b -> c, d -> b, plus a reference edge and an unresolved call target.
fn setup_chain(store: &mut Store) {
    let a = node("a", "a.rs::fn::a", "a.rs", NodeKind::Function);
    let b = node("b", "b.rs::fn::b", "b.rs", NodeKind::Function);
    let c = node("c", "c.rs::fn::c", "c.rs", NodeKind::Function);
    let d = node("d", "d.rs::fn::d", "d.rs", NodeKind::Function);
    let ty = node("TypeA", "t.rs::struct::TypeA", "t.rs", NodeKind::Struct);

    let ab = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let bc = make_edge(EdgeKind::Calls, "b.rs::fn::b", "c.rs::fn::c", "b.rs");
    let db = make_edge(EdgeKind::Calls, "d.rs::fn::d", "b.rs::fn::b", "d.rs");
    let reference = make_edge(
        EdgeKind::References,
        "a.rs::fn::a",
        "t.rs::struct::TypeA",
        "a.rs",
    );
    // Parsers record calls to expression targets that never become nodes.
    let unresolved = make_edge(EdgeKind::Calls, "a.rs::fn::a", "HashMap::new", "a.rs");

    for (file, hash, edges, node_slice) in [
        (
            "a.rs",
            "ha",
            vec![ab, reference, unresolved],
            vec![a.clone()],
        ),
        ("b.rs", "hb", vec![bc], vec![b.clone()]),
        ("c.rs", "hc", vec![], vec![c.clone()]),
        ("d.rs", "hd", vec![db], vec![d.clone()]),
        ("t.rs", "ht", vec![], vec![ty.clone()]),
    ] {
        store
            .replace_file_graph_for_repo(
                "repo_test",
                file,
                hash,
                Some("rust"),
                None,
                &node_slice,
                &edges,
            )
            .unwrap();
    }
}

fn calls() -> [EdgeKind; 1] {
    [EdgeKind::Calls]
}

fn trace(
    store: &Store,
    root: &str,
    direction: TraceDirection,
    kinds: &[EdgeKind],
    depth: u32,
    nodes: usize,
) -> TraceResult {
    let opts = TraceOptions {
        direction,
        edge_kinds: kinds,
        max_depth: depth,
        max_nodes: nodes,
    };
    store
        .trace_calls(root, &opts)
        .unwrap()
        .expect("root symbol must exist")
}

fn qns(result: &TraceResult) -> Vec<&str> {
    result
        .nodes
        .iter()
        .map(|node| node.node.qualified_name.as_str())
        .collect()
}

#[test]
fn trace_down_follows_callees_only() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let result = trace(&store, "a.rs::fn::a", TraceDirection::Down, &kinds, 3, 50);
    assert_eq!(
        qns(&result),
        vec!["a.rs::fn::a", "b.rs::fn::b", "c.rs::fn::c"]
    );
    assert_eq!(result.max_depth_reached, 2);
    assert!(!result.truncated);
}

#[test]
fn trace_up_follows_callers_only() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let result = trace(&store, "c.rs::fn::c", TraceDirection::Up, &kinds, 3, 50);
    assert_eq!(
        qns(&result),
        vec!["c.rs::fn::c", "b.rs::fn::b", "a.rs::fn::a", "d.rs::fn::d"]
    );
}

#[test]
fn trace_both_mixes_directions_one_hop() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let result = trace(&store, "b.rs::fn::b", TraceDirection::Both, &kinds, 1, 50);
    let names = qns(&result);
    assert!(names.contains(&"a.rs::fn::a"));
    assert!(names.contains(&"c.rs::fn::c"));
    assert!(names.contains(&"d.rs::fn::d"));
    assert_eq!(names.len(), 4);
}

#[test]
fn trace_respects_edge_kind_filter() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = [EdgeKind::References];
    let result = trace(&store, "a.rs::fn::a", TraceDirection::Down, &kinds, 3, 50);
    assert_eq!(qns(&result), vec!["a.rs::fn::a", "t.rs::struct::TypeA"]);
}

#[test]
fn trace_drops_unresolved_expression_targets() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let result = trace(&store, "a.rs::fn::a", TraceDirection::Down, &kinds, 3, 50);
    assert!(
        result
            .edges
            .iter()
            .all(|edge| edge.target_qn != "HashMap::new"),
        "expression-level call targets must not appear in a trace"
    );
    assert!(
        result
            .nodes
            .iter()
            .all(|node| node.node.qualified_name != "HashMap::new")
    );
}

#[test]
fn trace_honors_depth_cap() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let result = trace(&store, "a.rs::fn::a", TraceDirection::Down, &kinds, 1, 50);
    assert_eq!(qns(&result), vec!["a.rs::fn::a", "b.rs::fn::b"]);
}

#[test]
fn trace_reports_node_budget_truncation() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let result = trace(&store, "a.rs::fn::a", TraceDirection::Down, &kinds, 3, 2);
    assert_eq!(result.nodes.len(), 2);
    assert!(result.truncated);
}

#[test]
fn trace_missing_root_returns_none() {
    let store = open_in_memory();
    let kinds = calls();
    let opts = TraceOptions::new(TraceDirection::Down, &kinds);
    assert!(
        store
            .trace_calls("missing.rs::fn::nope", &opts)
            .unwrap()
            .is_none()
    );
}

#[test]
fn trace_path_finds_downstream_shortest_path() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let opts = TraceOptions {
        direction: TraceDirection::Down,
        edge_kinds: &kinds,
        max_depth: 3,
        max_nodes: 50,
    };
    let result = store
        .trace_call_path("a.rs::fn::a", "c.rs::fn::c", &opts)
        .unwrap()
        .expect("both endpoints exist");
    let path = result.path.expect("path must exist");
    assert_eq!(
        path.nodes
            .iter()
            .map(|node| node.qualified_name.as_str())
            .collect::<Vec<_>>(),
        vec!["a.rs::fn::a", "b.rs::fn::b", "c.rs::fn::c"]
    );
    assert_eq!(path.edges.len(), 2);
}

#[test]
fn trace_path_respects_direction() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let opts = TraceOptions {
        direction: TraceDirection::Up,
        edge_kinds: &kinds,
        max_depth: 3,
        max_nodes: 50,
    };
    let result = store
        .trace_call_path("c.rs::fn::c", "a.rs::fn::a", &opts)
        .unwrap()
        .expect("both endpoints exist");
    assert!(result.path.is_some());
}

#[test]
fn trace_path_returns_none_when_unreachable() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let opts = TraceOptions::new(TraceDirection::Down, &kinds);
    let result = store
        .trace_call_path("a.rs::fn::a", "d.rs::fn::d", &opts)
        .unwrap()
        .expect("both endpoints exist");
    assert!(result.path.is_none());
}

#[test]
fn trace_path_honors_depth_cap() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let opts = TraceOptions {
        direction: TraceDirection::Down,
        edge_kinds: &kinds,
        max_depth: 1,
        max_nodes: 50,
    };
    let result = store
        .trace_call_path("a.rs::fn::a", "c.rs::fn::c", &opts)
        .unwrap()
        .expect("both endpoints exist");
    assert!(result.path.is_none());
}

#[test]
fn trace_path_missing_endpoint_returns_none() {
    let mut store = open_in_memory();
    setup_chain(&mut store);
    let kinds = calls();
    let opts = TraceOptions::new(TraceDirection::Down, &kinds);
    assert!(
        store
            .trace_call_path("a.rs::fn::a", "missing.rs::fn::nope", &opts)
            .unwrap()
            .is_none()
    );
}
