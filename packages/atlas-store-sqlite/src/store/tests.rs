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

fn open_in_memory() -> Store {
    let conn = Connection::open_in_memory().unwrap();
    Store::apply_pragmas(&conn).unwrap();
    Store::register_regexp_udf(&conn).unwrap();
    let mut store = Store { conn };
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

#[test]
fn fts5_escape_preserves_safe_prefix_query() {
    assert_eq!(fts5_escape("gre* OR tw*"), "gre* OR tw*");
}

#[test]
fn fts5_escape_quotes_unsafe_query() {
    assert_eq!(fts5_escape("gre* OR tw*(foo)"), "\"gre* OR tw*(foo)\"");
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

// --- nodes_by_file / edges_by_file ---------------------------------------

#[test]
fn nodes_by_file_returns_only_that_file() {
    let mut store = open_in_memory();
    let na = make_node(NodeKind::Function, "fa", "a.rs::fn::fa", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "fb", "b.rs::fn::fb", "b.rs", "rust");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();

    let got = store.nodes_by_file("a.rs").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "fa");
}

#[test]
fn edges_by_file_returns_only_that_file() {
    let mut store = open_in_memory();
    let nodes_a = vec![make_node(
        NodeKind::Function,
        "fa",
        "a.rs::fn::fa",
        "a.rs",
        "rust",
    )];
    let nodes_b = vec![make_node(
        NodeKind::Function,
        "fb",
        "b.rs::fn::fb",
        "b.rs",
        "rust",
    )];
    let edges_a = vec![make_edge(
        EdgeKind::Calls,
        "a.rs::fn::fa",
        "b.rs::fn::fb",
        "a.rs",
    )];
    let edges_b = vec![make_edge(
        EdgeKind::Calls,
        "b.rs::fn::fb",
        "a.rs::fn::fa",
        "b.rs",
    )];
    store
        .replace_file_graph("a.rs", "h", None, None, &nodes_a, &edges_a)
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &nodes_b, &edges_b)
        .unwrap();

    let got = store.edges_by_file("a.rs").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].source_qn, "a.rs::fn::fa");
}

// --- find_dependents -----------------------------------------------------

#[test]
fn find_dependents_returns_importing_files() {
    let mut store = open_in_memory();
    // a.rs defines "Foo"; b.rs calls Foo.
    let na = make_node(NodeKind::Struct, "Foo", "a.rs::struct::Foo", "a.rs", "rust");
    let nb = make_node(
        NodeKind::Function,
        "use_foo",
        "b.rs::fn::use_foo",
        "b.rs",
        "rust",
    );
    let edge = make_edge(
        EdgeKind::References,
        "b.rs::fn::use_foo",
        "a.rs::struct::Foo",
        "b.rs",
    );
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[edge])
        .unwrap();

    let deps = store.find_dependents(&["a.rs"]).unwrap();
    assert!(deps.contains(&"b.rs".to_string()));
    assert!(!deps.contains(&"a.rs".to_string()));
}

#[test]
fn find_dependents_empty_input_returns_empty() {
    let store = open_in_memory();
    let deps = store.find_dependents(&[]).unwrap();
    assert!(deps.is_empty());
}

// --- impact_radius -------------------------------------------------------

#[test]
fn impact_radius_one_hop() {
    let mut store = open_in_memory();
    // a.rs::fn::a  →Calls→  b.rs::fn::b
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();

    let result = store.impact_radius(&["a.rs"], 3, 200).unwrap();
    assert_eq!(result.changed_nodes.len(), 1);
    assert!(
        result
            .impacted_nodes
            .iter()
            .any(|n| n.qualified_name == "b.rs::fn::b")
    );
    assert!(result.impacted_files.contains(&"b.rs".to_string()));
}

