use super::*;

#[test]
fn replace_file_graph_rolls_back_on_insert_error() {
    let mut store = open_in_memory();
    let original_node = make_node(
        NodeKind::Function,
        "stable",
        "a.rs::fn::stable",
        "a.rs",
        "rust",
    );
    let original_edge = make_edge(
        EdgeKind::Calls,
        "a.rs::fn::stable",
        "a.rs::fn::stable",
        "a.rs",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "old-hash",
            Some("rust"),
            Some(10),
            std::slice::from_ref(&original_node),
            std::slice::from_ref(&original_edge),
        )
        .unwrap();

    store
        .conn
        .execute_batch(
            "CREATE TEMP TRIGGER fail_replace_file_graph
             BEFORE INSERT ON nodes
             WHEN NEW.qualified_name = 'a.rs::fn::broken'
             BEGIN
                 SELECT RAISE(ABORT, 'simulated node insert failure');
             END;",
        )
        .unwrap();

    let err = store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "new-hash",
            Some("rust"),
            Some(20),
            &[make_node(
                NodeKind::Function,
                "broken",
                "a.rs::fn::broken",
                "a.rs",
                "rust",
            )],
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, AtlasError::Db(msg) if msg.contains("simulated node insert failure")));

    store
        .conn
        .execute_batch("DROP TRIGGER fail_replace_file_graph")
        .unwrap();

    let nodes = store.nodes_by_file("a.rs").unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].qualified_name, original_node.qualified_name);

    let edges = store.edges_by_file("a.rs").unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].source_qn, original_edge.source_qn);
    assert_eq!(edges[0].target_qn, original_edge.target_qn);

    let stored_hash: String = store
        .conn
        .query_row("SELECT hash FROM files WHERE path = 'a.rs'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(stored_hash, "old-hash");

    let results = store
        .search(&SearchQuery {
            text: "stable".to_string(),
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.qualified_name, "a.rs::fn::stable");
}

#[test]
fn replace_files_transactional_rolls_back_all_files_on_error() {
    let mut store = open_in_memory();
    store
        .conn
        .execute_batch(
            "CREATE TEMP TRIGGER fail_replace_files_transactional
             BEFORE INSERT ON nodes
             WHEN NEW.file_path = 'src/b.rs'
             BEGIN
                 SELECT RAISE(ABORT, 'simulated batch insert failure');
             END;",
        )
        .unwrap();

    let files = vec![
        ParsedFile {
            path: "src/a.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h1".to_string(),
            size: Some(10),
            nodes: vec![make_node(
                NodeKind::Function,
                "good",
                "src/a.rs::fn::good",
                "src/a.rs",
                "rust",
            )],
            edges: vec![],
        },
        ParsedFile {
            path: "src/b.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h2".to_string(),
            size: Some(20),
            nodes: vec![make_node(
                NodeKind::Function,
                "bad",
                "src/b.rs::fn::bad",
                "src/b.rs",
                "rust",
            )],
            edges: vec![],
        },
    ];

    let err = store
        .replace_files_transactional_for_repo("repo_test", &files)
        .unwrap_err();
    assert!(matches!(err, AtlasError::Db(msg) if msg.contains("simulated batch insert failure")));

    store
        .conn
        .execute_batch("DROP TRIGGER fail_replace_files_transactional")
        .unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.node_count, 0);
    assert_eq!(stats.edge_count, 0);
    assert!(store.nodes_by_file("src/a.rs").unwrap().is_empty());
    assert!(store.nodes_by_file("src/b.rs").unwrap().is_empty());
}

#[test]
fn replace_files_transactional_canonicalizes_path_identity() {
    let mut store = open_in_memory();
    let files = vec![ParsedFile {
        path: "./src/../src/lib.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "h1".to_string(),
        size: Some(10),
        nodes: vec![make_node(
            NodeKind::Function,
            "good",
            "src\\lib.rs::fn::good",
            "src\\lib.rs",
            "rust",
        )],
        edges: vec![make_edge(
            EdgeKind::Calls,
            "src\\lib.rs::fn::good",
            "external::fn::target",
            "src\\lib.rs",
        )],
    }];

    store
        .replace_files_transactional_for_repo("repo_test", &files)
        .unwrap();

    let nodes = store.nodes_by_file("src/lib.rs").unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].qualified_name, "src/lib.rs::fn::good");

    let edges = store.edges_by_file("src/lib.rs").unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].file_path, "src/lib.rs");
    assert_eq!(edges[0].source_qn, "src/lib.rs::fn::good");

    let files_count = store.stats().unwrap().file_count;
    assert_eq!(files_count, 1);
}

#[test]
#[cfg(target_os = "windows")]
fn replace_file_graph_uses_stable_windows_case_policy() {
    let mut store = open_in_memory();
    let nodes = vec![make_node(
        NodeKind::Function,
        "foo",
        "SRC\\Module.RS::fn::foo",
        "SRC\\Module.RS",
        "rust",
    )];

    store
        .replace_file_graph_for_repo(
            "repo_test",
            "SRC\\Module.RS",
            "hash1",
            Some("rust"),
            None,
            &nodes,
            &[],
        )
        .unwrap();

    let stored = store.nodes_by_file("src/module.rs").unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].file_path, "src/module.rs");
    assert_eq!(stored[0].qualified_name, "src/module.rs::fn::foo");
    assert_eq!(
        store.file_hash("src/module.rs").unwrap(),
        Some("hash1".to_string())
    );
}

