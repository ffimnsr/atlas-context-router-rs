use super::*;

#[test]
fn stale_memories_never_report_protected_critical() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "low",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old low fact",
            importance: MemoryImportance::Low,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "critical",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old critical fact",
            importance: MemoryImportance::Critical,
            source: None,
            category: None,
        },
    );

    let stale = super::markdown_memory::stale_memories(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        &default_policy(),
        now_ts(),
    )
    .unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].memory.id, "low");
}

#[test]
fn prune_dry_run_reports_candidates_without_deleting() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "low",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old low fact",
            importance: MemoryImportance::Low,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "normal",
        SeedSpec {
            at: RECENT,
            topic: "t",
            title: "",
            body: "fresh normal fact",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "critical",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old critical fact",
            importance: MemoryImportance::Critical,
            source: None,
            category: None,
        },
    );

    let result = super::markdown_memory::prune_memories(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        &default_policy(),
        now_ts(),
        true,
        false,
    )
    .unwrap();
    assert!(result.dry_run);
    assert_eq!(result.candidate_count, 1);
    assert_eq!(result.deleted_count, 0);
    assert_eq!(result.protected_count, 1);
    assert_eq!(result.candidates[0].id, "low");

    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn prune_apply_deletes_only_pruneable_rows() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "low",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old low fact",
            importance: MemoryImportance::Low,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "critical",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old critical fact",
            importance: MemoryImportance::Critical,
            source: None,
            category: None,
        },
    );

    let result = super::markdown_memory::prune_memories(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        &default_policy(),
        now_ts(),
        false,
        false,
    )
    .unwrap();
    assert_eq!(result.candidate_count, 1);
    assert_eq!(result.deleted_count, 1);
    assert_eq!(result.protected_count, 1);

    let remaining: Vec<String> = store
        .conn
        .prepare("SELECT id FROM memories ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(remaining, vec!["critical"]);
}

#[test]
fn prune_importance_low_selects_only_low_rows() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "low",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old low fact",
            importance: MemoryImportance::Low,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "normal",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old normal fact",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );

    let filter = MemoryListFilter {
        importance: Some(MemoryImportance::Low),
        ..Default::default()
    };
    let result = super::markdown_memory::prune_memories(
        &store.markdown,
        &store.conn,
        "/repo",
        &filter,
        &default_policy(),
        now_ts(),
        true,
        false,
    )
    .unwrap();
    assert_eq!(result.candidate_count, 1);
    assert_eq!(result.candidates[0].id, "low");
}

#[test]
fn prune_critical_requires_explicit_override() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "critical",
        SeedSpec {
            at: OLD,
            topic: "t",
            title: "",
            body: "old critical fact",
            importance: MemoryImportance::Critical,
            source: None,
            category: None,
        },
    );
    // Without protection, critical rows decay on the high retention window;
    // shrink it so the seeded row is well past it.
    let no_protection = MemoryDecayPolicy {
        critical_never_prune: false,
        high_days: 30,
        ..default_policy()
    };
    let filter = MemoryListFilter {
        importance: Some(MemoryImportance::Critical),
        ..Default::default()
    };

    let err = super::markdown_memory::prune_memories(
        &store.markdown,
        &store.conn,
        "/repo",
        &filter,
        &no_protection,
        now_ts(),
        true,
        false,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("--allow-critical"),
        "error must mention the override: {err}"
    );

    // With the explicit override the critical row becomes a candidate.
    let result = super::markdown_memory::prune_memories(
        &store.markdown,
        &store.conn,
        "/repo",
        &filter,
        &no_protection,
        now_ts(),
        true,
        true,
    )
    .unwrap();
    assert_eq!(result.candidate_count, 1);
    assert_eq!(result.candidates[0].id, "critical");
}

// ── Health ────────────────────────────────────────────────────────────────────
