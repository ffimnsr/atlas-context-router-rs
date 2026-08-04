use super::*;

#[test]
fn recall_memories_ranks_exact_topic_above_broad_matches() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m1",
        "2026-01-01T00:00:00Z",
        "deploy pipeline runs weekly",
        "hooks",
        MemoryImportance::Critical,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m2",
        "2026-01-02T00:00:00Z",
        "deploy notes",
        "deploy",
        MemoryImportance::Low,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m3",
        "2026-01-03T00:00:00Z",
        "unrelated body",
        "deployment",
        MemoryImportance::High,
        MemoryScope::Project,
    );

    let hits = store
        .recall_memories(
            "/repo",
            "deploy",
            &MemoryListFilter::default(),
            false,
            &MemoryViewer {
                frontend: "cli".to_owned(),
                session_id: "s1".to_owned(),
            },
            10,
        )
        .unwrap();
    let order = hits
        .iter()
        .map(|hit| hit.memory.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(order, vec!["m2", "m3", "m1"]);
    assert_eq!(
        hits.iter()
            .map(|hit| hit.relevance_score)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "exact topic must rank above contains matches above body-only matches"
    );
}

#[test]
fn recall_memories_respects_scope_and_shared_filters() {
    let (_dir, store) = open_store(16, 1024);
    for (id, scope) in [
        ("p", MemoryScope::Project),
        ("g", MemoryScope::Global),
        ("s", MemoryScope::Session),
        ("f", MemoryScope::Frontend),
    ] {
        seed_memory(
            &store,
            id,
            "2026-01-01T00:00:00Z",
            "shared topic body",
            "hooks",
            MemoryImportance::Normal,
            scope,
        );
    }

    let shared = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter::default(),
            true,
            &cli_viewer(),
            10,
        )
        .unwrap();
    let mut shared_ids = shared
        .iter()
        .map(|hit| hit.memory.id.as_str())
        .collect::<Vec<_>>();
    shared_ids.sort();
    assert_eq!(
        shared_ids,
        vec!["g", "p"],
        "--shared must exclude session/frontend"
    );

    let frontend_only = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter {
                scope: Some(MemoryScope::Frontend),
                ..Default::default()
            },
            false,
            &MemoryViewer {
                frontend: "codex".to_owned(),
                session_id: "s1".to_owned(),
            },
            10,
        )
        .unwrap();
    assert_eq!(frontend_only.len(), 1);
    assert_eq!(frontend_only[0].memory.id, "f");

    // Visibility: the codex viewer sees everything (session s1 + codex), the
    // cli viewer does not see codex-frontend memories.
    let codex_view = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter::default(),
            false,
            &MemoryViewer {
                frontend: "codex".to_owned(),
                session_id: "s1".to_owned(),
            },
            10,
        )
        .unwrap();
    assert_eq!(codex_view.len(), 4);
    let cli_view = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    let mut cli_ids = cli_view
        .iter()
        .map(|hit| hit.memory.id.as_str())
        .collect::<Vec<_>>();
    cli_ids.sort();
    assert_eq!(
        cli_ids,
        vec!["g", "p", "s"],
        "cli viewer sees project/global/session"
    );
}

#[test]
fn recall_memories_applies_topic_and_importance_filters_and_escapes_like() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m1",
        "2026-01-01T00:00:00Z",
        "progress is 100% done",
        "hooks",
        MemoryImportance::Critical,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m2",
        "2026-01-01T00:00:00Z",
        "progress is 10000",
        "hooks",
        MemoryImportance::Low,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m3",
        "2026-01-01T00:00:00Z",
        "deploy",
        "deploy",
        MemoryImportance::Critical,
        MemoryScope::Project,
    );

    // Literal substring match: the `%` in the query must not act as a wildcard.
    let literal = store
        .recall_memories(
            "/repo",
            "100%",
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert_eq!(literal.len(), 1);
    assert_eq!(literal[0].memory.id, "m1");

    // Topic and importance filters restrict recall.
    let filtered = store
        .recall_memories(
            "/repo",
            "deploy",
            &MemoryListFilter {
                topic: Some("HOOKS".to_owned()),
                ..Default::default()
            },
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert!(
        filtered.is_empty(),
        "topic filter is exact and case-insensitive"
    );

    let critical = store
        .recall_memories(
            "/repo",
            "deploy",
            &MemoryListFilter {
                importance: Some(MemoryImportance::Critical),
                ..Default::default()
            },
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert_eq!(critical.len(), 1);
    assert_eq!(critical[0].memory.id, "m3");
}

#[test]
fn list_memories_sorts_by_updated_at_desc_and_filters() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "old",
        "2026-01-01T00:00:00Z",
        "b",
        "hooks",
        MemoryImportance::Low,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "mid",
        "2026-01-02T00:00:00Z",
        "b",
        "hooks",
        MemoryImportance::Critical,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "new",
        "2026-01-03T00:00:00Z",
        "b",
        "other",
        MemoryImportance::Critical,
        MemoryScope::Global,
    );

    let all = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert_eq!(
        all.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["new", "mid", "old"],
        "list must sort by updated_at DESC"
    );

    let critical = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                importance: Some(MemoryImportance::Critical),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        critical.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["new", "mid"]
    );

    let hooks = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                topic: Some("HOOKS".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        hooks.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["mid", "old"],
        "topic filter must be case-insensitive exact match"
    );

    let range = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                older_than: Some("2026-01-03T00:00:00Z".to_owned()),
                newer_than: Some("2026-01-01T00:00:00Z".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        range.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["mid"]
    );
}

#[test]
fn delete_memory_requires_exact_id_and_respects_dry_run() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m1",
        "2026-01-01T00:00:00Z",
        "b",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    let dry = store.delete_memory("/repo", "m1", true).unwrap();
    assert!(dry.found);
    assert!(!dry.deleted);
    assert!(dry.dry_run);
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "dry-run must not mutate storage");

    let missing = store.delete_memory("/repo", "nope", false).unwrap();
    assert!(!missing.found);
    assert!(!missing.deleted);

    let other_repo = store.delete_memory("/elsewhere", "m1", false).unwrap();
    assert!(!other_repo.found, "delete is repo-scoped by exact id");

    let removed = store.delete_memory("/repo", "m1", false).unwrap();
    assert!(removed.found);
    assert!(removed.deleted);
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