#[test]
fn impact_radius_cyclic_graph_terminates() {
    let mut store = open_in_memory();
    // a → b → a (cycle)
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let e2 = make_edge(EdgeKind::Calls, "b.rs::fn::b", "a.rs::fn::a", "b.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[e1])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[e2])
        .unwrap();

    // Must not loop forever and must return both nodes.
    let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
    let all_qns: Vec<&str> = result
        .changed_nodes
        .iter()
        .chain(result.impacted_nodes.iter())
        .map(|n| n.qualified_name.as_str())
        .collect();
    assert!(all_qns.contains(&"a.rs::fn::a"));
    assert!(all_qns.contains(&"b.rs::fn::b"));
}

#[test]
fn impact_radius_empty_input_returns_empty() {
    let store = open_in_memory();
    let result = store.impact_radius(&[], 3, 200).unwrap();
    assert!(result.changed_nodes.is_empty());
    assert!(result.impacted_nodes.is_empty());
}

#[test]
fn impact_radius_disconnected_graph() {
    let mut store = open_in_memory();
    // a.rs and c.rs exist but share no edges; b.rs is connected to a.rs.
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
    let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();
    store
        .replace_file_graph("c.rs", "h", None, None, &[nc], &[])
        .unwrap();

    let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
    // b.rs is reachable; c.rs is disconnected and must not appear.
    let all_qns: Vec<&str> = result
        .changed_nodes
        .iter()
        .chain(result.impacted_nodes.iter())
        .map(|n| n.qualified_name.as_str())
        .collect();
    assert!(
        all_qns.contains(&"a.rs::fn::a"),
        "seed node must be present"
    );
    assert!(
        all_qns.contains(&"b.rs::fn::b"),
        "connected node must be present"
    );
    assert!(
        !all_qns.contains(&"c.rs::fn::c"),
        "disconnected node must not appear"
    );
}

#[test]
fn impact_radius_depth_cap() {
    let mut store = open_in_memory();
    // Chain: a → b → c → d  (each hop is one depth unit)
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
    let nd = make_node(NodeKind::Function, "d", "d.rs::fn::d", "d.rs", "rust");
    let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let e2 = make_edge(EdgeKind::Calls, "b.rs::fn::b", "c.rs::fn::c", "b.rs");
    let e3 = make_edge(EdgeKind::Calls, "c.rs::fn::c", "d.rs::fn::d", "c.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[e1])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[e2])
        .unwrap();
    store
        .replace_file_graph("c.rs", "h", None, None, &[nc], &[e3])
        .unwrap();
    store
        .replace_file_graph("d.rs", "h", None, None, &[nd], &[])
        .unwrap();

    // max_depth=1: only b should be reachable beyond the seed.
    let result = store.impact_radius(&["a.rs"], 1, 200).unwrap();
    let impacted_qns: Vec<&str> = result
        .impacted_nodes
        .iter()
        .map(|n| n.qualified_name.as_str())
        .collect();
    assert!(
        impacted_qns.contains(&"b.rs::fn::b"),
        "one-hop node must be reachable at depth=1"
    );
    assert!(
        !impacted_qns.contains(&"c.rs::fn::c"),
        "two-hop node must not be reachable at depth=1"
    );
    assert!(
        !impacted_qns.contains(&"d.rs::fn::d"),
        "three-hop node must not be reachable at depth=1"
    );
}

#[test]
fn impact_radius_max_node_cap() {
    let mut store = open_in_memory();
    // Star topology: a.rs is the seed; b, c, d, e each called from a.
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nodes_src: Vec<(&str, &str, &str)> = vec![
        ("b", "b.rs::fn::b", "b.rs"),
        ("c", "c.rs::fn::c", "c.rs"),
        ("d", "d.rs::fn::d", "d.rs"),
        ("e", "e.rs::fn::e", "e.rs"),
    ];
    let mut edges: Vec<Edge> = nodes_src
        .iter()
        .map(|(_, qn, fp)| make_edge(EdgeKind::Calls, "a.rs::fn::a", qn, fp))
        .collect();
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &edges)
        .unwrap();
    // Clear so we can reuse this vec as a per-file edges slice.
    edges.clear();
    for (name, qn, fp) in &nodes_src {
        let n = make_node(NodeKind::Function, name, qn, fp, "rust");
        store
            .replace_file_graph(fp, "h", None, None, &[n], &[])
            .unwrap();
    }

    // Cap at 2 total nodes; seed a.rs has 1 node, so at most 1 impacted node
    // should be returned regardless of star size.
    let result = store.impact_radius(&["a.rs"], 5, 2).unwrap();
    let total = result.changed_nodes.len() + result.impacted_nodes.len();
    assert!(
        total <= 2,
        "total nodes must not exceed max_nodes cap; got {total}"
    );
}

#[test]
fn impact_radius_deleted_seed_file_returns_empty() {
    let mut store = open_in_memory();
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();

    // Delete the seed file before querying impact.
    store.delete_file_graph("a.rs").unwrap();

    let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
    assert!(
        result.changed_nodes.is_empty(),
        "deleted seed file must yield no changed nodes"
    );
    assert!(
        result.impacted_nodes.is_empty(),
        "deleted seed file must yield no impacted nodes"
    );
}

#[test]
fn impact_radius_seed_file_with_no_nodes() {
    let mut store = open_in_memory();
    // Index a file record with zero nodes.
    store
        .replace_file_graph("empty.rs", "h", None, None, &[], &[])
        .unwrap();

    let result = store.impact_radius(&["empty.rs"], 5, 200).unwrap();
    assert!(
        result.changed_nodes.is_empty(),
        "file with no nodes must yield no changed nodes"
    );
    assert!(
        result.impacted_nodes.is_empty(),
        "file with no nodes must yield no impacted nodes"
    );
    assert!(result.relevant_edges.is_empty());
}

// --- FTS search ----------------------------------------------------------

#[test]
fn fts_search_finds_indexed_node() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "replace_file_graph",
        "store.rs::fn::replace_file_graph",
        "store.rs",
        "rust",
    );
    store
        .replace_file_graph("store.rs", "h", Some("rust"), None, &[node], &[])
        .unwrap();

    let q = SearchQuery {
        text: "replace_file_graph".to_string(),
        limit: 5,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].node.name, "replace_file_graph");
}

#[test]
fn fts_search_empty_query_returns_empty() {
    let store = open_in_memory();
    let q = SearchQuery {
        text: "".to_string(),
        ..Default::default()
    };
    let err = store.search(&q).unwrap_err();
    assert!(
        err.to_string().contains("non-empty text or regex pattern"),
        "expected empty-query error, got: {err}"
    );
}

