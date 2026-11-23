use super::*;

// --- replace_file_graph --------------------------------------------------

#[test]
fn nodes_with_same_qname_coexist_across_repo_ids() {
    let mut store = open_in_memory();
    let parsed = |hash: &str| ParsedFile {
        path: "a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: hash.to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "x",
            "a.rs::fn::x",
            "a.rs",
            "rust",
        )],
        edges: vec![],
    };

    // Un-namespaced qnames collide on relative path across repos: each repo
    // must keep its own node row (UNIQUE (source_repo_id, qualified_name)).
    store
        .replace_files_transactional_for_repo("repo_a", &[parsed("h1")])
        .unwrap();
    store
        .replace_files_transactional_for_repo("repo_b", &[parsed("h2")])
        .unwrap();

    let count: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM nodes
             WHERE qualified_name = 'a.rs::fn::x'
               AND (source_repo_id = 'repo_a' OR source_repo_id = 'repo_b')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 2,
        "each repo keeps its own node row for the same qname"
    );
}

#[test]
fn file_paths_under_dir_uses_literal_half_open_range() {
    let mut store = open_in_memory();
    let parsed = |path: &str| ParsedFile {
        path: path.to_string(),
        language: Some("rust".to_string()),
        hash: "h1".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "x",
            &format!("{path}::fn::x"),
            path,
            "rust",
        )],
        edges: vec![],
    };
    let paths = [
        "src/dir/a.rs",
        "src/dir/b.rs",
        "src/dir", // same-name file is not a child
        "src/dir0.rs",
        "src/dirX/b.rs",
        "src/dir.x/b.rs",
    ];
    let files = paths.iter().map(|path| parsed(path)).collect::<Vec<_>>();
    store
        .replace_files_transactional_for_repo("repo_a", &files)
        .unwrap();

    assert_eq!(
        store
            .file_paths_under_dir_for_repo("repo_a", "src/dir")
            .unwrap(),
        ["src/dir/a.rs", "src/dir/b.rs"]
    );
}

#[test]
fn file_graph_exists_tracks_repo_scoped_footprint() {
    let mut store = open_in_memory();
    let parsed = ParsedFile {
        path: "a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "h1".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "x",
            "a.rs::fn::x",
            "a.rs",
            "rust",
        )],
        edges: vec![],
    };
    store
        .replace_files_transactional_for_repo("repo_a", &[parsed])
        .unwrap();

    assert!(store.file_graph_exists_for_repo("repo_a", "a.rs").unwrap());
    assert!(!store.file_graph_exists_for_repo("repo_a", "b.rs").unwrap());
    assert!(!store.file_graph_exists_for_repo("repo_b", "a.rs").unwrap());

    store.delete_file_graph_for_repo("repo_a", "a.rs").unwrap();
    assert!(!store.file_graph_exists_for_repo("repo_a", "a.rs").unwrap());
}

#[test]
fn chunk_replacement_is_repo_scoped() {
    let mut store = open_in_memory();
    let parsed = |hash: &str, name: &str, qn: &str| ParsedFile {
        path: "a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: hash.to_string(),
        size: None,
        nodes: vec![make_node(NodeKind::Function, name, qn, "a.rs", "rust")],
        edges: vec![],
    };

    store
        .replace_files_transactional_for_repo("repo_a", &[parsed("h1", "x", "a.rs::fn::x")])
        .unwrap();
    store
        .replace_files_transactional_for_repo("repo_b", &[parsed("h2", "y", "a.rs::fn::y")])
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", &[parsed("h1", "x", "a.rs::fn::x")])
        .unwrap();

    // Replacing chunks for repo_b must not delete repo_a's chunks
    // (same relative path in both repos).
    store
        .replace_chunks_for_parsed_files("repo_b", &[parsed("h2", "y", "a.rs::fn::y")])
        .unwrap();

    let mut stmt = store
        .conn
        .prepare("SELECT node_qn FROM retrieval_chunks ORDER BY node_qn")
        .unwrap();
    let qns: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(
        qns,
        ["a.rs::fn::x", "a.rs::fn::y"],
        "chunk replacement must stay within the source repo"
    );
}

fn chunk_qns(store: &Store) -> Vec<String> {
    let mut stmt = store
        .conn
        .prepare("SELECT node_qn FROM retrieval_chunks ORDER BY node_qn")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}

fn chunk_owners(store: &Store) -> Vec<(String, String)> {
    let mut stmt = store
        .conn
        .prepare(
            "SELECT source_repo_id, node_qn FROM retrieval_chunks
             ORDER BY source_repo_id, node_qn, chunk_idx",
        )
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap()
}

