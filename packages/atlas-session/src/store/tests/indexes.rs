//! Query-plan and schema regressions for the migration 016 index batch.

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
fn overview_active_predicate_uses_supersedes_index() {
    let (_dir, store) = open_store(16, 1024);
    let plan = explain(
        &store.conn,
        "SELECT id FROM overview_records
         WHERE repo_root = ?1 AND subject_id = ?2 AND source_kind = ?3
           AND NOT EXISTS (
               SELECT 1 FROM overview_records newer
               WHERE newer.supersedes_id = overview_records.id
           )
         ORDER BY created_at DESC, id DESC LIMIT 1",
        &[&"/repo", &"src/lib.rs::fn::run", &"manual"],
    );
    assert!(
        plan.contains("idx_overview_records_supersedes"),
        "expected the supersedes index to resolve the active predicate, got:\n{plan}"
    );
    assert!(
        !plan.contains("SCAN newer"),
        "active predicate must not scan overview_records per row, got:\n{plan}"
    );
}

#[test]
fn overview_source_repair_uses_source_id_index() {
    let (_dir, store) = open_store(16, 1024);
    let plan = explain(
        &store.conn,
        "SELECT id, source_id FROM overview_records
         WHERE source_id IS NOT NULL AND source_id <> ''",
        &[],
    );
    assert!(
        plan.contains("idx_overview_records_source_id"),
        "expected idx_overview_records_source_id, got:\n{plan}"
    );
}

#[test]
fn feedback_recent_ordering_uses_index() {
    let (_dir, store) = open_store(16, 1024);
    let plan = explain(
        &store.conn,
        "SELECT id FROM feedback_records
         WHERE repo_root = ?1
         ORDER BY created_at DESC, id DESC
         LIMIT ?2",
        &[&"/repo", &10_i64],
    );
    assert!(
        plan.contains("idx_feedback_repo_created"),
        "expected idx_feedback_repo_created, got:\n{plan}"
    );
    assert!(
        !plan.contains("TEMP B-TREE"),
        "feedback index must supply order, got:\n{plan}"
    );
}

#[test]
fn experience_filter_and_prune_use_repo_indexes() {
    let (_dir, store) = open_store(16, 1024);
    let freshness = explain(
        &store.conn,
        "SELECT id FROM experience_edges WHERE repo_root = ?1 AND freshness = 'fresh'",
        &[&"/repo"],
    );
    assert!(
        freshness.contains("idx_experience_edges_repo_freshness"),
        "expected freshness index, got:\n{freshness}"
    );

    let validated = explain(
        &store.conn,
        "SELECT id FROM experience_edges
         WHERE repo_root = ?1 AND last_validated_at < ?2",
        &[&"/repo", &"2026-01-01"],
    );
    assert!(
        validated.contains("idx_experience_edges_repo_validated"),
        "expected validation-age index, got:\n{validated}"
    );
}

#[test]
fn memory_recall_lookups_use_repo_indexes() {
    let (_dir, store) = open_store(16, 1024);
    let active = explain(
        &store.conn,
        "SELECT COUNT(*) FROM memories
         WHERE repo_root = ?1 AND superseded_by IS NULL",
        &[&"/repo"],
    );
    assert!(
        active.contains("SEARCH memories USING INDEX idx_memories_repo_"),
        "expected a repo-scoped memory index, got:\n{active}"
    );
    assert!(
        !active.starts_with("SCAN memories"),
        "active-memory count must not full-scan memories, got:\n{active}"
    );

    let by_source = explain(
        &store.conn,
        "SELECT id FROM memories WHERE repo_root = ?1 AND source_id = ?2",
        &[&"/repo", &"src-1"],
    );
    assert!(
        by_source.contains("idx_memories_repo_source"),
        "expected source-id index, got:\n{by_source}"
    );
}

#[test]
fn durable_task_pagination_uses_cursor_index() {
    let (_dir, store) = open_store(16, 1024);
    let plan = explain(
        &store.conn,
        "SELECT task_id FROM durable_tasks
         ORDER BY updated_at DESC, created_at DESC, task_id DESC",
        &[],
    );
    assert!(
        plan.contains("idx_durable_tasks_updated_created_task"),
        "expected cursor pagination index, got:\n{plan}"
    );
    assert!(
        !plan.contains("TEMP B-TREE"),
        "cursor index must supply order, got:\n{plan}"
    );
}

#[test]
fn migration_016_adds_hot_path_indexes() {
    let (_dir, store) = open_store(16, 1024);
    let indexes = index_names(&store.conn);
    for expected in [
        "idx_overview_records_supersedes",
        "idx_overview_records_source_id",
        "idx_session_meta_updated_at",
        "idx_feedback_repo_created",
        "idx_experience_edges_repo_freshness",
        "idx_experience_edges_repo_validated",
        "idx_memoir_relations_repo_source",
        "idx_memoir_relations_repo_target",
        "idx_memories_repo_active",
        "idx_memories_repo_updated",
        "idx_memories_repo_source",
        "idx_durable_tasks_updated_created_task",
        "idx_session_events_event_type",
    ] {
        assert!(
            indexes.iter().any(|name| name == expected),
            "missing expected index {expected}"
        );
    }
}

#[test]
fn migration_016_drops_redundant_and_unused_indexes() {
    let (_dir, store) = open_store(16, 1024);
    let indexes = index_names(&store.conn);
    for removed in [
        "idx_memoir_concepts_graph",
        "idx_memoir_graphs_repo_scope",
        "idx_experience_edges_source",
        "idx_overview_records_repo",
        "idx_overview_records_commit",
        "idx_overview_records_file",
        "idx_overview_records_qname",
        "idx_overview_records_freshness",
        "idx_overview_records_subject_kind",
        "idx_overview_packets_repo",
        "idx_decision_memory_session_updated",
        "idx_memories_repo_accessed",
        "idx_memory_supersessions_new",
        "idx_durable_tasks_updated_task",
    ] {
        assert!(
            !indexes.iter().any(|name| name == removed),
            "index {removed} should be dropped by migration 016"
        );
    }
    assert!(
        indexes
            .iter()
            .any(|name| name == "idx_overview_records_subject"),
        "repo + subject overview lookups still need idx_overview_records_subject"
    );
}