#[test]
fn fts_search_respects_kind_filter() {
    let mut store = open_in_memory();
    let func = make_node(
        NodeKind::Function,
        "process",
        "a.rs::fn::process",
        "a.rs",
        "rust",
    );
    let strct = make_node(
        NodeKind::Struct,
        "ProcessConfig",
        "a.rs::struct::ProcessConfig",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h", None, None, &[func, strct], &[])
        .unwrap();

    let q = SearchQuery {
        text: "process".to_string(),
        kind: Some("struct".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(
        results
            .iter()
            .all(|r| matches!(r.node.kind, NodeKind::Struct))
    );
}

#[test]
fn fts_search_not_found_after_delete() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "vanishing_fn",
        "a.rs::fn::vanishing_fn",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h", None, None, &[node], &[])
        .unwrap();
    store.delete_file_graph("a.rs").unwrap();

    let q = SearchQuery {
        text: "vanishing_fn".to_string(),
        limit: 5,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(results.is_empty());
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

// --- file_hashes ---------------------------------------------------------

#[test]
fn file_hashes_returns_stored_hashes() {
    let mut store = open_in_memory();
    let nodes_a = vec![make_node(
        NodeKind::Function,
        "f",
        "a.rs::fn::f",
        "a.rs",
        "rust",
    )];
    let nodes_b = vec![make_node(
        NodeKind::Function,
        "g",
        "b.rs::fn::g",
        "b.rs",
        "go",
    )];
    store
        .replace_file_graph("a.rs", "hash_aaa", Some("rust"), None, &nodes_a, &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "hash_bbb", Some("go"), None, &nodes_b, &[])
        .unwrap();

    let hashes = store.file_hashes().unwrap();
    assert_eq!(hashes.get("a.rs").map(String::as_str), Some("hash_aaa"));
    assert_eq!(hashes.get("b.rs").map(String::as_str), Some("hash_bbb"));
}

#[test]
fn file_hashes_empty_when_no_files() {
    let store = open_in_memory();
    let hashes = store.file_hashes().unwrap();
    assert!(hashes.is_empty());
}

#[test]
fn file_hashes_updated_after_replace() {
    let mut store = open_in_memory();
    let nodes = vec![make_node(
        NodeKind::Function,
        "f",
        "a.rs::fn::f",
        "a.rs",
        "rust",
    )];
    store
        .replace_file_graph("a.rs", "old_hash", None, None, &nodes, &[])
        .unwrap();
    store
        .replace_file_graph("a.rs", "new_hash", None, None, &nodes, &[])
        .unwrap();

    let hashes = store.file_hashes().unwrap();
    assert_eq!(hashes.get("a.rs").map(String::as_str), Some("new_hash"));
    assert_eq!(hashes.len(), 1);
}

// --- impact_radius limits ------------------------------------------------

#[test]
fn impact_radius_respects_depth_limit() {
    let mut store = open_in_memory();
    // a → b → c → d: with depth=1, only b should be reachable from a.
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
    let nd = make_node(NodeKind::Function, "d", "d.rs::fn::d", "d.rs", "rust");
    let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let e2 = make_edge(EdgeKind::Calls, "b.rs::fn::b", "c.rs::fn::c", "b.rs");
    let e3 = make_edge(EdgeKind::Calls, "c.rs::fn::c", "d.rs::fn::d", "c.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[e1])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[e2])
        .unwrap();
    store
        .replace_file_graph("c.rs", "h", None, None, &[nc], &[e3])
        .unwrap();
    store
        .replace_file_graph("d.rs", "h", None, None, &[nd], &[])
        .unwrap();

    let result = store.impact_radius(&["a.rs"], 1, 200).unwrap();
    let all_files: Vec<&str> = result
        .changed_nodes
        .iter()
        .chain(result.impacted_nodes.iter())
        .map(|n| n.file_path.as_str())
        .collect();
    // At depth 1 from a.rs, b.rs should be reachable but c.rs and d.rs should not.
    assert!(
        all_files.contains(&"b.rs"),
        "b.rs should be reached at depth 1"
    );
    assert!(
        !all_files.contains(&"d.rs"),
        "d.rs beyond depth limit should not appear"
    );
}

#[test]
fn impact_radius_respects_node_count_limit() {
    let mut store = open_in_memory();
    // Create a fan-out: a → b, a → c, a → d, a → e (4 targets)
    let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
    let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
    let nd = make_node(NodeKind::Function, "d", "d.rs::fn::d", "d.rs", "rust");
    let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
    let e2 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "c.rs::fn::c", "a.rs");
    let e3 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "d.rs::fn::d", "a.rs");
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[e1, e2, e3])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();
    store
        .replace_file_graph("c.rs", "h", None, None, &[nc], &[])
        .unwrap();
    store
        .replace_file_graph("d.rs", "h", None, None, &[nd], &[])
        .unwrap();

    // Limit to 2 total nodes — should stop before visiting all of b/c/d.
    let result = store.impact_radius(&["a.rs"], 5, 2).unwrap();
    let total = result.changed_nodes.len() + result.impacted_nodes.len();
    assert!(
        total <= 2,
        "node count limit must be respected; got {total}"
    );
}

// --- FTS language / file_path / is_test filters --------------------------

// --- regex post-filter tests ---------------------------------------------

#[test]
fn regex_matches_name() {
    let mut store = open_in_memory();
    let f1 = make_node(
        NodeKind::Function,
        "handle_request",
        "a.rs::fn::handle_request",
        "a.rs",
        "rust",
    );
    let f2 = make_node(
        NodeKind::Function,
        "parse_body",
        "a.rs::fn::parse_body",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h", None, None, &[f1, f2], &[])
        .unwrap();

    let q = SearchQuery {
        text: "handle".to_string(),
        regex_pattern: Some(r"handle_\w+".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty(), "expected at least one result");
    for r in &results {
        assert!(r.node.name.contains("handle") || r.node.qualified_name.contains("handle"));
    }
}

#[test]
fn regex_matches_qualified_name() {
    let mut store = open_in_memory();
    let f1 = make_node(
        NodeKind::Function,
        "foo",
        "pkg::service::foo",
        "a.rs",
        "rust",
    );
    let f2 = make_node(
        NodeKind::Function,
        "bar",
        "pkg::service::bar",
        "a.rs",
        "rust",
    );
    let f3 = make_node(NodeKind::Function, "baz", "pkg::util::baz", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h", None, None, &[f1, f2, f3], &[])
        .unwrap();

    // FTS text is empty → structural scan, regex filters qualified name
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"::service::".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 2);
    for r in &results {
        assert!(r.node.qualified_name.contains("::service::"));
    }
}

#[test]
fn regex_structural_scan_empty_text() {
    let mut store = open_in_memory();
    let f1 = make_node(
        NodeKind::Function,
        "fn_alpha",
        "a.rs::fn::fn_alpha",
        "a.rs",
        "rust",
    );
    let s1 = make_node(
        NodeKind::Struct,
        "MyStruct",
        "a.rs::struct::MyStruct",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h", None, None, &[f1, s1], &[])
        .unwrap();

    // regex matches only lower-case names starting with fn_
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^fn_".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.name, "fn_alpha");
}

#[test]
fn regex_invalid_pattern_returns_error() {
    let store = open_in_memory();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some("[invalid".to_string()),
        limit: 10,
        ..Default::default()
    };
    let err = store.search(&q).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("invalid regex"),
        "expected invalid regex message, got: {msg}"
    );
}

#[test]
fn regex_combined_with_fts_postfilters() {
    let mut store = open_in_memory();
    let f1 = make_node(
        NodeKind::Function,
        "search_fast",
        "a.rs::fn::search_fast",
        "a.rs",
        "rust",
    );
    let f2 = make_node(
        NodeKind::Function,
        "search_slow",
        "a.rs::fn::search_slow",
        "a.rs",
        "rust",
    );
    let f3 = make_node(
        NodeKind::Function,
        "other_fn",
        "a.rs::fn::other_fn",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h", None, None, &[f1, f2, f3], &[])
        .unwrap();

    let q = SearchQuery {
        text: "search".to_string(),
        regex_pattern: Some(r"search_fast".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.name, "search_fast");
}

#[test]
fn regex_none_empty_text_returns_empty() {
    let store = open_in_memory();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: None,
        limit: 10,
        ..Default::default()
    };
    let err = store.search(&q).unwrap_err();
    assert!(
        err.to_string().contains("non-empty text or regex pattern"),
        "expected empty-query error, got: {err}"
    );
}

// --- regex UDF comprehensive tests --------------------------------------

fn seed_regex_store() -> Store {
    let mut store = open_in_memory();
    // Deliberately varied names to exercise alternation, anchoring, case, multi-file.
    let nodes_a: Vec<Node> = vec![
        make_node(
            NodeKind::Function,
            "handle_request",
            "pkg::http::handle_request",
            "http.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "handle_response",
            "pkg::http::handle_response",
            "http.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "parse_body",
            "pkg::http::parse_body",
            "http.rs",
            "rust",
        ),
        make_node(
            NodeKind::Struct,
            "HttpClient",
            "pkg::http::HttpClient",
            "http.rs",
            "rust",
        ),
        make_node(
            NodeKind::Method,
            "send",
            "pkg::http::HttpClient::send",
            "http.rs",
            "rust",
        ),
    ];
    let nodes_b: Vec<Node> = vec![
        make_node(
            NodeKind::Function,
            "benchmark_context_retrieval_latency",
            "pkg::bench::benchmark_context_retrieval_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "benchmark_impact_analysis_latency",
            "pkg::bench::benchmark_impact_analysis_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "benchmark_dead_code_scan_latency",
            "pkg::bench::benchmark_dead_code_scan_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "benchmark_rename_planning_latency",
            "pkg::bench::benchmark_rename_planning_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "benchmark_import_cleanup_latency",
            "pkg::bench::benchmark_import_cleanup_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "setup_fixture",
            "pkg::bench::setup_fixture",
            "bench.rs",
            "rust",
        ),
    ];
    let nodes_c: Vec<Node> = vec![
        make_node(
            NodeKind::Function,
            "HANDLE_AUTH",
            "pkg::auth::HANDLE_AUTH",
            "auth.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "Handle_Login",
            "pkg::auth::Handle_Login",
            "auth.rs",
            "rust",
        ),
        make_node(
            NodeKind::Struct,
            "AuthService",
            "pkg::auth::AuthService",
            "auth.rs",
            "rust",
        ),
    ];
    store
        .replace_file_graph("http.rs", "h1", Some("rust"), None, &nodes_a, &[])
        .unwrap();
    store
        .replace_file_graph("bench.rs", "h2", Some("rust"), None, &nodes_b, &[])
        .unwrap();
    store
        .replace_file_graph("auth.rs", "h3", Some("rust"), None, &nodes_c, &[])
        .unwrap();
    store
}

#[test]
fn regex_udf_alternation_pipe_matches_multiple() {
    // Mirrors the motivating use-case: pipe-separated alternation in structural scan.
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"benchmark_context_retrieval_latency|benchmark_impact_analysis_latency|benchmark_dead_code_scan_latency|benchmark_rename_planning_latency|benchmark_import_cleanup_latency".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 5, "all five benchmark symbols should match");
    let names: Vec<&str> = results.iter().map(|r| r.node.name.as_str()).collect();
    assert!(names.contains(&"benchmark_context_retrieval_latency"));
    assert!(names.contains(&"benchmark_impact_analysis_latency"));
    assert!(names.contains(&"benchmark_dead_code_scan_latency"));
    assert!(names.contains(&"benchmark_rename_planning_latency"));
    assert!(names.contains(&"benchmark_import_cleanup_latency"));
    assert!(
        !names.contains(&"setup_fixture"),
        "non-matching node must not appear"
    );
}

#[test]
fn regex_udf_case_sensitive_distinguishes_variants() {
    // handle_request and HANDLE_AUTH and Handle_Login differ by case.
    let store = seed_regex_store();
    // Exact lowercase anchor — should not match HANDLE_AUTH or Handle_Login.
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^handle_".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.node.name.as_str()).collect();
    assert!(names.contains(&"handle_request"));
    assert!(names.contains(&"handle_response"));
    assert!(
        !names.contains(&"HANDLE_AUTH"),
        "uppercase must not match ^handle_"
    );
    assert!(
        !names.contains(&"Handle_Login"),
        "mixed-case must not match ^handle_"
    );
}

#[test]
fn regex_udf_case_insensitive_flag_matches_all_variants() {
    // (?i-u) inline flag (ASCII-only case fold) — should match all three case variants.
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"(?i-u)^handle_".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.node.name.as_str()).collect();
    assert!(names.contains(&"handle_request"));
    assert!(names.contains(&"handle_response"));
    assert!(names.contains(&"HANDLE_AUTH"));
    assert!(names.contains(&"Handle_Login"));
}

#[test]
fn regex_udf_anchored_end_matches_suffix() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"_latency$".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    // All five benchmark nodes end in _latency, nothing else does.
    assert_eq!(results.len(), 5);
    assert!(results.iter().all(|r| r.node.name.ends_with("_latency")));
}

#[test]
fn regex_udf_structural_scan_respects_kind_filter() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"pkg::".to_string()),
        kind: Some("struct".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert!(
        results
            .iter()
            .all(|r| matches!(r.node.kind, NodeKind::Struct))
    );
}

