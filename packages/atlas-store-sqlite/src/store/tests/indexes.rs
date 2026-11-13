//! Query-plan regressions for the migration 021 index batch.
//!
//! Each test binds parameters before explaining the plan: SQLite only applies
//! the case-insensitive `LIKE` range optimization once it can see the bound
//! pattern, which is exactly the state rusqlite prepares queries in.

use rusqlite::ToSql;

use super::*;

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

fn assert_uses_index(plan: &str, index: &str) {
    assert!(
        plan.contains(index),
        "expected plan to use {index}, got:\n{plan}"
    );
    assert!(
        !plan.contains("SCAN nodes\n") && !plan.starts_with("SCAN nodes"),
        "plan must not full-scan nodes, got:\n{plan}"
    );
}

#[test]
fn nodes_by_name_uses_name_order_index() {
    let store = open_in_memory();
    let plan = explain(
        &store.conn,
        "SELECT id, kind, name, qualified_name, file_path, line_start
         FROM nodes
         WHERE name = ?1
         ORDER BY file_path, line_start
         LIMIT ?2",
        &[&"compute", &20_i64],
    );
    assert_uses_index(&plan, "idx_nodes_name_order");
    assert!(
        !plan.contains("TEMP B-TREE"),
        "name index must supply file_path/line_start order, got:\n{plan}"
    );
}

#[test]
fn nodes_by_name_prefix_uses_nocase_index() {
    let store = open_in_memory();
    let plan = explain(
        &store.conn,
        "SELECT id, kind, name, qualified_name, file_path, line_start
         FROM nodes
         WHERE name LIKE ?1 ESCAPE '\\'
         ORDER BY length(name), name, file_path, line_start
         LIMIT ?2",
        &[&"comp%", &20_i64],
    );
    assert_uses_index(&plan, "idx_nodes_name_nocase");
}

#[test]
fn node_subpath_filter_uses_nocase_file_path_index() {
    let store = open_in_memory();
    let plan = explain(
        &store.conn,
        "SELECT id FROM nodes WHERE file_path LIKE ?1 ESCAPE '\\'",
        &[&"src/%"],
    );
    assert_uses_index(&plan, "idx_nodes_file_path_nocase");
}

#[test]
fn repo_scoped_subpath_filter_uses_repo_nocase_index() {
    let store = open_in_memory();
    let plan = explain(
        &store.conn,
        "SELECT id FROM nodes
         WHERE source_repo_id = ?1 AND file_path LIKE ?2 ESCAPE '\\'",
        &[&"repo", &"src/%"],
    );
    assert_uses_index(&plan, "idx_nodes_repo_file_path_nocase");
}

#[test]
fn unscoped_file_hash_lookup_uses_path_index() {
    let store = open_in_memory();
    let plan = explain(
        &store.conn,
        "SELECT hash FROM files WHERE path = ?1",
        &[&"src/main.rs"],
    );
    assert!(
        plan.contains("idx_files_path"),
        "expected idx_files_path, got:\n{plan}"
    );
}

#[test]
fn prefix_file_scan_uses_files_primary_key_range() {
    let store = open_in_memory();
    let plan = explain(
        &store.conn,
        "SELECT path FROM files
         WHERE source_repo_id = ?1 AND path >= ?2 AND path < ?3
         ORDER BY path",
        &[&"repo", &"src/", &"src0"],
    );
    assert!(
        plan.contains("sqlite_autoindex_files_1"),
        "expected the (source_repo_id, path) primary key range, got:\n{plan}"
    );
}

#[test]
fn recent_files_ordering_uses_indexed_at_index() {
    let store = open_in_memory();
    let plan = explain(
        &store.conn,
        "SELECT path FROM files ORDER BY indexed_at DESC LIMIT ?1",
        &[&10_i64],
    );
    assert!(
        plan.contains("idx_files_indexed_at"),
        "expected idx_files_indexed_at, got:\n{plan}"
    );
    assert!(
        !plan.contains("TEMP B-TREE"),
        "indexed_at index must supply order, got:\n{plan}"
    );
}

#[test]
fn ordered_edge_export_uses_edge_order_index() {
    let store = open_in_memory();
    let plan = explain(
        &store.conn,
        "SELECT id, kind, source_qualified, target_qualified, file_path
         FROM edges
         ORDER BY source_qualified, target_qualified, kind, file_path",
        &[],
    );
    assert!(
        plan.contains("idx_edges_order"),
        "expected idx_edges_order, got:\n{plan}"
    );
    assert!(
        !plan.contains("TEMP B-TREE"),
        "edge order index must supply order, got:\n{plan}"
    );
}

#[test]
fn migration_021_adds_hot_path_indexes() {
    let store = open_in_memory();
    let indexes = schema_indexes(&store.conn);
    for expected in [
        "idx_nodes_name_order",
        "idx_nodes_name_nocase",
        "idx_nodes_file_path_order",
        "idx_nodes_repo_file_path_order",
        "idx_nodes_file_path_nocase",
        "idx_nodes_repo_file_path_nocase",
        "idx_nodes_file_parent",
        "idx_files_path",
        "idx_files_indexed_at",
        "idx_edges_target_kind",
        "idx_edges_order",
        "idx_edges_repo_confidence_tier",
        "idx_commits_repo_author_time",
        "idx_retrieval_chunks_repo_embedded",
    ] {
        assert!(
            indexes.contains(expected),
            "missing expected index {expected}"
        );
    }
}

#[test]
fn migration_021_drops_redundant_and_unused_indexes() {
    let store = open_in_memory();
    let indexes = schema_indexes(&store.conn);
    for removed in [
        "idx_files_source_repo_path",
        "idx_repos_root_path",
        "idx_graph_snapshots_repo_id",
        "idx_graph_snapshots_commit_sha",
        "idx_historical_nodes_file_hash",
        "idx_historical_edges_file_hash",
        "idx_snapshot_files_snapshot_id",
        "idx_snapshot_edges_snapshot_id",
        "idx_snapshot_nodes_snapshot_id",
        "idx_snapshot_membership_blobs_snapshot_id",
        "idx_snapshot_membership_blobs_file_hash",
        "idx_snapshot_nodes_file_hash",
        "idx_snapshot_edges_file_hash",
        "idx_node_history_repo_id",
        "idx_node_history_qualified_name",
        "idx_node_history_introduction_commit",
        "idx_node_history_removal_commit",
        "idx_edge_history_repo_id",
        "idx_edge_history_source_qn",
        "idx_edge_history_target_qn",
        "idx_edge_history_introduction_commit",
        "idx_edge_history_removal_commit",
        "idx_commits_indexed_ref",
        "idx_commits_author_time",
        "idx_commits_committer_time",
        "idx_flows_kind",
        "idx_communities_algorithm",
        "idx_communities_parent",
        "idx_postprocess_state_state",
        "idx_postprocess_state_updated_at_ms",
        "idx_edges_kind",
        "idx_edges_source",
        "idx_edges_target",
        "idx_nodes_file_path",
        "idx_nodes_source_repo_file_path",
        "idx_commits_repo_id",
    ] {
        assert!(
            !indexes.contains(removed),
            "index {removed} should be dropped by migration 021"
        );
    }
}
