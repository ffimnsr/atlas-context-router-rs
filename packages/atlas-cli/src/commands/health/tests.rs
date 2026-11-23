use super::*;
use atlas_core::NodeKind;
use atlas_core::{Edge, EdgeKind, Node, NodeId};

fn open_store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.sqlite");
    Store::open(db_path.to_str().unwrap()).unwrap()
}

fn source_node(qn: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "source".to_owned(),
        qualified_name: qn.to_owned(),
        file_path: "src/lib.rs".to_owned(),
        line_start: 1,
        line_end: 1,
        language: "rust".to_owned(),
        parent_name: Some("src/lib.rs".to_owned()),
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "hash".to_owned(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn dangling_edge(kind: EdgeKind, target_qn: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: "src/lib.rs::fn::source".to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: "src/lib.rs".to_owned(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: None,
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

#[test]
fn structural_dangling_edges_ignores_nonstructural_calls() {
    let mut store = open_store();
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "src/lib.rs",
            "hash",
            Some("rust"),
            None,
            &[source_node("src/lib.rs::fn::source")],
            &[dangling_edge(EdgeKind::Calls, "Path::new")],
        )
        .unwrap();

    assert!(structural_dangling_edges(&store, 100).is_empty());
}

#[test]
fn structural_dangling_edges_keeps_structural_contains() {
    let mut store = open_store();
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "src/lib.rs",
            "hash",
            Some("rust"),
            None,
            &[source_node("src/lib.rs::fn::source")],
            &[dangling_edge(EdgeKind::Contains, "src/lib.rs::fn::missing")],
        )
        .unwrap();

    let dangling = structural_dangling_edges(&store, 100);
    assert_eq!(dangling.len(), 1);
    assert_eq!(dangling[0].3, "contains");
}
