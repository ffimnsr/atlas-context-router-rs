use super::*;

// --- nodes_by_file / edges_by_file ---------------------------------------

#[test]
fn nodes_by_file_returns_only_that_file() {
    let mut store = open_in_memory();
    let na = make_node(NodeKind::Function, "fa", "a.rs::fn::fa", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "fb", "b.rs::fn::fb", "b.rs", "rust");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();

    let got = store.nodes_by_file("a.rs").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "fa");
}

#[test]
fn edges_by_file_returns_only_that_file() {
    let mut store = open_in_memory();
    let nodes_a = vec![make_node(
        NodeKind::Function,
        "fa",
        "a.rs::fn::fa",
        "a.rs",
        "rust",
    )];
    let nodes_b = vec![make_node(
        NodeKind::Function,
        "fb",
        "b.rs::fn::fb",
        "b.rs",
        "rust",
    )];
    let edges_a = vec![make_edge(
        EdgeKind::Calls,
        "a.rs::fn::fa",
        "b.rs::fn::fb",
        "a.rs",
    )];
    let edges_b = vec![make_edge(
        EdgeKind::Calls,
        "b.rs::fn::fb",
        "a.rs::fn::fa",
        "b.rs",
    )];
    store
        .replace_file_graph("a.rs", "h", None, None, &nodes_a, &edges_a)
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &nodes_b, &edges_b)
        .unwrap();

    let got = store.edges_by_file("a.rs").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].source_qn, "a.rs::fn::fa");
}

// --- find_dependents -----------------------------------------------------

#[test]
fn find_dependents_returns_importing_files() {
    let mut store = open_in_memory();
    // a.rs defines "Foo"; b.rs calls Foo.
    let na = make_node(NodeKind::Struct, "Foo", "a.rs::struct::Foo", "a.rs", "rust");
    let nb = make_node(
        NodeKind::Function,
        "use_foo",
        "b.rs::fn::use_foo",
        "b.rs",
        "rust",
    );
    let edge = make_edge(
        EdgeKind::References,
        "b.rs::fn::use_foo",
        "a.rs::struct::Foo",
        "b.rs",
    );
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[edge])
        .unwrap();

    let deps = store.find_dependents(&["a.rs"]).unwrap();
    assert!(deps.contains(&"b.rs".to_string()));
    assert!(!deps.contains(&"a.rs".to_string()));
}

#[test]
fn find_dependents_empty_input_returns_empty() {
    let store = open_in_memory();
    let deps = store.find_dependents(&[]).unwrap();
    assert!(deps.is_empty());
}

// --- impact_radius -------------------------------------------------------

#[test]
fn impact_radius_one_hop() {
    let mut store = open_in_memory();
    // a.rs::fn::a  →Calls→  b.rs::fn::b
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();

    let result = store.impact_radius(&["a.rs"], 3, 200).unwrap();
    assert_eq!(result.changed_nodes.len(), 1);
    assert!(
        result
            .impacted_nodes
            .iter()
            .any(|n| n.qualified_name == "b.rs::fn::b")
    );
    assert!(result.impacted_files.contains(&"b.rs".to_string()));
}