#[test]
fn regex_udf_structural_scan_respects_language_filter() {
    let mut store = seed_regex_store();
    let go_node = make_node(
        NodeKind::Function,
        "handle_request",
        "main::handle_request",
        "main.go",
        "go",
    );
    store
        .replace_file_graph("main.go", "h4", Some("go"), None, &[go_node], &[])
        .unwrap();

    // Restrict to go only — must not return rust handle_request.
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^handle_request$".to_string()),
        language: Some("go".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.language, "go");
}

#[test]
fn regex_udf_structural_scan_respects_subpath_filter() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"handle".to_string()),
        subpath: Some("http.rs".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    // Only http.rs has handle_* nodes in the store.
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.node.file_path == "http.rs"));
}

#[test]
fn regex_udf_limit_respected_in_structural_scan() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"pkg::".to_string()), // matches all 14 nodes
        limit: 3,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 3, "result count must not exceed limit");
}

#[test]
fn regex_udf_with_fts_alternation_in_text() {
    // Both text and regex set: FTS5 + UDF.
    let store = seed_regex_store();
    let q = SearchQuery {
        text: "handle".to_string(),
        regex_pattern: Some(r"^handle_re".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.node.name.as_str()).collect();
    assert!(names.contains(&"handle_request"));
    assert!(names.contains(&"handle_response"));
    assert!(
        !names.contains(&"HANDLE_AUTH"),
        "HANDLE_AUTH must not match ^handle_re"
    );
    assert!(
        !names.contains(&"Handle_Login"),
        "Handle_Login must not match ^handle_re"
    );
}

#[test]
fn regex_udf_with_fts_limit_respected() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: "benchmark".to_string(),
        regex_pattern: Some(r"benchmark_".to_string()),
        limit: 2,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(
        results.len() <= 2,
        "limit must be respected with FTS + UDF; got {}",
        results.len()
    );
}

#[test]
fn regex_udf_no_match_returns_empty_not_error() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^zzz_nonexistent_symbol_xyz$".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(
        results.is_empty(),
        "no match should return empty vec, not error"
    );
}

#[test]
fn regex_udf_dot_star_matches_all() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r".*".to_string()),
        limit: 100,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    // All 14 nodes inserted across 3 files.
    assert_eq!(results.len(), 14);
}

#[test]
fn regex_udf_empty_pattern_returns_error() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(String::new()),
        limit: 10,
        ..Default::default()
    };
    // Empty pattern is valid regex (matches everything) but text is also empty,
    // so either the UDF runs (returning all nodes up to limit) OR we could treat
    // this as a degenerate case. Assert it at least doesn't panic.
    let _ = store.search(&q);
}

#[test]
fn regex_udf_udf_not_leaked_between_queries() {
    // Two sequential searches with different patterns must not interfere.
    let store = seed_regex_store();
    let q1 = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^handle_request$".to_string()),
        limit: 20,
        ..Default::default()
    };
    let q2 = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^parse_body$".to_string()),
        limit: 20,
        ..Default::default()
    };
    let r1 = store.search(&q1).unwrap();
    let r2 = store.search(&q2).unwrap();
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0].node.name, "handle_request");
    assert_eq!(r2.len(), 1);
    assert_eq!(r2[0].node.name, "parse_body");
}