#[test]
fn replace_file_graph_drops_chunks_for_removed_symbols() {
    let mut store = open_in_memory();
    let parsed = |hash: &str, names: &[(&str, &str)]| ParsedFile {
        path: "src/a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: hash.to_string(),
        size: None,
        nodes: names
            .iter()
            .map(|(name, qn)| make_node(NodeKind::Function, name, qn, "src/a.rs", "rust"))
            .collect(),
        edges: vec![],
    };
    let both = parsed(
        "h1",
        &[
            ("keep", "src/a.rs::fn::keep"),
            ("dead", "src/a.rs::fn::dead"),
        ],
    );

    store
        .replace_files_transactional_for_repo("repo_a", std::slice::from_ref(&both))
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", std::slice::from_ref(&both))
        .unwrap();
    assert_eq!(chunk_qns(&store).len(), 2);

    // Editing the file removes `dead`; the stale chunk must not survive.
    let kept = parsed("h2", &[("keep", "src/a.rs::fn::keep")]);
    store
        .replace_files_transactional_for_repo("repo_a", std::slice::from_ref(&kept))
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", std::slice::from_ref(&kept))
        .unwrap();

    assert_eq!(chunk_qns(&store), ["src/a.rs::fn::keep"]);
}

#[test]
fn delete_file_graph_for_repo_removes_chunks() {
    let mut store = open_in_memory();
    let file = ParsedFile {
        path: "src/gone.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "h1".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "gone",
            "src/gone.rs::fn::gone",
            "src/gone.rs",
            "rust",
        )],
        edges: vec![],
    };
    store
        .replace_files_transactional_for_repo("repo_a", std::slice::from_ref(&file))
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", std::slice::from_ref(&file))
        .unwrap();
    assert_eq!(chunk_qns(&store), ["src/gone.rs::fn::gone"]);

    store
        .delete_file_graph_for_repo("repo_a", "src/gone.rs")
        .unwrap();

    assert!(
        chunk_qns(&store).is_empty(),
        "deleted files must not leave retrieval chunks behind"
    );
    let nodes: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM nodes WHERE file_path = 'src/gone.rs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(nodes, 0);
}

#[test]
fn delete_file_graph_keeps_chunks_backed_by_other_repo() {
    let mut store = open_in_memory();
    let parsed = |hash: &str| ParsedFile {
        path: "a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: hash.to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "x",
            "a.rs::fn::x",
            "a.rs",
            "rust",
        )],
        edges: vec![],
    };
    store
        .replace_files_transactional_for_repo("repo_a", &[parsed("h1")])
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", &[parsed("h1")])
        .unwrap();
    store
        .replace_files_transactional_for_repo("repo_b", &[parsed("h2")])
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_b", &[parsed("h2")])
        .unwrap();
    assert_eq!(
        chunk_owners(&store),
        [
            ("repo_a".to_string(), "a.rs::fn::x".to_string()),
            ("repo_b".to_string(), "a.rs::fn::x".to_string()),
        ]
    );

    // Same qname exists in repo_b: deleting repo_a's file must not drop
    // repo_b's chunk row.
    store.delete_file_graph_for_repo("repo_a", "a.rs").unwrap();
    assert_eq!(
        chunk_owners(&store),
        [("repo_b".to_string(), "a.rs::fn::x".to_string())]
    );

    store.delete_file_graph_for_repo("repo_b", "a.rs").unwrap();
    assert!(
        chunk_qns(&store).is_empty(),
        "last owner of the qname clears the chunk"
    );
}

#[test]
fn replace_file_graph_keeps_chunks_backed_by_other_repo() {
    let mut store = open_in_memory();
    let parsed = |hash: &str, names: &[(&str, &str)]| ParsedFile {
        path: "a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: hash.to_string(),
        size: None,
        nodes: names
            .iter()
            .map(|(name, qn)| make_node(NodeKind::Function, name, qn, "a.rs", "rust"))
            .collect(),
        edges: vec![],
    };
    let both = parsed("h1", &[("x", "a.rs::fn::x"), ("y", "a.rs::fn::y")]);
    store
        .replace_files_transactional_for_repo("repo_a", std::slice::from_ref(&both))
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", std::slice::from_ref(&both))
        .unwrap();
    let shared = parsed("h2", &[("x", "a.rs::fn::x")]);
    store
        .replace_files_transactional_for_repo("repo_b", std::slice::from_ref(&shared))
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_b", std::slice::from_ref(&shared))
        .unwrap();

    // repo_a drops `y` while repo_b still owns `x`; each repo keeps its own
    // chunk row for the shared qname.
    let kept = parsed("h3", &[("x", "a.rs::fn::x")]);
    store
        .replace_files_transactional_for_repo("repo_a", std::slice::from_ref(&kept))
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", std::slice::from_ref(&kept))
        .unwrap();

    assert_eq!(
        chunk_owners(&store),
        [
            ("repo_a".to_string(), "a.rs::fn::x".to_string()),
            ("repo_b".to_string(), "a.rs::fn::x".to_string()),
        ]
    );
}

