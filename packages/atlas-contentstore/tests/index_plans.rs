//! Query-plan and schema regressions for the migration 011 index batch.

use atlas_contentstore::ContentStore;
use rusqlite::{Connection, ToSql};
use tempfile::TempDir;

fn migrated_db() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("context.db");
    let path = path.to_str().unwrap().to_string();
    let mut store = ContentStore::open(&path).unwrap();
    store.migrate().unwrap();
    drop(store);
    (dir, path)
}

fn explain(conn: &Connection, sql: &str, params: &[&dyn ToSql]) -> String {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare query plan");
    let rows = stmt
        .query_map(params, |row| row.get::<_, String>(3))
        .expect("run query plan");
    rows.map(|row| row.expect("plan row"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn index_names(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'index' AND sql IS NOT NULL AND name NOT LIKE 'sqlite_%'",
        )
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

#[test]
fn source_ordering_uses_created_at_indexes() {
    let (_dir, path) = migrated_db();
    let conn = Connection::open(&path).unwrap();

    let oldest = explain(
        &conn,
        "SELECT id FROM sources ORDER BY created_at ASC LIMIT 1",
        &[],
    );
    assert!(
        oldest.contains("idx_sources_created_at"),
        "expected idx_sources_created_at, got:\n{oldest}"
    );

    let scoped = explain(
        &conn,
        "SELECT id FROM sources WHERE session_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        &[&"sess", &10_i64],
    );
    assert!(
        scoped.contains("idx_sources_session_created"),
        "expected idx_sources_session_created, got:\n{scoped}"
    );
    assert!(
        !scoped.contains("TEMP B-TREE"),
        "session index must supply order, got:\n{scoped}"
    );
}

#[test]
fn chunk_lookups_use_unique_indexes() {
    let (_dir, path) = migrated_db();
    let conn = Connection::open(&path).unwrap();

    let by_source = explain(
        &conn,
        "SELECT chunk_id FROM chunks WHERE source_id = ?1",
        &[&"src"],
    );
    assert!(
        by_source.contains("idx_chunks_source_chunk_id") || by_source.contains("sqlite_autoindex"),
        "expected a source_id index, got:\n{by_source}"
    );

    let by_chunk = explain(
        &conn,
        "SELECT id FROM chunks WHERE source_id = ?1 AND chunk_id = ?2",
        &[&"src", &"chunk"],
    );
    assert!(
        by_chunk.contains("idx_chunks_source_chunk_id"),
        "expected idx_chunks_source_chunk_id, got:\n{by_chunk}"
    );
}

#[test]
fn migration_011_adds_hot_path_indexes() {
    let (_dir, path) = migrated_db();
    let conn = Connection::open(&path).unwrap();
    let indexes = index_names(&conn);
    for expected in [
        "idx_sources_created_at",
        "idx_sources_session_created",
        "idx_sources_session_agent",
    ] {
        assert!(
            indexes.iter().any(|name| name == expected),
            "missing expected index {expected}"
        );
    }
}

#[test]
fn migration_011_drops_redundant_and_unused_indexes() {
    let (_dir, path) = migrated_db();
    let conn = Connection::open(&path).unwrap();
    let indexes = index_names(&conn);
    for removed in [
        "idx_chunks_source_id",
        "idx_sources_session",
        "idx_chunks_chunk_id",
        "idx_sources_repo_id",
    ] {
        assert!(
            !indexes.iter().any(|name| name == removed),
            "index {removed} should be dropped by migration 011"
        );
    }
}
