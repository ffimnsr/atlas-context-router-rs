use super::*;

// --- replace_file_graph --------------------------------------------------

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
        .replace_file_graph("src/a.rs", "hash1", Some("rust"), Some(200), &nodes, &edges)
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
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &nodes, &edges)
        .unwrap();
    store
        .replace_file_graph("a.rs", "h2", Some("rust"), None, &nodes, &edges)
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
        .replace_file_graph("a.rs", "h1", None, None, &first, &[])
        .unwrap();

    let second = vec![make_node(
        NodeKind::Function,
        "new_fn",
        "a.rs::fn::new_fn",
        "a.rs",
        "rust",
    )];
    store
        .replace_file_graph("a.rs", "h2", None, None, &second, &[])
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
        .replace_file_graph(
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
        .replace_file_graph("a.rs", "h", None, None, &nodes, &edges)
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
        .replace_file_graph("a.rs", "h", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[cross_edge])
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
        .replace_file_graph("a.rs", "h1", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h1", None, None, &[nb], &[stale])
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
        .replace_file_graph("a.rs", "h2", None, None, &[new_na], &[])
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
    store.replace_batch(&batch).unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.node_count, 2);
    assert_eq!(stats.edge_count, 1);
}

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
        .replace_file_graph(
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
        .replace_file_graph(
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

    let err = store.replace_files_transactional(&files).unwrap_err();
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

    store.replace_files_transactional(&files).unwrap();

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
        .replace_file_graph("SRC\\Module.RS", "hash1", Some("rust"), None, &nodes, &[])
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
        .replace_file_graph(
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
        .replace_file_graph(
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
        .replace_files_transactional(&[ParsedFile {
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
        }])
        .unwrap_err();
    assert!(matches!(err, AtlasError::Db(msg) if msg.contains("locked")));

    lock_holder.conn.execute_batch("ROLLBACK").unwrap();
    blocked_writer
        .replace_files_transactional(&[ParsedFile {
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
        }])
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
        .replace_file_graph("a.rs", "h", None, None, &[node], &[])
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

    let (total_nodes, total_edges) = store.replace_files_transactional(&files).unwrap();
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
    let (n, e) = store.replace_files_transactional(&[]).unwrap();
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
    store.replace_files_transactional(&files).unwrap();
    store.replace_files_transactional(&files).unwrap();
    assert_eq!(store.stats().unwrap().node_count, 1);
}

// -------------------------------------------------------------------------
// node_signatures_by_file
// -------------------------------------------------------------------------

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
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &nodes, &[])
        .unwrap();

    let sigs = store.node_signatures_by_file("a.rs").unwrap();
    assert_eq!(sigs.len(), 2);
    assert!(sigs.contains_key("a.rs::fn::foo"));
    assert!(sigs.contains_key("a.rs::fn::bar"));
}

#[test]
fn node_signatures_stable_across_position_change() {
    let mut store = open_in_memory();
    let mut node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    node.line_start = 1;
    node.line_end = 5;
    store
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node.clone()], &[])
        .unwrap();
    let sigs_before = store.node_signatures_by_file("a.rs").unwrap();

    // Move the function to different lines — signature must be identical.
    let mut moved = node;
    moved.line_start = 100;
    moved.line_end = 110;
    store
        .replace_file_graph("a.rs", "h2", Some("rust"), None, &[moved], &[])
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

#[test]
fn file_owner_round_trips_cargo_owner() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "foo",
        "crates/foo/src/lib.rs::fn::foo",
        "crates/foo/src/lib.rs",
        "rust",
    );
    store
        .replace_file_graph(
            "crates/foo/src/lib.rs",
            "abc",
            Some("rust"),
            None,
            &[node],
            &[],
        )
        .unwrap();

    let owner = PackageOwner {
        owner_id: "cargo:crates/foo/Cargo.toml".to_owned(),
        kind: PackageOwnerKind::Cargo,
        root: "crates/foo".to_owned(),
        manifest_path: "crates/foo/Cargo.toml".to_owned(),
        package_name: Some("foo".to_owned()),
    };
    store
        .upsert_file_owner("crates/foo/src/lib.rs", Some(&owner))
        .unwrap();

    let stored = store
        .file_owner("crates/foo/src/lib.rs")
        .unwrap()
        .expect("owner");
    assert_eq!(stored.owner_id, "cargo:crates/foo/Cargo.toml");
    assert_eq!(stored.kind, PackageOwnerKind::Cargo);
    assert_eq!(stored.root, "crates/foo");
    assert_eq!(stored.manifest_path, "crates/foo/Cargo.toml");
    assert_eq!(stored.package_name.as_deref(), Some("foo"));
}

#[test]
fn file_owner_returns_none_when_not_set() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "bar",
        "scripts/run.py::fn::bar",
        "scripts/run.py",
        "python",
    );
    store
        .replace_file_graph("scripts/run.py", "ff", Some("python"), None, &[node], &[])
        .unwrap();
    // No upsert → owner should be None.
    assert_eq!(store.file_owner("scripts/run.py").unwrap(), None);
}

#[test]
fn file_owner_id_returns_id_string() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "go_fn",
        "lib/core/core.go::fn::go_fn",
        "lib/core/core.go",
        "go",
    );
    store
        .replace_file_graph("lib/core/core.go", "g1", Some("go"), None, &[node], &[])
        .unwrap();

    let owner = PackageOwner {
        owner_id: "go:lib/core/go.mod".to_owned(),
        kind: PackageOwnerKind::Go,
        root: "lib/core".to_owned(),
        manifest_path: "lib/core/go.mod".to_owned(),
        package_name: Some("example.com/core".to_owned()),
    };
    store
        .upsert_file_owner("lib/core/core.go", Some(&owner))
        .unwrap();

    assert_eq!(
        store.file_owner_id("lib/core/core.go").unwrap().as_deref(),
        Some("go:lib/core/go.mod")
    );
}

// file_hash
// -------------------------------------------------------------------------

#[test]
fn file_hash_returns_none_for_unknown_path() {
    let store = open_in_memory();
    assert_eq!(store.file_hash("nonexistent.rs").unwrap(), None);
}

#[test]
fn file_hash_returns_stored_hash() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "deadbeef", Some("rust"), None, &[node], &[])
        .unwrap();
    assert_eq!(
        store.file_hash("a.rs").unwrap(),
        Some("deadbeef".to_string())
    );
}

// -------------------------------------------------------------------------
// rename_file_graph
// -------------------------------------------------------------------------

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
        .replace_file_graph("old.rs", "h1", Some("rust"), None, &[node], &[edge])
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
fn rename_file_graph_preserves_node_ids() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node], &[])
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
        .replace_file_graph("old.rs", "h1", Some("rust"), None, &[node], &[])
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