#[test]
fn impact_radius_cyclic_graph_terminates() {
    let mut store = open_in_memory();
    // a → b → a (cycle)
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let e2 = make_edge(EdgeKind::Calls, "b.rs::fn::b", "a.rs::fn::a", "b.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[e1])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[e2])
        .unwrap();

    // Must not loop forever and must return both nodes.
    let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
    let all_qns: Vec<&str> = result
        .changed_nodes
        .iter()
        .chain(result.impacted_nodes.iter())
        .map(|n| n.qualified_name.as_str())
        .collect();
    assert!(all_qns.contains(&"a.rs::fn::a"));
    assert!(all_qns.contains(&"b.rs::fn::b"));
}

#[test]
fn impact_radius_empty_input_returns_empty() {
    let store = open_in_memory();
    let result = store.impact_radius(&[], 3, 200).unwrap();
    assert!(result.changed_nodes.is_empty());
    assert!(result.impacted_nodes.is_empty());
}

#[test]
fn impact_radius_disconnected_graph() {
    let mut store = open_in_memory();
    // a.rs and c.rs exist but share no edges; b.rs is connected to a.rs.
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
    let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();
    store
        .replace_file_graph("c.rs", "h", None, None, &[nc], &[])
        .unwrap();

    let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
    // b.rs is reachable; c.rs is disconnected and must not appear.
    let all_qns: Vec<&str> = result
        .changed_nodes
        .iter()
        .chain(result.impacted_nodes.iter())
        .map(|n| n.qualified_name.as_str())
        .collect();
    assert!(
        all_qns.contains(&"a.rs::fn::a"),
        "seed node must be present"
    );
    assert!(
        all_qns.contains(&"b.rs::fn::b"),
        "connected node must be present"
    );
    assert!(
        !all_qns.contains(&"c.rs::fn::c"),
        "disconnected node must not appear"
    );
}

#[test]
fn impact_radius_depth_cap() {
    let mut store = open_in_memory();
    // Chain: a → b → c → d  (each hop is one depth unit)
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
    let nd = make_node(NodeKind::Function, "d", "d.rs::fn::d", "d.rs", "rust");
    let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let e2 = make_edge(EdgeKind::Calls, "b.rs::fn::b", "c.rs::fn::c", "b.rs");
    let e3 = make_edge(EdgeKind::Calls, "c.rs::fn::c", "d.rs::fn::d", "c.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[e1])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[e2])
        .unwrap();
    store
        .replace_file_graph("c.rs", "h", None, None, &[nc], &[e3])
        .unwrap();
    store
        .replace_file_graph("d.rs", "h", None, None, &[nd], &[])
        .unwrap();

    // max_depth=1: only b should be reachable beyond the seed.
    let result = store.impact_radius(&["a.rs"], 1, 200).unwrap();
    let impacted_qns: Vec<&str> = result
        .impacted_nodes
        .iter()
        .map(|n| n.qualified_name.as_str())
        .collect();
    assert!(
        impacted_qns.contains(&"b.rs::fn::b"),
        "one-hop node must be reachable at depth=1"
    );
    assert!(
        !impacted_qns.contains(&"c.rs::fn::c"),
        "two-hop node must not be reachable at depth=1"
    );
    assert!(
        !impacted_qns.contains(&"d.rs::fn::d"),
        "three-hop node must not be reachable at depth=1"
    );
}

#[test]
fn impact_radius_max_node_cap() {
    let mut store = open_in_memory();
    // Star topology: a.rs is the seed; b, c, d, e each called from a.
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nodes_src: Vec<(&str, &str, &str)> = vec![
        ("b", "b.rs::fn::b", "b.rs"),
        ("c", "c.rs::fn::c", "c.rs"),
        ("d", "d.rs::fn::d", "d.rs"),
        ("e", "e.rs::fn::e", "e.rs"),
    ];
    let mut edges: Vec<Edge> = nodes_src
        .iter()
        .map(|(_, qn, fp)| make_edge(EdgeKind::Calls, "a.rs::fn::a", qn, fp))
        .collect();
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &edges)
        .unwrap();
    // Clear so we can reuse this vec as a per-file edges slice.
    edges.clear();
    for (name, qn, fp) in &nodes_src {
        let n = make_node(NodeKind::Function, name, qn, fp, "rust");
        store
            .replace_file_graph(fp, "h", None, None, &[n], &[])
            .unwrap();
    }

    // Cap at 2 total nodes; seed a.rs has 1 node, so at most 1 impacted node
    // should be returned regardless of star size.
    let result = store.impact_radius(&["a.rs"], 5, 2).unwrap();
    let total = result.changed_nodes.len() + result.impacted_nodes.len();
    assert!(
        total <= 2,
        "total nodes must not exceed max_nodes cap; got {total}"
    );
}

#[test]
fn impact_radius_deleted_seed_file_returns_empty() {
    let mut store = open_in_memory();
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();

    // Delete the seed file before querying impact.
    store.delete_file_graph("a.rs").unwrap();

    let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
    assert!(
        result.changed_nodes.is_empty(),
        "deleted seed file must yield no changed nodes"
    );
    assert!(
        result.impacted_nodes.is_empty(),
        "deleted seed file must yield no impacted nodes"
    );
}

#[test]
fn impact_radius_seed_file_with_no_nodes() {
    let mut store = open_in_memory();
    // Index a file record with zero nodes.
    store
        .replace_file_graph("empty.rs", "h", None, None, &[], &[])
        .unwrap();

    let result = store.impact_radius(&["empty.rs"], 5, 200).unwrap();
    assert!(
        result.changed_nodes.is_empty(),
        "file with no nodes must yield no changed nodes"
    );
    assert!(
        result.impacted_nodes.is_empty(),
        "file with no nodes must yield no impacted nodes"
    );
    assert!(result.relevant_edges.is_empty());
}

// --- file_hashes ---------------------------------------------------------

#[test]
fn file_hashes_returns_stored_hashes() {
    let mut store = open_in_memory();
    let nodes_a = vec![make_node(
        NodeKind::Function,
        "f",
        "a.rs::fn::f",
        "a.rs",
        "rust",
    )];
    let nodes_b = vec![make_node(
        NodeKind::Function,
        "g",
        "b.rs::fn::g",
        "b.rs",
        "go",
    )];
    store
        .replace_file_graph("a.rs", "hash_aaa", Some("rust"), None, &nodes_a, &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "hash_bbb", Some("go"), None, &nodes_b, &[])
        .unwrap();

    let hashes = store.file_hashes().unwrap();
    assert_eq!(hashes.get("a.rs").map(String::as_str), Some("hash_aaa"));
    assert_eq!(hashes.get("b.rs").map(String::as_str), Some("hash_bbb"));
}

#[test]
fn file_hashes_empty_when_no_files() {
    let store = open_in_memory();
    let hashes = store.file_hashes().unwrap();
    assert!(hashes.is_empty());
}

#[test]
fn file_hashes_updated_after_replace() {
    let mut store = open_in_memory();
    let nodes = vec![make_node(
        NodeKind::Function,
        "f",
        "a.rs::fn::f",
        "a.rs",
        "rust",
    )];
    store
        .replace_file_graph("a.rs", "old_hash", None, None, &nodes, &[])
        .unwrap();
    store
        .replace_file_graph("a.rs", "new_hash", None, None, &nodes, &[])
        .unwrap();

    let hashes = store.file_hashes().unwrap();
    assert_eq!(hashes.get("a.rs").map(String::as_str), Some("new_hash"));
    assert_eq!(hashes.len(), 1);
}

// --- impact_radius limits ------------------------------------------------

#[test]
fn impact_radius_respects_depth_limit() {
    let mut store = open_in_memory();
    // a → b → c → d: with depth=1, only b should be reachable from a.
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
    let nd = make_node(NodeKind::Function, "d", "d.rs::fn::d", "d.rs", "rust");
    let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let e2 = make_edge(EdgeKind::Calls, "b.rs::fn::b", "c.rs::fn::c", "b.rs");
    let e3 = make_edge(EdgeKind::Calls, "c.rs::fn::c", "d.rs::fn::d", "c.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[e1])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[e2])
        .unwrap();
    store
        .replace_file_graph("c.rs", "h", None, None, &[nc], &[e3])
        .unwrap();
    store
        .replace_file_graph("d.rs", "h", None, None, &[nd], &[])
        .unwrap();

    let result = store.impact_radius(&["a.rs"], 1, 200).unwrap();
    let all_files: Vec<&str> = result
        .changed_nodes
        .iter()
        .chain(result.impacted_nodes.iter())
        .map(|n| n.file_path.as_str())
        .collect();
    // At depth 1 from a.rs, b.rs should be reachable but c.rs and d.rs should not.
    assert!(
        all_files.contains(&"b.rs"),
        "b.rs should be reached at depth 1"
    );
    assert!(
        !all_files.contains(&"d.rs"),
        "d.rs beyond depth limit should not appear"
    );
}

#[test]
fn impact_radius_respects_node_count_limit() {
    let mut store = open_in_memory();
    // Create a fan-out: a → b, a → c, a → d, a → e (4 targets)
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
    let nd = make_node(NodeKind::Function, "d", "d.rs::fn::d", "d.rs", "rust");
    let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let e2 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "c.rs::fn::c", "a.rs");
    let e3 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "d.rs::fn::d", "a.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[e1, e2, e3])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();
    store
        .replace_file_graph("c.rs", "h", None, None, &[nc], &[])
        .unwrap();
    store
        .replace_file_graph("d.rs", "h", None, None, &[nd], &[])
        .unwrap();

    // Limit to 2 total nodes — should stop before visiting all of b/c/d.
    let result = store.impact_radius(&["a.rs"], 5, 2).unwrap();
    let total = result.changed_nodes.len() + result.impacted_nodes.len();
    assert!(
        total <= 2,
        "node count limit must be respected; got {total}"
    );
}

// -------------------------------------------------------------------------
// find_dependents_for_qnames
// -------------------------------------------------------------------------

#[test]
fn find_dependents_for_qnames_returns_importers_of_changed_symbols() {
    let mut store = open_in_memory();

    // a.rs defines `foo`
    let node_a = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node_a], &[])
        .unwrap();

    // b.rs defines `bar` and calls a.rs::fn::foo
    let node_b = make_node(NodeKind::Function, "bar", "b.rs::fn::bar", "b.rs", "rust");
    let edge_b_to_a = make_edge(EdgeKind::Calls, "b.rs::fn::bar", "a.rs::fn::foo", "b.rs");
    store
        .replace_file_graph("b.rs", "h2", Some("rust"), None, &[node_b], &[edge_b_to_a])
        .unwrap();

    // c.rs defines `qux` with no edges
    let node_c = make_node(NodeKind::Function, "qux", "c.rs::fn::qux", "c.rs", "rust");
    store
        .replace_file_graph("c.rs", "h3", Some("rust"), None, &[node_c], &[])
        .unwrap();

    // Changing a.rs::fn::foo should only invalidate b.rs, not c.rs.
    let deps = store
        .find_dependents_for_qnames(&["a.rs::fn::foo"])
        .unwrap();
    assert_eq!(deps, vec!["b.rs"]);
}

#[test]
fn find_dependents_for_qnames_empty_input_returns_empty() {
    let store = open_in_memory();
    let deps = store.find_dependents_for_qnames(&[]).unwrap();
    assert!(deps.is_empty());
}

#[test]
fn find_dependents_for_qnames_no_edges_returns_empty() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node], &[])
        .unwrap();
    let deps = store
        .find_dependents_for_qnames(&["a.rs::fn::foo"])
        .unwrap();
    assert!(deps.is_empty());
}