#[test]
fn replace_file_graph_inserts_nodes_and_edges() {
    let mut store = open_in_memory();
    let nodes = vec![
        make_node(
            NodeKind::Function,
            "foo",
            "src/a.rs::fn::foo",
            "src/a.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "bar",
            "src/a.rs::fn::bar",
            "src/a.rs",
            "rust",
        ),
    ];
    let edges = vec![make_edge(
        EdgeKind::Calls,
        "src/a.rs::fn::foo",
        "src/a.rs::fn::bar",
        "src/a.rs",
    )];

    store
        .replace_file_graph_for_repo(
            "repo_test",
            "src/a.rs",
            "hash1",
            Some("rust"),
            Some(200),
            &nodes,
            &edges,
        )
        .unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.file_count, 1);
    assert_eq!(stats.node_count, 2);
    assert_eq!(stats.edge_count, 1);
}

#[test]
fn replace_file_graph_is_idempotent() {
    let mut store = open_in_memory();
    let nodes = vec![make_node(
        NodeKind::Function,
        "foo",
        "a.rs::fn::foo",
        "a.rs",
        "rust",
    )];
    let edges: Vec<Edge> = vec![];

    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h1",
            Some("rust"),
            None,
            &nodes,
            &edges,
        )
        .unwrap();
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h2",
            Some("rust"),
            None,
            &nodes,
            &edges,
        )
        .unwrap();

    // Second replace must not double the counts.
    let stats = store.stats().unwrap();
    assert_eq!(stats.node_count, 1);
    assert_eq!(stats.file_count, 1);
}

#[test]
fn replace_file_graph_updates_nodes() {
    let mut store = open_in_memory();
    let first = vec![make_node(
        NodeKind::Function,
        "old",
        "a.rs::fn::old",
        "a.rs",
        "rust",
    )];
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h1", None, None, &first, &[])
        .unwrap();

    let second = vec![make_node(
        NodeKind::Function,
        "new_fn",
        "a.rs::fn::new_fn",
        "a.rs",
        "rust",
    )];
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h2", None, None, &second, &[])
        .unwrap();

    let got = store.nodes_by_file("a.rs").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "new_fn");
}

#[test]
fn replace_file_graph_canonicalizes_equivalent_raw_paths() {
    let mut store = open_in_memory();
    let nodes = vec![make_node(
        NodeKind::Function,
        "foo",
        "src\\module.rs::fn::foo",
        "src\\module.rs",
        "rust",
    )];
    let edges = vec![make_edge(
        EdgeKind::Calls,
        "src\\module.rs::fn::foo",
        "src\\module.rs::fn::foo",
        "src\\module.rs",
    )];

    store
        .replace_file_graph_for_repo(
            "repo_test",
            "./src/feature/../module.rs",
            "hash1",
            Some("rust"),
            Some(200),
            &nodes,
            &edges,
        )
        .unwrap();

    let stored_nodes = store.nodes_by_file("src/module.rs").unwrap();
    assert_eq!(stored_nodes.len(), 1);
    assert_eq!(stored_nodes[0].file_path, "src/module.rs");
    assert_eq!(stored_nodes[0].qualified_name, "src/module.rs::fn::foo");

    let stored_edges = store.edges_by_file("src/module.rs").unwrap();
    assert_eq!(stored_edges.len(), 1);
    assert_eq!(stored_edges[0].file_path, "src/module.rs");
    assert_eq!(stored_edges[0].source_qn, "src/module.rs::fn::foo");
    assert_eq!(stored_edges[0].target_qn, "src/module.rs::fn::foo");

    let file_count: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path = 'src/module.rs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(file_count, 1);
    assert_eq!(
        store.file_hash("src/module.rs").unwrap(),
        Some("hash1".to_string())
    );
}

// --- delete_file_graph ---------------------------------------------------

#[test]
fn delete_file_graph_removes_all_rows() {
    let mut store = open_in_memory();
    let nodes = vec![make_node(
        NodeKind::Function,
        "f",
        "a.rs::fn::f",
        "a.rs",
        "rust",
    )];
    let edges = vec![make_edge(
        EdgeKind::Calls,
        "a.rs::fn::f",
        "b.rs::fn::g",
        "a.rs",
    )];
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &nodes, &edges)
        .unwrap();

    store.delete_file_graph("a.rs").unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.node_count, 0);
    assert_eq!(stats.edge_count, 0);
}

