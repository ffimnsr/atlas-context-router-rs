use super::*;

fn setup_call_graph(store: &mut Store) {
    // a.rs: caller -> b.rs: callee
    let caller = make_node(
        NodeKind::Function,
        "caller",
        "a.rs::fn::caller",
        "a.rs",
        "rust",
    );
    let callee = make_node(
        NodeKind::Function,
        "callee",
        "b.rs::fn::callee",
        "b.rs",
        "rust",
    );
    let edge = make_edge(
        EdgeKind::Calls,
        "a.rs::fn::caller",
        "b.rs::fn::callee",
        "a.rs",
    );
    store
        .replace_file_graph(
            "a.rs",
            "h1",
            Some("rust"),
            None,
            &[caller],
            std::slice::from_ref(&edge),
        )
        .unwrap();
    store
        .replace_file_graph("b.rs", "h2", Some("rust"), None, &[callee], &[])
        .unwrap();
}

// --- node_by_qname -------------------------------------------------------

#[test]
fn node_by_qname_exact_hit() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let node = store.node_by_qname("a.rs::fn::caller").unwrap();
    assert!(node.is_some());
    assert_eq!(node.unwrap().name, "caller");
}

#[test]
fn node_by_qname_missing_returns_none() {
    let store = open_in_memory();
    let node = store.node_by_qname("nonexistent::fn::missing").unwrap();
    assert!(node.is_none());
}

// --- nodes_by_name -------------------------------------------------------

#[test]
fn nodes_by_name_single_match() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let nodes = store.nodes_by_name("caller", 10).unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].qualified_name, "a.rs::fn::caller");
}

#[test]
fn nodes_by_name_multiple_matches() {
    let mut store = open_in_memory();
    let n1 = make_node(
        NodeKind::Function,
        "process",
        "a.rs::fn::process",
        "a.rs",
        "rust",
    );
    let n2 = make_node(
        NodeKind::Function,
        "process",
        "b.rs::fn::process",
        "b.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[n1], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h2", None, None, &[n2], &[])
        .unwrap();
    let nodes = store.nodes_by_name("process", 10).unwrap();
    assert_eq!(nodes.len(), 2);
}

#[test]
fn nodes_by_name_limit_respected() {
    let mut store = open_in_memory();
    let n1 = make_node(
        NodeKind::Function,
        "process",
        "a.rs::fn::process",
        "a.rs",
        "rust",
    );
    let n2 = make_node(
        NodeKind::Function,
        "process",
        "b.rs::fn::process",
        "b.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[n1], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h2", None, None, &[n2], &[])
        .unwrap();
    let nodes = store.nodes_by_name("process", 1).unwrap();
    assert_eq!(nodes.len(), 1);
}

#[test]
fn nodes_by_name_missing_returns_empty() {
    let store = open_in_memory();
    let nodes = store.nodes_by_name("ghost_fn", 10).unwrap();
    assert!(nodes.is_empty());
}

// --- direct_callers -------------------------------------------------------

#[test]
fn direct_callers_returns_caller() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let callers = store.direct_callers("b.rs::fn::callee", 10).unwrap();
    assert_eq!(callers.len(), 1);
    let (node, edge) = &callers[0];
    assert_eq!(node.qualified_name, "a.rs::fn::caller");
    assert_eq!(edge.target_qn, "b.rs::fn::callee");
    assert_eq!(edge.kind, EdgeKind::Calls);
}

#[test]
fn direct_callers_empty_for_no_callers() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    // caller has no incoming call edges
    let callers = store.direct_callers("a.rs::fn::caller", 10).unwrap();
    assert!(callers.is_empty());
}

#[test]
fn direct_callers_missing_node_returns_empty() {
    let store = open_in_memory();
    let callers = store.direct_callers("does::not::exist", 10).unwrap();
    assert!(callers.is_empty());
}

// --- direct_callees -------------------------------------------------------

#[test]
fn direct_callees_returns_callee() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let callees = store.direct_callees("a.rs::fn::caller", 10).unwrap();
    assert_eq!(callees.len(), 1);
    let (node, edge) = &callees[0];
    assert_eq!(node.qualified_name, "b.rs::fn::callee");
    assert_eq!(edge.source_qn, "a.rs::fn::caller");
    assert_eq!(edge.kind, EdgeKind::Calls);
}

#[test]
fn direct_callees_empty_for_no_callees() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let callees = store.direct_callees("b.rs::fn::callee", 10).unwrap();
    assert!(callees.is_empty());
}

#[test]
fn direct_callees_missing_node_returns_empty() {
    let store = open_in_memory();
    let callees = store.direct_callees("does::not::exist", 10).unwrap();
    assert!(callees.is_empty());
}

// --- import_neighbors ----------------------------------------------------

fn setup_import_graph(store: &mut Store) {
    let importer = make_node(
        NodeKind::Module,
        "mod_a",
        "a.rs::mod::mod_a",
        "a.rs",
        "rust",
    );
    let importee = make_node(
        NodeKind::Module,
        "mod_b",
        "b.rs::mod::mod_b",
        "b.rs",
        "rust",
    );
    let edge = make_edge(
        EdgeKind::Imports,
        "a.rs::mod::mod_a",
        "b.rs::mod::mod_b",
        "a.rs",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[importer], &[edge])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h2", None, None, &[importee], &[])
        .unwrap();
}