#[test]
fn regex_udf_complex_pattern_qualified_name_scope() {
    // Pattern anchored to qualified_name structure — pkg::bench:: prefix.
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^pkg::bench::".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 6); // 5 benchmarks + setup_fixture
    assert!(results.iter().all(|r| r.node.file_path == "bench.rs"));
}

// --- FTS language / file_path / is_test filters (existing) --------------

#[test]
fn fts_search_respects_language_filter() {
    let mut store = open_in_memory();
    let rust_fn = make_node(
        NodeKind::Function,
        "shared_name",
        "a.rs::fn::shared_name",
        "a.rs",
        "rust",
    );
    let go_fn = make_node(
        NodeKind::Function,
        "shared_name",
        "b.go::fn::shared_name",
        "b.go",
        "go",
    );
    store
        .replace_file_graph("a.rs", "h", Some("rust"), None, &[rust_fn], &[])
        .unwrap();
    store
        .replace_file_graph("b.go", "h", Some("go"), None, &[go_fn], &[])
        .unwrap();

    let q = SearchQuery {
        text: "shared_name".to_string(),
        language: Some("go".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.node.language == "go"));
}

#[test]
fn fts_search_respects_file_path_filter() {
    let mut store = open_in_memory();
    let na = make_node(
        NodeKind::Function,
        "common",
        "a.rs::fn::common",
        "a.rs",
        "rust",
    );
    let nb = make_node(
        NodeKind::Function,
        "common",
        "b.rs::fn::common",
        "b.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
        .unwrap();

    let q = SearchQuery {
        text: "common".to_string(),
        file_path: Some("a.rs".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.node.file_path == "a.rs"));
}

#[test]
fn fts_search_respects_is_test_filter() {
    let mut store = open_in_memory();
    let mut test_node = make_node(
        NodeKind::Function,
        "test_foo",
        "a.rs::fn::test_foo",
        "a.rs",
        "rust",
    );
    test_node.is_test = true;
    let prod_node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h", None, None, &[test_node, prod_node], &[])
        .unwrap();

    // Search for is_test = true should only return test nodes.
    let q = SearchQuery {
        text: "foo".to_string(),
        is_test: Some(true),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.node.is_test));
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
// find_dependents_for_qnames
// -------------------------------------------------------------------------

#[test]
fn find_dependents_for_qnames_returns_importers_of_changed_symbols() {
    let mut store = open_in_memory();

    // a.rs defines `foo`
    let node_a = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node_a], &[])
        .unwrap();

    // b.rs defines `bar` and calls a.rs::fn::foo
    let node_b = make_node(NodeKind::Function, "bar", "b.rs::fn::bar", "b.rs", "rust");
    let edge_b_to_a = make_edge(EdgeKind::Calls, "b.rs::fn::bar", "a.rs::fn::foo", "b.rs");
    store
        .replace_file_graph("b.rs", "h2", Some("rust"), None, &[node_b], &[edge_b_to_a])
        .unwrap();

    // c.rs defines `qux` with no edges
    let node_c = make_node(NodeKind::Function, "qux", "c.rs::fn::qux", "c.rs", "rust");
    store
        .replace_file_graph("c.rs", "h3", Some("rust"), None, &[node_c], &[])
        .unwrap();

    // Changing a.rs::fn::foo should only invalidate b.rs, not c.rs.
    let deps = store
        .find_dependents_for_qnames(&["a.rs::fn::foo"])
        .unwrap();
    assert_eq!(deps, vec!["b.rs"]);
}

#[test]
fn find_dependents_for_qnames_empty_input_returns_empty() {
    let store = open_in_memory();
    let deps = store.find_dependents_for_qnames(&[]).unwrap();
    assert!(deps.is_empty());
}

#[test]
fn find_dependents_for_qnames_no_edges_returns_empty() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node], &[])
        .unwrap();
    let deps = store
        .find_dependents_for_qnames(&["a.rs::fn::foo"])
        .unwrap();
    assert!(deps.is_empty());
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

// -------------------------------------------------------------------------
// Context engine helpers (Phase 22 Slice 2)
// -------------------------------------------------------------------------

fn setup_call_graph(store: &mut Store) {
    // a.rs: caller → b.rs: callee
    let caller = make_node(
        NodeKind::Function,
        "caller",
        "a.rs::fn::caller",
        "a.rs",
        "rust",
    );
    let callee = make_node(
        NodeKind::Function,
        "callee",
        "b.rs::fn::callee",
        "b.rs",
        "rust",
    );
    let edge = make_edge(
        EdgeKind::Calls,
        "a.rs::fn::caller",
        "b.rs::fn::callee",
        "a.rs",
    );
    store
        .replace_file_graph(
            "a.rs",
            "h1",
            Some("rust"),
            None,
            &[caller],
            std::slice::from_ref(&edge),
        )
        .unwrap();
    store
        .replace_file_graph("b.rs", "h2", Some("rust"), None, &[callee], &[])
        .unwrap();
}

// --- node_by_qname -------------------------------------------------------

#[test]
fn node_by_qname_exact_hit() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let node = store.node_by_qname("a.rs::fn::caller").unwrap();
    assert!(node.is_some());
    assert_eq!(node.unwrap().name, "caller");
}

#[test]
fn node_by_qname_missing_returns_none() {
    let store = open_in_memory();
    let node = store.node_by_qname("nonexistent::fn::missing").unwrap();
    assert!(node.is_none());
}

// --- nodes_by_name -------------------------------------------------------

#[test]
fn nodes_by_name_single_match() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let nodes = store.nodes_by_name("caller", 10).unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].qualified_name, "a.rs::fn::caller");
}

#[test]
fn nodes_by_name_multiple_matches() {
    let mut store = open_in_memory();
    let n1 = make_node(
        NodeKind::Function,
        "process",
        "a.rs::fn::process",
        "a.rs",
        "rust",
    );
    let n2 = make_node(
        NodeKind::Function,
        "process",
        "b.rs::fn::process",
        "b.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[n1], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h2", None, None, &[n2], &[])
        .unwrap();
    let nodes = store.nodes_by_name("process", 10).unwrap();
    assert_eq!(nodes.len(), 2);
}

#[test]
fn nodes_by_name_limit_respected() {
    let mut store = open_in_memory();
    let n1 = make_node(
        NodeKind::Function,
        "process",
        "a.rs::fn::process",
        "a.rs",
        "rust",
    );
    let n2 = make_node(
        NodeKind::Function,
        "process",
        "b.rs::fn::process",
        "b.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[n1], &[])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h2", None, None, &[n2], &[])
        .unwrap();
    let nodes = store.nodes_by_name("process", 1).unwrap();
    assert_eq!(nodes.len(), 1);
}

#[test]
fn nodes_by_name_missing_returns_empty() {
    let store = open_in_memory();
    let nodes = store.nodes_by_name("ghost_fn", 10).unwrap();
    assert!(nodes.is_empty());
}

// --- direct_callers -------------------------------------------------------