#[test]
fn delete_file_graph_removes_dangling_cross_file_edges() {
    let mut store = open_in_memory();
    // b.rs has an edge pointing INTO a node from a.rs.
    let na = make_node(NodeKind::Function, "fa", "a.rs::fn::fa", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "fb", "b.rs::fn::fb", "b.rs", "rust");
    // Edge lives in b.rs but targets a.rs::fn::fa.
    let cross_edge = make_edge(EdgeKind::Calls, "b.rs::fn::fb", "a.rs::fn::fa", "b.rs");
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph_for_repo("repo_test", "b.rs", "h", None, None, &[nb], &[cross_edge])
        .unwrap();

    // Verify the cross-file edge is present before deletion.
    assert_eq!(store.stats().unwrap().edge_count, 1);

    // Deleting a.rs must also remove the dangling edge from b.rs.
    store.delete_file_graph("a.rs").unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.file_count, 1, "b.rs file should remain");
    assert_eq!(
        stats.edge_count, 0,
        "dangling cross-file edge must be removed"
    );
}

#[test]
fn replace_file_graph_preserves_cross_file_edges_to_retained_nodes() {
    let mut store = open_in_memory();
    let original_target = make_node(
        NodeKind::Function,
        "target",
        "a.rs::fn::target",
        "a.rs",
        "rust",
    );
    let caller = make_node(
        NodeKind::Function,
        "caller",
        "b.rs::fn::caller",
        "b.rs",
        "rust",
    );
    let inbound = make_edge(
        EdgeKind::Calls,
        "b.rs::fn::caller",
        "a.rs::fn::target",
        "b.rs",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h1",
            None,
            None,
            &[original_target],
            &[],
        )
        .unwrap();
    store
        .replace_file_graph_for_repo("repo_test", "b.rs", "h1", None, None, &[caller], &[inbound])
        .unwrap();

    let updated_target = make_node(
        NodeKind::Function,
        "target",
        "a.rs::fn::target",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h2",
            None,
            None,
            &[updated_target],
            &[],
        )
        .unwrap();

    let caller_edges = store.edges_by_file("b.rs").unwrap();
    assert_eq!(caller_edges.len(), 1);
    assert_eq!(caller_edges[0].target_qn, "a.rs::fn::target");
}

#[test]
fn replace_file_graph_removes_stale_cross_file_edges_on_update() {
    let mut store = open_in_memory();
    // b.rs references old_fn from a.rs; after a.rs is re-indexed with only
    // new_fn the edge must be cleaned up.
    let na = make_node(
        NodeKind::Function,
        "old_fn",
        "a.rs::fn::old_fn",
        "a.rs",
        "rust",
    );
    let nb = make_node(
        NodeKind::Function,
        "caller",
        "b.rs::fn::caller",
        "b.rs",
        "rust",
    );
    let stale = make_edge(
        EdgeKind::Calls,
        "b.rs::fn::caller",
        "a.rs::fn::old_fn",
        "b.rs",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h1", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph_for_repo("repo_test", "b.rs", "h1", None, None, &[nb], &[stale])
        .unwrap();
    assert_eq!(store.stats().unwrap().edge_count, 1);

    // Re-index a.rs with a *different* function; old_fn is now gone.
    let new_na = make_node(
        NodeKind::Function,
        "new_fn",
        "a.rs::fn::new_fn",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h2", None, None, &[new_na], &[])
        .unwrap();

    // The stale edge from b.rs towards the now-gone old_fn must be removed.
    assert_eq!(
        store.stats().unwrap().edge_count,
        0,
        "stale cross-file edge must be cleaned up"
    );
}

// --- replace_batch -------------------------------------------------------

#[test]
fn replace_batch_processes_multiple_files() {
    let mut store = open_in_memory();
    let batch = vec![
        ParsedFile {
            path: "a.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "ha".to_string(),
            size: None,
            nodes: vec![make_node(
                NodeKind::Function,
                "a",
                "a.rs::fn::a",
                "a.rs",
                "rust",
            )],
            edges: vec![],
        },
        ParsedFile {
            path: "b.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "hb".to_string(),
            size: None,
            nodes: vec![make_node(
                NodeKind::Function,
                "b",
                "b.rs::fn::b",
                "b.rs",
                "rust",
            )],
            edges: vec![make_edge(
                EdgeKind::Calls,
                "b.rs::fn::b",
                "a.rs::fn::a",
                "b.rs",
            )],
        },
    ];
    store
        .replace_files_transactional_for_repo("repo_test", &batch)
        .unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.node_count, 2);
    assert_eq!(stats.edge_count, 1);
}