#[test]
fn import_neighbors_forward_direction() {
    let mut store = open_in_memory();
    setup_import_graph(&mut store);
    // a imports b → b is a neighbor of a
    let neighbors = store.import_neighbors("a.rs::mod::mod_a", 10).unwrap();
    assert_eq!(neighbors.len(), 1);
    let (node, edge) = &neighbors[0];
    assert_eq!(node.qualified_name, "b.rs::mod::mod_b");
    assert_eq!(edge.kind, EdgeKind::Imports);
}

#[test]
fn import_neighbors_backward_direction() {
    let mut store = open_in_memory();
    setup_import_graph(&mut store);
    // b is imported by a → a is a neighbor of b
    let neighbors = store.import_neighbors("b.rs::mod::mod_b", 10).unwrap();
    assert_eq!(neighbors.len(), 1);
    let (node, _edge) = &neighbors[0];
    assert_eq!(node.qualified_name, "a.rs::mod::mod_a");
}

#[test]
fn import_neighbors_missing_node_returns_empty() {
    let store = open_in_memory();
    let neighbors = store.import_neighbors("no::such::module", 10).unwrap();
    assert!(neighbors.is_empty());
}

// --- containment_siblings ------------------------------------------------

fn setup_sibling_graph(store: &mut Store) {
    let parent_method = |name: &str, qn: &str| -> Node {
        let mut n = make_node(NodeKind::Method, name, qn, "a.rs", "rust");
        n.parent_name = Some("MyClass".to_string());
        n
    };
    let n1 = parent_method("method_a", "a.rs::MyClass::method_a");
    let n2 = parent_method("method_b", "a.rs::MyClass::method_b");
    let n3 = parent_method("method_c", "a.rs::MyClass::method_c");
    store
        .replace_file_graph("a.rs", "h1", None, None, &[n1, n2, n3], &[])
        .unwrap();
}

#[test]
fn containment_siblings_returns_same_parent() {
    let mut store = open_in_memory();
    setup_sibling_graph(&mut store);
    let siblings = store
        .containment_siblings("a.rs::MyClass::method_a", 10)
        .unwrap();
    assert_eq!(siblings.len(), 2);
    let names: Vec<_> = siblings.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"method_b"));
    assert!(names.contains(&"method_c"));
    // seed itself must be excluded
    assert!(!names.contains(&"method_a"));
}

#[test]
fn containment_siblings_limit_respected() {
    let mut store = open_in_memory();
    setup_sibling_graph(&mut store);
    let siblings = store
        .containment_siblings("a.rs::MyClass::method_a", 1)
        .unwrap();
    assert_eq!(siblings.len(), 1);
}

#[test]
fn containment_siblings_no_parent_returns_empty() {
    let mut store = open_in_memory();
    // node with no parent_name
    let n = make_node(
        NodeKind::Function,
        "standalone",
        "a.rs::fn::standalone",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[n], &[])
        .unwrap();
    let siblings = store
        .containment_siblings("a.rs::fn::standalone", 10)
        .unwrap();
    assert!(siblings.is_empty());
}

#[test]
fn containment_siblings_missing_node_returns_empty() {
    let store = open_in_memory();
    let siblings = store.containment_siblings("does::not::exist", 10).unwrap();
    assert!(siblings.is_empty());
}

// --- test_neighbors ------------------------------------------------------

fn setup_test_graph(store: &mut Store) {
    let src = make_node(
        NodeKind::Function,
        "parse",
        "a.rs::fn::parse",
        "a.rs",
        "rust",
    );
    let mut test_node = make_node(
        NodeKind::Test,
        "test_parse",
        "tests.rs::test::test_parse",
        "tests.rs",
        "rust",
    );
    test_node.is_test = true;
    let edge = make_edge(
        EdgeKind::Tests,
        "tests.rs::test::test_parse",
        "a.rs::fn::parse",
        "tests.rs",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[src], &[])
        .unwrap();
    store
        .replace_file_graph("tests.rs", "h2", None, None, &[test_node], &[edge])
        .unwrap();
}

#[test]
fn test_neighbors_source_node_finds_test() {
    let mut store = open_in_memory();
    setup_test_graph(&mut store);
    // parse is tested by test_parse → test_parse must appear
    let neighbors = store.test_neighbors("a.rs::fn::parse", 10).unwrap();
    assert_eq!(neighbors.len(), 1);
    let (node, edge) = &neighbors[0];
    assert_eq!(node.qualified_name, "tests.rs::test::test_parse");
    assert_eq!(edge.kind, EdgeKind::Tests);
}

#[test]
fn test_neighbors_test_node_finds_source() {
    let mut store = open_in_memory();
    setup_test_graph(&mut store);
    // test_parse tests parse → parse must appear
    let neighbors = store
        .test_neighbors("tests.rs::test::test_parse", 10)
        .unwrap();
    assert_eq!(neighbors.len(), 1);
    let (node, _edge) = &neighbors[0];
    assert_eq!(node.qualified_name, "a.rs::fn::parse");
}

#[test]
fn test_neighbors_missing_node_returns_empty() {
    let store = open_in_memory();
    let neighbors = store.test_neighbors("no::test::here", 10).unwrap();
    assert!(neighbors.is_empty());
}
