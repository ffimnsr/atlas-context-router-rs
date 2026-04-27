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
fn migration_schema_matches_checked_in_schema_sql() {
    let store = open_in_memory();
    assert_eq!(
        normalize_schema_sql(&schema_dump(&store.conn)),
        normalize_schema_sql(include_str!("../../migrations/schema.sql"))
    );
}

#[test]
fn migration_schema_matches_versioned_schema_fixtures() {
    assert_eq!(MIGRATIONS.len(), 13, "add fixture when migration count changes");

    for migration in MIGRATIONS {
        let conn = open_unmigrated_in_memory();
        apply_migrations_through(&conn, migration.version);
        assert_eq!(
            schema_dump(&conn),
            schema_fixture(migration.version),
            "schema fixture mismatch at migration {}",
            migration.version
        );
    }
}

#[test]
fn migration_upgrades_every_historical_version_to_latest_schema() {
    let latest_version = MIGRATIONS.last().expect("latest migration").version;
    let latest_fixture = include_str!("../../migrations/schema.sql");

    for version in 0..latest_version {
        let conn = open_unmigrated_in_memory();
        apply_migrations_through(&conn, version);
        let mut store = Store {
            conn,
            _thread_bound: std::marker::PhantomData,
        };
        store.migrate().unwrap();
        assert_eq!(
            normalize_schema_sql(&schema_dump(&store.conn)),
            normalize_schema_sql(latest_fixture),
            "upgrade mismatch from schema version {}",
            version
        );
    }
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
fn migration_framework_records_history_and_provenance() {
    let store = open_in_memory();
    let history_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(history_count, MIGRATIONS.len() as i64);

    let mut stmt = store
        .conn
        .prepare("SELECT DISTINCT direction FROM schema_migrations ORDER BY direction")
        .unwrap();
    let directions = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(directions, vec!["up"]);

    let (db_kind, created_by, last_opened_by): (String, String, String) = store
        .conn
        .query_row(
            "SELECT db_kind, created_by, last_opened_by FROM atlas_provenance WHERE singleton_key = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(db_kind, "worldtree");
    assert_eq!(created_by, format!("atlas v{}", env!("CARGO_PKG_VERSION")));
    assert_eq!(last_opened_by, created_by);
}

#[test]
fn rollback_and_reupgrade_restore_latest_schema() {
    let mut store = open_in_memory();
    store.migrate_to(3).unwrap();
    let downgraded_version: i32 = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(downgraded_version, 3);
    assert!(!table_columns(&store.conn, "files").contains(&"owner_id".to_string()));
    let rollback_events: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE direction = 'down'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rollback_events, 10);

    store.migrate().unwrap();
    assert_eq!(
        normalize_schema_sql(&schema_dump(&store.conn)),
        normalize_schema_sql(include_str!("../../migrations/schema.sql"))
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
    atlas_db_utils::apply_atlas_pragmas(&conn).unwrap();
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

#[test]
fn integrity_check_reports_noncanonical_path_rows() {
    let store = open_in_memory();
    store
        .conn
        .execute(
            "INSERT INTO files (path, hash, language, indexed_at)
             VALUES (?1, 'h1', 'rust', '2025-01-01T00:00:00Z')",
            ["./src/lib.rs"],
        )
        .unwrap();

    let issues = store.integrity_check().expect("integrity_check should run");
    assert!(issues.iter().any(|issue| {
        issue.contains("noncanonical_path:")
            && issue.contains("table=files")
            && issue.contains("canonical=src/lib.rs")
    }));
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

// --- lock/retry behavior -------------------------------------------------
//
// Verifies that busy_timeout lets a write succeed even when a second
// connection holds the WAL write lock momentarily.  Two threads share the
// same file-backed DB; the blocker holds BEGIN IMMEDIATE for ~100 ms then
// rolls back.  The store write must succeed within the 5 s busy_timeout.

#[test]
fn write_succeeds_while_second_connection_holds_wal_write_lock() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    let (_dir, path, mut store) = open_file_backed();

    // Barrier: both threads start at the same time.
    let barrier = Arc::new(Barrier::new(2));
    let barrier2 = Arc::clone(&barrier);
    let path2 = path.clone();

    // Blocker thread: opens its own connection, holds a write transaction for
    // ~100 ms, then rolls back so the store's write can proceed.
    let blocker = thread::spawn(move || {
        let conn = rusqlite::Connection::open(&path2).unwrap();
        // Acquire the WAL write lock.
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        // Signal that the lock is held, then wait for the writer to start.
        barrier2.wait();
        thread::sleep(Duration::from_millis(100));
        conn.execute_batch("ROLLBACK").unwrap();
    });

    // Wait until the blocker has the write lock, then try to write.
    barrier.wait();

    let node = make_node(
        NodeKind::Function,
        "fn1",
        "lock.rs::fn::fn1",
        "lock.rs",
        "rust",
    );
    store
        .replace_file_graph("lock.rs", "h1", Some("rust"), None, &[node], &[])
        .expect("write must succeed within busy_timeout after lock is released");

    blocker.join().unwrap();

    // Confirm the node landed in the DB.
    let stats = store.stats().unwrap();
    assert_eq!(stats.node_count, 1);
}
