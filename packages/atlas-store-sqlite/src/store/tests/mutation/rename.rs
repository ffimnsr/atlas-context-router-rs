use super::*;

#[test]
fn rename_file_graph_moves_nodes_and_edges() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "foo",
        "old.rs::fn::foo",
        "old.rs",
        "rust",
    );
    let edge = make_edge(
        EdgeKind::Calls,
        "old.rs::fn::foo",
        "old.rs::fn::foo",
        "old.rs",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "old.rs",
            "h1",
            Some("rust"),
            None,
            &[node],
            &[edge],
        )
        .unwrap();

    store.rename_file_graph("old.rs", "new.rs").unwrap();

    // old path must be gone
    assert_eq!(store.nodes_by_file("old.rs").unwrap().len(), 0);
    assert_eq!(store.file_hash("old.rs").unwrap(), None);

    // new path must have the node and edge
    let new_nodes = store.nodes_by_file("new.rs").unwrap();
    assert_eq!(new_nodes.len(), 1);
    assert_eq!(new_nodes[0].file_path, "new.rs");
    assert_eq!(new_nodes[0].qualified_name, "old.rs::fn::foo");

    let new_edges = store.edges_by_file("new.rs").unwrap();
    assert_eq!(new_edges.len(), 1);
    assert_eq!(new_edges[0].file_path, "new.rs");

    // files row moved
    assert_eq!(store.file_hash("new.rs").unwrap(), Some("h1".to_string()));
}

#[test]
fn rename_file_graph_for_repo_preserves_other_repo_rows() {
    let mut store = open_in_memory();
    let repo_a_file = ParsedFile {
        path: "old.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "repo-a-hash".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "a",
            "repo::repo-a::old.rs::fn::a",
            "old.rs",
            "rust",
        )],
        edges: vec![],
    };
    let repo_b_file = ParsedFile {
        path: "old.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "repo-b-hash".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "b",
            "repo::repo-b::old.rs::fn::b",
            "old.rs",
            "rust",
        )],
        edges: vec![],
    };
    store
        .replace_files_transactional_for_repo("repo-a", &[repo_a_file])
        .unwrap();
    store
        .replace_files_transactional_for_repo("repo-b", &[repo_b_file])
        .unwrap();

    store
        .rename_file_graph_for_repo("repo-a", "old.rs", "new.rs")
        .unwrap();

    assert_eq!(
        store.file_hash_for_repo("repo-a", "new.rs").unwrap(),
        Some("repo-a-hash".to_string())
    );
    assert_eq!(store.file_hash_for_repo("repo-a", "old.rs").unwrap(), None);
    assert_eq!(
        store.file_hash_for_repo("repo-b", "old.rs").unwrap(),
        Some("repo-b-hash".to_string())
    );
    assert_eq!(store.file_hash_for_repo("repo-b", "new.rs").unwrap(), None);

    let repo_b_old_nodes: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE source_repo_id = 'repo-b' AND file_path = 'old.rs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repo_b_old_nodes, 1);
}

#[test]
fn rename_file_graph_preserves_node_ids() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h1", Some("rust"), None, &[node], &[])
        .unwrap();

    let id_before = store.nodes_by_file("a.rs").unwrap()[0].id;
    store.rename_file_graph("a.rs", "b.rs").unwrap();
    let id_after = store.nodes_by_file("b.rs").unwrap()[0].id;

    assert_eq!(id_before, id_after, "node id must be stable across rename");
}

#[test]
fn rename_file_graph_updates_fts_index() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "myfunc",
        "old.rs::fn::myfunc",
        "old.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "old.rs",
            "h1",
            Some("rust"),
            None,
            &[node],
            &[],
        )
        .unwrap();

    store.rename_file_graph("old.rs", "new.rs").unwrap();

    // FTS search by new file_path must return the node.
    let results = store
        .search(&atlas_core::SearchQuery {
            text: "myfunc".to_string(),
            file_path: Some("new.rs".to_string()),
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(results.len(), 1, "FTS must find node at new path");
    assert_eq!(results[0].node.file_path, "new.rs");

    // FTS search by old file_path must return nothing.
    let old_results = store
        .search(&atlas_core::SearchQuery {
            text: "myfunc".to_string(),
            file_path: Some("old.rs".to_string()),
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert!(
        old_results.is_empty(),
        "FTS must not return node at old path"
    );
}
