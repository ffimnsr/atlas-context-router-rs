use super::*;

// --- existing foundation tests -------------------------------------------

#[test]
fn migration_creates_schema() {
    let store = open_in_memory();
    let stats = store.stats().unwrap();
    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.node_count, 0);
    assert_eq!(stats.edge_count, 0);
}

#[test]
fn migration_creates_optional_flow_tables() {
    let store = open_in_memory();
    let mut stmt = store
        .conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name IN ('flows', 'flow_memberships', 'communities')
             ORDER BY name",
        )
        .unwrap();
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(names, vec!["communities", "flow_memberships", "flows"]);
}

#[test]
fn migration_schema_matches_golden_layout() {
    let store = open_in_memory();
    let expected_columns = BTreeMap::from([
        ("metadata".to_string(), cols(&["key", "value"])),
        (
            "files".to_string(),
            cols(&[
                "path",
                "language",
                "hash",
                "size",
                "indexed_at",
                "owner_id",
                "owner_kind",
                "owner_root",
                "owner_manifest_path",
                "owner_name",
            ]),
        ),
        (
            "nodes".to_string(),
            cols(&[
                "id",
                "kind",
                "name",
                "qualified_name",
                "file_path",
                "line_start",
                "line_end",
                "language",
                "parent_name",
                "params",
                "return_type",
                "modifiers",
                "is_test",
                "file_hash",
                "extra_json",
            ]),
        ),
        (
            "edges".to_string(),
            cols(&[
                "id",
                "kind",
                "source_qualified",
                "target_qualified",
                "file_path",
                "line",
                "confidence",
                "confidence_tier",
                "extra_json",
            ]),
        ),
        (
            "nodes_fts".to_string(),
            cols(&[
                "qualified_name",
                "name",
                "kind",
                "file_path",
                "language",
                "params",
                "return_type",
                "modifiers",
            ]),
        ),
        (
            "flows".to_string(),
            cols(&[
                "id",
                "name",
                "kind",
                "description",
                "extra_json",
                "created_at",
                "updated_at",
            ]),
        ),
        (
            "flow_memberships".to_string(),
            cols(&[
                "flow_id",
                "node_qualified_name",
                "position",
                "role",
                "extra_json",
            ]),
        ),
        (
            "communities".to_string(),
            cols(&[
                "id",
                "name",
                "algorithm",
                "level",
                "parent_community_id",
                "extra_json",
                "created_at",
                "updated_at",
            ]),
        ),
        (
            "retrieval_chunks".to_string(),
            cols(&["id", "node_qn", "chunk_idx", "text", "embedding"]),
        ),
        (
            "community_nodes".to_string(),
            cols(&["community_id", "node_qualified_name"]),
        ),
    ]);

    let actual_columns = expected_columns
        .keys()
        .map(|table| ((*table).to_string(), table_columns(&store.conn, table)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual_columns, expected_columns);

    let expected_indexes = BTreeSet::from([
        "idx_chunks_has_embedding".to_string(),
        "idx_chunks_node_qn".to_string(),
        "idx_communities_algorithm".to_string(),
        "idx_files_owner_id".to_string(),
        "idx_communities_parent".to_string(),
        "idx_community_nodes_node_qn".to_string(),
        "idx_edges_file_path".to_string(),
        "idx_edges_kind".to_string(),
        "idx_edges_source".to_string(),
        "idx_edges_target".to_string(),
        "idx_flow_memberships_flow_position".to_string(),
        "idx_flow_memberships_node_qualified_name".to_string(),
        "idx_flows_kind".to_string(),
        "idx_nodes_file_path".to_string(),
        "idx_nodes_kind".to_string(),
        "idx_nodes_language".to_string(),
        "idx_nodes_qualified_name".to_string(),
    ]);
    assert_eq!(schema_indexes(&store.conn), expected_indexes);

    let nodes_fts_sql: String = store
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'nodes_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(nodes_fts_sql.contains("USING fts5"));
    assert!(nodes_fts_sql.contains("content='nodes'"));
    assert!(nodes_fts_sql.contains("content_rowid='id'"));
}

#[test]
fn schema_version_stored() {
    let store = open_in_memory();
    let version: i32 = store
        .conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        version,
        MIGRATIONS.last().expect("latest migration").version
    );
}

#[test]
fn wal_mode_enabled() {
    let store = open_in_memory();
    let mode: String = store
        .conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    // In-memory databases report "memory" regardless; file DBs report "wal".
    assert!(!mode.is_empty());
}

