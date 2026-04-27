use std::collections::BTreeSet;
use std::time::Duration;

use atlas_core::{
    AtlasError, Edge, EdgeKind, Node, NodeId, NodeKind, PackageOwner, PackageOwnerKind, ParsedFile,
    SearchQuery,
};
use atlas_db_utils::set_user_version;
use rusqlite::Connection;

use crate::migrations::MIGRATIONS;

use super::helpers::fts5_escape;
use super::*;

mod build_state;
mod concurrency;
mod context;
mod graph;
mod helpers;
mod history;
mod lifecycle;
mod mutation;
mod postprocess;
mod search;
mod taxonomy;

fn open_in_memory() -> Store {
    let conn = Connection::open_in_memory().unwrap();
    atlas_db_utils::apply_atlas_pragmas(&conn).unwrap();
    Store::register_regexp_udf(&conn).unwrap();
    let mut store = Store {
        conn,
        _thread_bound: std::marker::PhantomData,
    };
    store.migrate().unwrap();
    store
}

fn open_unmigrated_in_memory() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    atlas_db_utils::apply_atlas_pragmas(&conn).unwrap();
    conn
}

fn open_file_backed() -> (tempfile::TempDir, String, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.sqlite");
    let path = db_path.to_str().unwrap().to_string();
    let store = Store::open(&path).unwrap();
    (dir, path, store)
}

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA table_info('{table}')");
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

fn schema_indexes(conn: &Connection) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(
            "SELECT name
             FROM sqlite_master
             WHERE type = 'index'
               AND sql IS NOT NULL
               AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .unwrap()
}

fn apply_migrations_through(conn: &Connection, version: i32) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS metadata (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )
    .unwrap();
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= version)
    {
        conn.execute_batch(migration.up_sql).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?1)",
            [migration.version.to_string()],
        )
        .unwrap();
    }
    set_user_version(conn, version).unwrap();
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn schema_dump(conn: &Connection) -> String {
    let user_version: i32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT type, name, sql
             FROM sqlite_master
             WHERE sql IS NOT NULL
             ORDER BY CASE type
                 WHEN 'table' THEN 0
                 WHEN 'index' THEN 1
                 WHEN 'trigger' THEN 2
                 WHEN 'view' THEN 3
                 ELSE 4
             END,
             name",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    let mut dump = vec![
        format!("-- schema_version: {user_version}"),
        format!("PRAGMA user_version = {user_version};"),
        String::new(),
    ];
    for (object_type, name, sql) in rows {
        dump.push(format!("-- {object_type}: {name}"));
        dump.push(format!("{};", normalize_schema_sql(&sql)));
        dump.push(String::new());
    }
    dump.join("\n").trim_end().to_string() + "\n"
}

fn schema_fixture(version: i32) -> &'static str {
    match version {
        1 => include_str!("../../migrations/schema_versions/001.sql"),
        2 => include_str!("../../migrations/schema_versions/002.sql"),
        3 => include_str!("../../migrations/schema_versions/003.sql"),
        4 => include_str!("../../migrations/schema_versions/004.sql"),
        5 => include_str!("../../migrations/schema_versions/005.sql"),
        6 => include_str!("../../migrations/schema_versions/006.sql"),
        7 => include_str!("../../migrations/schema_versions/007.sql"),
        8 => include_str!("../../migrations/schema_versions/008.sql"),
        9 => include_str!("../../migrations/schema_versions/009.sql"),
        10 => include_str!("../../migrations/schema_versions/010.sql"),
        11 => include_str!("../../migrations/schema_versions/011.sql"),
        12 => include_str!("../../migrations/schema_versions/012.sql"),
        13 => include_str!("../../migrations/schema_versions/013.sql"),
        _ => panic!("missing schema fixture for version {version}"),
    }
}

fn cols(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

fn make_node(kind: NodeKind, name: &str, qn: &str, file_path: &str, language: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_string(),
        qualified_name: qn.to_string(),
        file_path: file_path.to_string(),
        line_start: 1,
        line_end: 10,
        language: language.to_string(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "abc123".to_string(),
        extra_json: serde_json::Value::Null,
    }
}

fn make_edge(kind: EdgeKind, src: &str, tgt: &str, file_path: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: src.to_string(),
        target_qn: tgt.to_string(),
        file_path: file_path.to_string(),
        line: None,
        confidence: 1.0,
        confidence_tier: None,
        extra_json: serde_json::Value::Null,
    }
}