#[test]
fn direct_callers_returns_caller() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let callers = store.direct_callers("b.rs::fn::callee", 10).unwrap();
    assert_eq!(callers.len(), 1);
    let (node, edge) = &callers[0];
    assert_eq!(node.qualified_name, "a.rs::fn::caller");
    assert_eq!(edge.target_qn, "b.rs::fn::callee");
    assert_eq!(edge.kind, EdgeKind::Calls);
}

#[test]
fn direct_callers_empty_for_no_callers() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    // caller has no incoming call edges
    let callers = store.direct_callers("a.rs::fn::caller", 10).unwrap();
    assert!(callers.is_empty());
}

#[test]
fn direct_callers_missing_node_returns_empty() {
    let store = open_in_memory();
    let callers = store.direct_callers("does::not::exist", 10).unwrap();
    assert!(callers.is_empty());
}

// --- direct_callees -------------------------------------------------------

#[test]
fn direct_callees_returns_callee() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let callees = store.direct_callees("a.rs::fn::caller", 10).unwrap();
    assert_eq!(callees.len(), 1);
    let (node, edge) = &callees[0];
    assert_eq!(node.qualified_name, "b.rs::fn::callee");
    assert_eq!(edge.source_qn, "a.rs::fn::caller");
    assert_eq!(edge.kind, EdgeKind::Calls);
}

#[test]
fn direct_callees_empty_for_no_callees() {
    let mut store = open_in_memory();
    setup_call_graph(&mut store);
    let callees = store.direct_callees("b.rs::fn::callee", 10).unwrap();
    assert!(callees.is_empty());
}

#[test]
fn direct_callees_missing_node_returns_empty() {
    let store = open_in_memory();
    let callees = store.direct_callees("does::not::exist", 10).unwrap();
    assert!(callees.is_empty());
}

// --- import_neighbors ----------------------------------------------------

fn setup_import_graph(store: &mut Store) {
    let importer = make_node(
        NodeKind::Module,
        "mod_a",
        "a.rs::mod::mod_a",
        "a.rs",
        "rust",
    );
    let importee = make_node(
        NodeKind::Module,
        "mod_b",
        "b.rs::mod::mod_b",
        "b.rs",
        "rust",
    );
    let edge = make_edge(
        EdgeKind::Imports,
        "a.rs::mod::mod_a",
        "b.rs::mod::mod_b",
        "a.rs",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[importer], &[edge])
        .unwrap();
    store
        .replace_file_graph("b.rs", "h2", None, None, &[importee], &[])
        .unwrap();
}

#[test]
fn import_neighbors_forward_direction() {
    let mut store = open_in_memory();
    setup_import_graph(&mut store);
    // a imports b → b is a neighbor of a
    let neighbors = store.import_neighbors("a.rs::mod::mod_a", 10).unwrap();
    assert_eq!(neighbors.len(), 1);
    let (node, edge) = &neighbors[0];
    assert_eq!(node.qualified_name, "b.rs::mod::mod_b");
    assert_eq!(edge.kind, EdgeKind::Imports);
}

#[test]
fn import_neighbors_backward_direction() {
    let mut store = open_in_memory();
    setup_import_graph(&mut store);
    // b is imported by a → a is a neighbor of b
    let neighbors = store.import_neighbors("b.rs::mod::mod_b", 10).unwrap();
    assert_eq!(neighbors.len(), 1);
    let (node, _edge) = &neighbors[0];
    assert_eq!(node.qualified_name, "a.rs::mod::mod_a");
}

#[test]
fn import_neighbors_missing_node_returns_empty() {
    let store = open_in_memory();
    let neighbors = store.import_neighbors("no::such::module", 10).unwrap();
    assert!(neighbors.is_empty());
}

// --- containment_siblings ------------------------------------------------

fn setup_sibling_graph(store: &mut Store) {
    let parent_method = |name: &str, qn: &str| -> Node {
        let mut n = make_node(NodeKind::Method, name, qn, "a.rs", "rust");
        n.parent_name = Some("MyClass".to_string());
        n
    };
    let n1 = parent_method("method_a", "a.rs::MyClass::method_a");
    let n2 = parent_method("method_b", "a.rs::MyClass::method_b");
    let n3 = parent_method("method_c", "a.rs::MyClass::method_c");
    store
        .replace_file_graph("a.rs", "h1", None, None, &[n1, n2, n3], &[])
        .unwrap();
}

#[test]
fn containment_siblings_returns_same_parent() {
    let mut store = open_in_memory();
    setup_sibling_graph(&mut store);
    let siblings = store
        .containment_siblings("a.rs::MyClass::method_a", 10)
        .unwrap();
    assert_eq!(siblings.len(), 2);
    let names: Vec<_> = siblings.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"method_b"));
    assert!(names.contains(&"method_c"));
    // seed itself must be excluded
    assert!(!names.contains(&"method_a"));
}

#[test]
fn containment_siblings_limit_respected() {
    let mut store = open_in_memory();
    setup_sibling_graph(&mut store);
    let siblings = store
        .containment_siblings("a.rs::MyClass::method_a", 1)
        .unwrap();
    assert_eq!(siblings.len(), 1);
}

#[test]
fn containment_siblings_no_parent_returns_empty() {
    let mut store = open_in_memory();
    // node with no parent_name
    let n = make_node(
        NodeKind::Function,
        "standalone",
        "a.rs::fn::standalone",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[n], &[])
        .unwrap();
    let siblings = store
        .containment_siblings("a.rs::fn::standalone", 10)
        .unwrap();
    assert!(siblings.is_empty());
}

#[test]
fn containment_siblings_missing_node_returns_empty() {
    let store = open_in_memory();
    let siblings = store.containment_siblings("does::not::exist", 10).unwrap();
    assert!(siblings.is_empty());
}

// --- test_neighbors ------------------------------------------------------

fn setup_test_graph(store: &mut Store) {
    let src = make_node(
        NodeKind::Function,
        "parse",
        "a.rs::fn::parse",
        "a.rs",
        "rust",
    );
    let mut test_node = make_node(
        NodeKind::Test,
        "test_parse",
        "tests.rs::test::test_parse",
        "tests.rs",
        "rust",
    );
    test_node.is_test = true;
    let edge = make_edge(
        EdgeKind::Tests,
        "tests.rs::test::test_parse",
        "a.rs::fn::parse",
        "tests.rs",
    );
    store
        .replace_file_graph("a.rs", "h1", None, None, &[src], &[])
        .unwrap();
    store
        .replace_file_graph("tests.rs", "h2", None, None, &[test_node], &[edge])
        .unwrap();
}

