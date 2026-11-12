use super::*;

use crate::store::memory_consolidate::{
    overflow_source_id, route_overflow_body, synthesize_consolidated_body,
};
use crate::store::types::OVERSIZED_BODY_CHARS;

fn record(id: &str, created_at: &str, body: &str, repo_root: &str) -> MemoryRecord {
    MemoryRecord {
        id: id.to_owned(),
        repo_root: repo_root.to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: "deploy".to_owned(),
        title: "Deploy".to_owned(),
        body: body.to_owned(),
        importance: MemoryImportance::Normal,
        created_at: created_at.to_owned(),
        updated_at: created_at.to_owned(),
        last_accessed_at: created_at.to_owned(),
        decay_score: 0.0,
        source_id: None,
        metadata: serde_json::json!({}),
        superseded_by: None,
    }
}

#[test]
fn synthesis_keeps_unique_facts_and_chronological_order() {
    let merged = synthesize_consolidated_body(&[
        record("b", "2026-01-02T00:00:00Z", "second. shared.", "/repo"),
        record("a", "2026-01-01T00:00:00Z", "first. Shared!", "/repo"),
    ]);
    assert_eq!(merged.body, "first.\nShared!\nsecond.");
    assert_eq!(merged.distinct_units, 3);
    assert!(!merged.overflow);
}

#[test]
fn consolidation_apply_keeps_unique_member_facts_and_provenance() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m1",
        "2026-01-02T00:00:00Z",
        "deploy uses helm",
        "deploy",
        MemoryImportance::High,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m2",
        "2026-01-01T00:00:00Z",
        "rollback via helm",
        "deploy",
        MemoryImportance::Low,
        MemoryScope::Project,
    );

    // Give both rows the same title so they group deterministically.
    store
        .conn
        .execute("UPDATE memories SET title = 'Deploy process'", [])
        .unwrap();

    let plan = crate::store::memory::consolidation_plan(
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        false,
        None,
    )
    .unwrap();
    assert_eq!(plan.groups.len(), 1);
    let group = &plan.groups[0];
    assert!(group.body.contains("deploy uses helm"));
    assert!(group.body.contains("rollback via helm"));
    assert!(group.metadata_json.contains("merged_source_ids"));
    assert_eq!(group.merged_source_ids, vec!["src-m1", "src-m2"]);
    assert!(group.overflow_source_id.is_none());

    let consolidated = store
        .get_memory("/repo", group.consolidated_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert!(consolidated.body.contains("deploy uses helm"));
    assert!(consolidated.body.contains("rollback via helm"));
}

#[test]
fn oversized_consolidation_routes_full_body_to_context_db() {
    let (dir, store) = open_store(16, 1024);
    let repo = dir.path().to_str().unwrap();
    let long_a = format!("{} alpha", "a".repeat(OVERSIZED_BODY_CHARS));
    let long_b = format!("{} beta", "b".repeat(OVERSIZED_BODY_CHARS));
    seed_memory_full(
        &store,
        "m1",
        "2026-01-02T00:00:00Z",
        &long_a,
        "deploy",
        MemoryImportance::Normal,
        MemoryScope::Project,
        None,
        None,
        Some("src-a"),
    );
    seed_memory_full(
        &store,
        "m2",
        "2026-01-01T00:00:00Z",
        &long_b,
        "deploy",
        MemoryImportance::Normal,
        MemoryScope::Project,
        None,
        None,
        Some("src-b"),
    );
    store
        .conn
        .execute("UPDATE memories SET title = 'Deploy process'", [])
        .unwrap();
    store
        .conn
        .execute("UPDATE memories SET repo_root = ?1", [repo])
        .unwrap();

    let mut stored = Vec::new();
    let plan = {
        let mut sink =
            |source_id: &str, session_id: Option<&str>, body: &str| -> atlas_core::Result<()> {
                stored.push((
                    source_id.to_owned(),
                    session_id.map(str::to_owned),
                    body.to_owned(),
                ));
                Ok(())
            };
        store
            .consolidate_memories_with_overflow(
                repo,
                &MemoryListFilter::default(),
                false,
                &mut sink,
            )
            .unwrap()
    };
    let group = &plan.groups[0];
    let source_id = group
        .overflow_source_id
        .as_deref()
        .expect("overflow routed");
    assert!(group.body.chars().count() <= OVERSIZED_BODY_CHARS);
    assert!(group.metadata_json.contains(source_id));

    let mutual_ids = vec!["m1".to_owned(), "m2".to_owned()];
    assert_eq!(
        source_id,
        overflow_source_id(repo, &mutual_ids),
        "overflow id must be deterministic"
    );
    assert_eq!(stored.len(), 1, "overflow sink receives one body");
    assert_eq!(stored[0].0, source_id);
    assert!(stored[0].2.contains("alpha"));
    assert!(stored[0].2.contains("beta"));
}

#[test]
fn consolidation_dry_run_is_read_only_and_reports_synthesis() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m1",
        "2026-01-02T00:00:00Z",
        "alpha fact",
        "deploy",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m2",
        "2026-01-01T00:00:00Z",
        "beta fact",
        "deploy",
        MemoryImportance::Low,
        MemoryScope::Project,
    );
    store
        .conn
        .execute("UPDATE memories SET title = 'Same title'", [])
        .unwrap();

    let plan = crate::store::memory::consolidation_plan(
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        true,
        None,
    )
    .unwrap();
    assert_eq!(plan.groups.len(), 1);
    assert!(plan.groups[0].body.contains("alpha fact"));
    assert!(plan.groups[0].body.contains("beta fact"));
    assert_eq!(plan.groups[0].consolidated_id, None);

    let superseded: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE superseded_by IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(superseded, 0, "dry-run must not mutate storage");
}

#[test]
fn route_overflow_body_uses_deterministic_source_id() {
    let mut stored = Vec::new();
    let mut sink =
        |source_id: &str, _session_id: Option<&str>, body: &str| -> atlas_core::Result<()> {
            stored.push((source_id.to_owned(), body.to_owned()));
            Ok(())
        };
    let ids = vec!["m1".to_owned(), "m2".to_owned()];
    let first = route_overflow_body(&mut sink, "/repo", &ids, None, "full body").unwrap();
    let second = route_overflow_body(&mut sink, "/repo", &ids, None, "full body").unwrap();
    assert_eq!(first, second);
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].0, first);
}