#[test]
fn wal_mode_enabled_on_file_db() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    Store::apply_pragmas(&conn).unwrap();
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal", "file DB must use WAL journal mode");
}

// --- stats correctness ---------------------------------------------------

#[test]
fn stats_returns_nodes_by_kind() {
    let mut store = open_in_memory();
    let func = make_node(NodeKind::Function, "fn1", "a.rs::fn::fn1", "a.rs", "rust");
    let strct = make_node(
        NodeKind::Struct,
        "MyStruct",
        "a.rs::struct::MyStruct",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h", Some("rust"), None, &[func, strct], &[])
        .unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.node_count, 2);
    assert!(stats.nodes_by_kind.iter().any(|(k, _)| k == "function"));
    assert!(stats.nodes_by_kind.iter().any(|(k, _)| k == "struct"));
}

#[test]
fn stats_returns_languages() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "fn1", "a.rs::fn::fn1", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h", Some("rust"), None, &[node], &[])
        .unwrap();

    let stats = store.stats().unwrap();
    assert!(stats.languages.contains(&"rust".to_string()));
}

#[test]
fn stats_last_indexed_at_set_after_replace() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "fn1", "a.rs::fn::fn1", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h", Some("rust"), None, &[node], &[])
        .unwrap();

    let stats = store.stats().unwrap();
    assert!(
        stats.last_indexed_at.is_some(),
        "last_indexed_at should be set"
    );
}

#[test]
fn integrity_check_clean_db() {
    let store = open_in_memory();
    let issues = store
        .integrity_check()
        .expect("integrity_check should not error");
    assert!(
        issues.is_empty(),
        "fresh in-memory DB should have no issues: {issues:?}"
    );
}

#[test]
fn integrity_check_after_writes() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node], &[])
        .unwrap();
    let issues = store
        .integrity_check()
        .expect("integrity_check should not error");
    assert!(
        issues.is_empty(),
        "DB with data should still pass integrity check: {issues:?}"
    );
}

// --- orphan-node query regression ----------------------------------------
//
// Ensures orphan_nodes() uses the correct edge schema column names
// (source_qualified / target_qualified), not stale aliases like source_qn.

#[test]
fn orphan_nodes_returns_isolated_nodes() {
    let mut store = open_in_memory();
    let isolated = make_node(NodeKind::Function, "lone", "a.rs::fn::lone", "a.rs", "rust");
    let connected_a = make_node(
        NodeKind::Function,
        "caller",
        "a.rs::fn::caller",
        "a.rs",
        "rust",
    );
    let connected_b = make_node(
        NodeKind::Function,
        "callee",
        "a.rs::fn::callee",
        "a.rs",
        "rust",
    );
    let edge = make_edge(
        EdgeKind::Calls,
        "a.rs::fn::caller",
        "a.rs::fn::callee",
        "a.rs",
    );
    store
        .replace_file_graph(
            "a.rs",
            "h",
            Some("rust"),
            None,
            &[isolated.clone(), connected_a, connected_b],
            &[edge],
        )
        .unwrap();

    let orphans = store
        .orphan_nodes(100)
        .expect("orphan_nodes must not error");
    assert_eq!(orphans.len(), 1, "only the lone node should be an orphan");
    assert_eq!(orphans[0].qualified_name, "a.rs::fn::lone");
}

#[test]
fn orphan_nodes_empty_when_all_nodes_connected() {
    let mut store = open_in_memory();
    let a = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let b = make_node(NodeKind::Function, "b", "a.rs::fn::b", "a.rs", "rust");
    let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "a.rs::fn::b", "a.rs");
    store
        .replace_file_graph("a.rs", "h", Some("rust"), None, &[a, b], &[edge])
        .unwrap();

    let orphans = store
        .orphan_nodes(100)
        .expect("orphan_nodes must not error");
    assert!(
        orphans.is_empty(),
        "no orphans expected when all nodes have edges"
    );
}

#[test]
fn orphan_nodes_all_when_no_edges() {
    let mut store = open_in_memory();
    let a = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let b = make_node(NodeKind::Function, "b", "a.rs::fn::b", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h", Some("rust"), None, &[a, b], &[])
        .unwrap();

    let orphans = store
        .orphan_nodes(100)
        .expect("orphan_nodes must not error");
    assert_eq!(
        orphans.len(),
        2,
        "all nodes are orphans when there are no edges"
    );
}
