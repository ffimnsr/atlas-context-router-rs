use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use atlas_core::{
    AtlasError, Edge, EdgeKind, Node, NodeId, NodeKind, PackageOwner, PackageOwnerKind, ParsedFile,
    SearchQuery,
};
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