#[test]
fn test_neighbors_source_node_finds_test() {
    let mut store = open_in_memory();
    setup_test_graph(&mut store);
    // parse is tested by test_parse → test_parse must appear
    let neighbors = store.test_neighbors("a.rs::fn::parse", 10).unwrap();
    assert_eq!(neighbors.len(), 1);
    let (node, edge) = &neighbors[0];
    assert_eq!(node.qualified_name, "tests.rs::test::test_parse");
    assert_eq!(edge.kind, EdgeKind::Tests);
}

#[test]
fn test_neighbors_test_node_finds_source() {
    let mut store = open_in_memory();
    setup_test_graph(&mut store);
    // test_parse tests parse → parse must appear
    let neighbors = store
        .test_neighbors("tests.rs::test::test_parse", 10)
        .unwrap();
    assert_eq!(neighbors.len(), 1);
    let (node, _edge) = &neighbors[0];
    assert_eq!(node.qualified_name, "a.rs::fn::parse");
}

#[test]
fn test_neighbors_missing_node_returns_empty() {
    let store = open_in_memory();
    let neighbors = store.test_neighbors("no::test::here", 10).unwrap();
    assert!(neighbors.is_empty());
}

// --- flows ---------------------------------------------------------------

#[test]
fn flows_create_list_delete() {
    let store = open_in_memory();
    assert!(store.list_flows().unwrap().is_empty());

    let id = store
        .create_flow("login", Some("auth"), Some("Login flow"))
        .unwrap();
    assert!(id > 0);

    let flows = store.list_flows().unwrap();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].name, "login");
    assert_eq!(flows[0].kind.as_deref(), Some("auth"));
    assert_eq!(flows[0].description.as_deref(), Some("Login flow"));

    store.delete_flow(id).unwrap();
    assert!(store.list_flows().unwrap().is_empty());
}

#[test]
fn flows_get_by_id_and_name() {
    let store = open_in_memory();
    let id = store.create_flow("checkout", None, None).unwrap();

    let by_id = store.get_flow(id).unwrap().expect("must exist");
    assert_eq!(by_id.name, "checkout");

    let by_name = store
        .get_flow_by_name("checkout")
        .unwrap()
        .expect("must exist");
    assert_eq!(by_name.id, id);

    assert!(store.get_flow(9999).unwrap().is_none());
    assert!(store.get_flow_by_name("missing").unwrap().is_none());
}

#[test]
fn flows_add_remove_member_lifecycle() {
    let store = open_in_memory();
    let flow_id = store.create_flow("pipeline", None, None).unwrap();

    store
        .add_flow_member(flow_id, "pkg::fn::step_a", Some(0), Some("entry"))
        .unwrap();
    store
        .add_flow_member(flow_id, "pkg::fn::step_b", Some(1), None)
        .unwrap();
    store
        .add_flow_member(flow_id, "pkg::fn::step_c", Some(2), None)
        .unwrap();

    let members = store.get_flow_members(flow_id).unwrap();
    assert_eq!(members.len(), 3);
    assert_eq!(members[0].node_qualified_name, "pkg::fn::step_a");
    assert_eq!(members[0].position, Some(0));
    assert_eq!(members[0].role.as_deref(), Some("entry"));
    assert_eq!(members[2].node_qualified_name, "pkg::fn::step_c");

    store
        .remove_flow_member(flow_id, "pkg::fn::step_b")
        .unwrap();
    let members = store.get_flow_members(flow_id).unwrap();
    assert_eq!(members.len(), 2);
    assert!(
        !members
            .iter()
            .any(|m| m.node_qualified_name == "pkg::fn::step_b")
    );
}

#[test]
fn flow_membership_survives_node_rebuild() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "step", "pkg::fn::step", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", None, None, std::slice::from_ref(&node), &[])
        .unwrap();

    let flow_id = store.create_flow("myflow", None, None).unwrap();
    store
        .add_flow_member(flow_id, "pkg::fn::step", Some(0), None)
        .unwrap();

    // Simulate `atlas build` re-indexing the same file.
    store
        .replace_file_graph("a.rs", "h2", None, None, &[node], &[])
        .unwrap();

    // Membership must still exist after rebuild.
    let members = store.get_flow_members(flow_id).unwrap();
    assert_eq!(members.len(), 1, "membership must survive node rebuild");
    assert_eq!(members[0].node_qualified_name, "pkg::fn::step");
}

#[test]
fn flow_add_member_update_idempotent() {
    let store = open_in_memory();
    let flow_id = store.create_flow("f", None, None).unwrap();
    store
        .add_flow_member(flow_id, "a::b", Some(0), Some("old"))
        .unwrap();
    // Replace with updated position/role.
    store
        .add_flow_member(flow_id, "a::b", Some(5), Some("new"))
        .unwrap();
    let members = store.get_flow_members(flow_id).unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].position, Some(5));
    assert_eq!(members[0].role.as_deref(), Some("new"));
}

#[test]
fn flows_for_node_returns_correct_flows() {
    let store = open_in_memory();
    let f1 = store.create_flow("flow1", None, None).unwrap();
    let f2 = store.create_flow("flow2", None, None).unwrap();
    store.add_flow_member(f1, "my::fn", None, None).unwrap();
    store.add_flow_member(f2, "my::fn", None, None).unwrap();
    store.add_flow_member(f2, "other::fn", None, None).unwrap();

    let flows = store.flows_for_node("my::fn").unwrap();
    assert_eq!(flows.len(), 2);

    let flows = store.flows_for_node("other::fn").unwrap();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].name, "flow2");

    let flows = store.flows_for_node("no::such::fn").unwrap();
    assert!(flows.is_empty());
}

#[test]
fn flow_delete_cascades_memberships() {
    let store = open_in_memory();
    let flow_id = store.create_flow("cascade-test", None, None).unwrap();
    store.add_flow_member(flow_id, "x::fn", None, None).unwrap();

    store.delete_flow(flow_id).unwrap();

    // Cascaded — querying members of deleted flow returns empty.
    let members = store.get_flow_members(flow_id).unwrap();
    assert!(members.is_empty());
}

// --- communities ---------------------------------------------------------

#[test]
fn communities_create_list_delete() {
    let store = open_in_memory();
    assert!(store.list_communities().unwrap().is_empty());

    let id = store
        .create_community("cluster-a", Some("louvain"), Some(0), None)
        .unwrap();
    assert!(id > 0);

    let comms = store.list_communities().unwrap();
    assert_eq!(comms.len(), 1);
    assert_eq!(comms[0].name, "cluster-a");
    assert_eq!(comms[0].algorithm.as_deref(), Some("louvain"));
    assert_eq!(comms[0].level, Some(0));

    store.delete_community(id).unwrap();
    assert!(store.list_communities().unwrap().is_empty());
}