#[test]
fn replace_file_graph_reports_lock_contention() {
    let (_dir, path, lock_holder) = open_file_backed();
    let mut blocked_writer = Store::open(&path).unwrap();
    lock_holder
        .conn
        .busy_timeout(Duration::from_millis(50))
        .unwrap();
    blocked_writer
        .conn
        .busy_timeout(Duration::from_millis(50))
        .unwrap();

    lock_holder.conn.execute_batch("BEGIN IMMEDIATE").unwrap();

    let err = blocked_writer
        .replace_file_graph_for_repo(
            "repo_test",
            "locked.rs",
            "h1",
            Some("rust"),
            Some(10),
            &[make_node(
                NodeKind::Function,
                "locked",
                "locked.rs::fn::locked",
                "locked.rs",
                "rust",
            )],
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, AtlasError::Db(msg) if msg.contains("locked")));

    lock_holder.conn.execute_batch("ROLLBACK").unwrap();
    blocked_writer
        .replace_file_graph_for_repo(
            "repo_test",
            "locked.rs",
            "h1",
            Some("rust"),
            Some(10),
            &[make_node(
                NodeKind::Function,
                "locked",
                "locked.rs::fn::locked",
                "locked.rs",
                "rust",
            )],
            &[],
        )
        .unwrap();
    assert_eq!(blocked_writer.nodes_by_file("locked.rs").unwrap().len(), 1);
}

#[test]
fn replace_files_transactional_reports_lock_contention() {
    let (_dir, path, lock_holder) = open_file_backed();
    let mut blocked_writer = Store::open(&path).unwrap();
    lock_holder
        .conn
        .busy_timeout(Duration::from_millis(50))
        .unwrap();
    blocked_writer
        .conn
        .busy_timeout(Duration::from_millis(50))
        .unwrap();

    lock_holder.conn.execute_batch("BEGIN IMMEDIATE").unwrap();

    let err = blocked_writer
        .replace_files_transactional_for_repo(
            "repo_test",
            &[ParsedFile {
                path: "locked.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h1".to_string(),
                size: Some(10),
                nodes: vec![make_node(
                    NodeKind::Function,
                    "locked_batch",
                    "locked.rs::fn::locked_batch",
                    "locked.rs",
                    "rust",
                )],
                edges: vec![],
            }],
        )
        .unwrap_err();
    assert!(matches!(err, AtlasError::Db(msg) if msg.contains("locked")));

    lock_holder.conn.execute_batch("ROLLBACK").unwrap();
    blocked_writer
        .replace_files_transactional_for_repo(
            "repo_test",
            &[ParsedFile {
                path: "locked.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h1".to_string(),
                size: Some(10),
                nodes: vec![make_node(
                    NodeKind::Function,
                    "locked_batch",
                    "locked.rs::fn::locked_batch",
                    "locked.rs",
                    "rust",
                )],
                edges: vec![],
            }],
        )
        .unwrap();
    assert_eq!(blocked_writer.nodes_by_file("locked.rs").unwrap().len(), 1);
}

// --- NodeId type ---------------------------------------------------------

#[test]
fn node_id_assigned_after_insert() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "fn_alpha",
        "a.rs::fn::fn_alpha",
        "a.rs",
        "rust",
    );
    assert_eq!(node.id, NodeId::UNSET, "before insert id must be UNSET");
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[node], &[])
        .unwrap();
    let fetched = store.nodes_by_file("a.rs").unwrap();
    assert_eq!(fetched.len(), 1);
    assert_ne!(
        fetched[0].id,
        NodeId::UNSET,
        "after insert id must be a real DB id"
    );
    assert!(fetched[0].id.0 > 0);
}

// -------------------------------------------------------------------------
// replace_files_transactional
// -------------------------------------------------------------------------

#[test]
fn replace_files_transactional_inserts_all_files() {
    let mut store = open_in_memory();
    let files = vec![
        ParsedFile {
            path: "src/a.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h1".to_string(),
            size: Some(100),
            nodes: vec![make_node(
                NodeKind::Function,
                "foo",
                "src/a.rs::fn::foo",
                "src/a.rs",
                "rust",
            )],
            edges: vec![],
        },
        ParsedFile {
            path: "src/b.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h2".to_string(),
            size: Some(200),
            nodes: vec![
                make_node(
                    NodeKind::Function,
                    "bar",
                    "src/b.rs::fn::bar",
                    "src/b.rs",
                    "rust",
                ),
                make_node(
                    NodeKind::Function,
                    "baz",
                    "src/b.rs::fn::baz",
                    "src/b.rs",
                    "rust",
                ),
            ],
            edges: vec![make_edge(
                EdgeKind::Calls,
                "src/b.rs::fn::baz",
                "src/b.rs::fn::bar",
                "src/b.rs",
            )],
        },
    ];

    let (total_nodes, total_edges) = store
        .replace_files_transactional_for_repo("repo_test", &files)
        .unwrap();
    assert_eq!(total_nodes, 3);
    assert_eq!(total_edges, 1);

    let stats = store.stats().unwrap();
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.node_count, 3);
    assert_eq!(stats.edge_count, 1);
}

#[test]
fn replace_files_transactional_empty_is_noop() {
    let mut store = open_in_memory();
    let (n, e) = store
        .replace_files_transactional_for_repo("repo_test", &[])
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(e, 0);
    assert_eq!(store.stats().unwrap().file_count, 0);
}

#[test]
fn replace_files_transactional_is_idempotent() {
    let mut store = open_in_memory();
    let files = vec![ParsedFile {
        path: "a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "h1".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "foo",
            "a.rs::fn::foo",
            "a.rs",
            "rust",
        )],
        edges: vec![],
    }];
    store
        .replace_files_transactional_for_repo("repo_test", &files)
        .unwrap();
    store
        .replace_files_transactional_for_repo("repo_test", &files)
        .unwrap();
    assert_eq!(store.stats().unwrap().node_count, 1);
}

// -------------------------------------------------------------------------
// node_signatures_by_file
// -------------------------------------------------------------------------
