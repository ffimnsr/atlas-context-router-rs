use super::*;

#[test]
fn node_signatures_by_file_returns_empty_for_unknown_file() {
    let store = open_in_memory();
    let sigs = store.node_signatures_by_file("nonexistent.rs").unwrap();
    assert!(sigs.is_empty());
}

#[test]
fn node_signatures_by_file_returns_entry_per_node() {
    let mut store = open_in_memory();
    let nodes = vec![
        make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust"),
        make_node(NodeKind::Function, "bar", "a.rs::fn::bar", "a.rs", "rust"),
    ];
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h1", Some("rust"), None, &nodes, &[])
        .unwrap();

    let sigs = store.node_signatures_by_file("a.rs").unwrap();
    assert_eq!(sigs.len(), 2);
    assert!(sigs.contains_key("a.rs::fn::foo"));
    assert!(sigs.contains_key("a.rs::fn::bar"));
}

#[test]
fn node_signatures_by_file_for_repo_excludes_same_path_from_other_repo() {
    let mut store = open_in_memory();
    for (repo_id, qname) in [
        ("repo-a", "repo::repo-a::src/lib.rs::fn::run"),
        ("repo-b", "repo::repo-b::src/lib.rs::fn::run"),
    ] {
        let file = ParsedFile {
            path: "src/lib.rs".to_string(),
            language: Some("rust".to_string()),
            hash: format!("{repo_id}-hash"),
            size: None,
            nodes: vec![make_node(
                NodeKind::Function,
                "run",
                qname,
                "src/lib.rs",
                "rust",
            )],
            edges: vec![],
        };
        store
            .replace_files_transactional_for_repo(repo_id, &[file])
            .unwrap();
    }

    let repo_a = store
        .node_signatures_by_file_for_repo("repo-a", "src/lib.rs")
        .unwrap();
    let repo_b = store
        .node_signatures_by_file_for_repo("repo-b", "src/lib.rs")
        .unwrap();

    assert_eq!(repo_a.len(), 1);
    assert!(repo_a.contains_key("repo::repo-a::src/lib.rs::fn::run"));
    assert_eq!(repo_b.len(), 1);
    assert!(repo_b.contains_key("repo::repo-b::src/lib.rs::fn::run"));
}

#[test]
fn node_signatures_stable_across_position_change() {
    let mut store = open_in_memory();
    let mut node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    node.line_start = 1;
    node.line_end = 5;
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h1",
            Some("rust"),
            None,
            &[node.clone()],
            &[],
        )
        .unwrap();
    let sigs_before = store.node_signatures_by_file("a.rs").unwrap();

    // Move the function to different lines — signature must be identical.
    let mut moved = node;
    moved.line_start = 100;
    moved.line_end = 110;
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h2", Some("rust"), None, &[moved], &[])
        .unwrap();
    let sigs_after = store.node_signatures_by_file("a.rs").unwrap();

    assert_eq!(
        sigs_before["a.rs::fn::foo"], sigs_after["a.rs::fn::foo"],
        "moving a function should not change its signature"
    );
}

// -------------------------------------------------------------------------
// file_owner / upsert_file_owner
// -------------------------------------------------------------------------