#[test]
fn communities_get_by_id_and_name() {
    let store = open_in_memory();
    let id = store.create_community("c1", None, None, None).unwrap();

    let by_id = store.get_community(id).unwrap().expect("must exist");
    assert_eq!(by_id.name, "c1");

    let by_name = store
        .get_community_by_name("c1")
        .unwrap()
        .expect("must exist");
    assert_eq!(by_name.id, id);

    assert!(store.get_community(9999).unwrap().is_none());
    assert!(store.get_community_by_name("missing").unwrap().is_none());
}

#[test]
fn community_parent_child_relationship() {
    let store = open_in_memory();
    let parent_id = store
        .create_community("parent", Some("louvain"), Some(0), None)
        .unwrap();
    let child_id = store
        .create_community("child", Some("louvain"), Some(1), Some(parent_id))
        .unwrap();

    let child = store.get_community(child_id).unwrap().expect("must exist");
    assert_eq!(child.parent_community_id, Some(parent_id));
}

#[test]
fn community_add_remove_node_lifecycle() {
    let store = open_in_memory();
    let comm_id = store.create_community("grp", None, None, None).unwrap();

    store.add_community_node(comm_id, "a::fn").unwrap();
    store.add_community_node(comm_id, "b::struct").unwrap();
    store.add_community_node(comm_id, "c::method").unwrap();

    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert_eq!(nodes.len(), 3);
    // ordered by qname
    assert_eq!(nodes[0].node_qualified_name, "a::fn");
    assert_eq!(nodes[1].node_qualified_name, "b::struct");
    assert_eq!(nodes[2].node_qualified_name, "c::method");

    store.remove_community_node(comm_id, "b::struct").unwrap();
    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert_eq!(nodes.len(), 2);
    assert!(!nodes.iter().any(|n| n.node_qualified_name == "b::struct"));
}

#[test]
fn community_membership_survives_node_rebuild() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "fn_a", "pkg::fn::fn_a", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", None, None, std::slice::from_ref(&node), &[])
        .unwrap();

    let comm_id = store.create_community("mycomm", None, None, None).unwrap();
    store.add_community_node(comm_id, "pkg::fn::fn_a").unwrap();

    // Rebuild — simulates `atlas build`.
    store
        .replace_file_graph("a.rs", "h2", None, None, &[node], &[])
        .unwrap();

    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert_eq!(
        nodes.len(),
        1,
        "community membership must survive node rebuild"
    );
    assert_eq!(nodes[0].node_qualified_name, "pkg::fn::fn_a");
}

#[test]
fn community_add_node_idempotent() {
    let store = open_in_memory();
    let comm_id = store.create_community("idem", None, None, None).unwrap();
    store.add_community_node(comm_id, "x::fn").unwrap();
    store.add_community_node(comm_id, "x::fn").unwrap(); // duplicate — no error
    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert_eq!(nodes.len(), 1);
}

#[test]
fn communities_for_node_returns_correct() {
    let store = open_in_memory();
    let c1 = store.create_community("c1", None, None, None).unwrap();
    let c2 = store.create_community("c2", None, None, None).unwrap();
    store.add_community_node(c1, "shared::fn").unwrap();
    store.add_community_node(c2, "shared::fn").unwrap();
    store.add_community_node(c2, "only_c2::fn").unwrap();

    let comms = store.communities_for_node("shared::fn").unwrap();
    assert_eq!(comms.len(), 2);

    let comms = store.communities_for_node("only_c2::fn").unwrap();
    assert_eq!(comms.len(), 1);
    assert_eq!(comms[0].name, "c2");

    let comms = store.communities_for_node("nobody").unwrap();
    assert!(comms.is_empty());
}

#[test]
fn community_delete_cascades_nodes() {
    let store = open_in_memory();
    let comm_id = store.create_community("cascade", None, None, None).unwrap();
    store.add_community_node(comm_id, "x::fn").unwrap();

    store.delete_community(comm_id).unwrap();

    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert!(nodes.is_empty());
}

// ---------------------------------------------------------------------------
// Graph build lifecycle state tests
// ---------------------------------------------------------------------------

#[test]
fn begin_build_sets_state_building() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::Building);
    assert_eq!(status.nodes_written, 0);
    assert!(status.last_error.is_none());
}

#[test]
fn finish_build_after_begin_sets_built_with_counters() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    store
        .finish_build(
            "/repo",
            BuildFinishStats {
                files_discovered: 10,
                files_processed: 9,
                files_failed: 1,
                nodes_written: 50,
                edges_written: 30,
            },
        )
        .unwrap();
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::Built);
    assert_eq!(status.files_discovered, 10);
    assert_eq!(status.files_processed, 9);
    assert_eq!(status.files_failed, 1);
    assert_eq!(status.nodes_written, 50);
    assert_eq!(status.edges_written, 30);
    assert!(status.last_built_at.is_some());
    assert!(status.last_error.is_none());
}

#[test]
fn fail_build_after_begin_sets_build_failed_with_error() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    store.fail_build("/repo", "disk full").unwrap();
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::BuildFailed);
    assert_eq!(status.last_error.as_deref(), Some("disk full"));
}

#[test]
fn get_build_status_returns_none_when_no_row() {
    let store = open_in_memory();
    let result = store.get_build_status("/nonexistent").unwrap();
    assert!(result.is_none());
}

#[test]
fn list_build_statuses_returns_all_repos() {
    let store = open_in_memory();
    store.begin_build("/repo/a").unwrap();
    store.begin_build("/repo/b").unwrap();
    store
        .finish_build(
            "/repo/b",
            BuildFinishStats {
                files_discovered: 5,
                files_processed: 5,
                files_failed: 0,
                nodes_written: 20,
                edges_written: 10,
            },
        )
        .unwrap();
    let statuses = store.list_build_statuses().unwrap();
    assert_eq!(statuses.len(), 2);
    // Ordered by repo_root
    assert_eq!(statuses[0].repo_root, "/repo/a");
    assert_eq!(statuses[0].state, GraphBuildState::Building);
    assert_eq!(statuses[1].repo_root, "/repo/b");
    assert_eq!(statuses[1].state, GraphBuildState::Built);
}

#[test]
fn interrupted_build_state_stays_building() {
    // Simulate a crash: begin_build called but finish/fail never called.
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    // Reopen — state must still be 'building', detectable by doctor.
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::Building);
}

#[test]
fn counters_overwritten_on_repeated_finish() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    store
        .finish_build(
            "/repo",
            BuildFinishStats {
                files_discovered: 5,
                files_processed: 5,
                files_failed: 0,
                nodes_written: 10,
                edges_written: 5,
            },
        )
        .unwrap();
    // Second build run
    store.begin_build("/repo").unwrap();
    store
        .finish_build(
            "/repo",
            BuildFinishStats {
                files_discovered: 20,
                files_processed: 18,
                files_failed: 2,
                nodes_written: 80,
                edges_written: 40,
            },
        )
        .unwrap();
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.files_discovered, 20);
    assert_eq!(status.nodes_written, 80);
}
