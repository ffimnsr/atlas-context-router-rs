use super::*;

#[test]
fn decay_policy_protects_critical_and_scores_linearly() {
    let policy = default_policy();
    assert_eq!(policy.retention_days(MemoryImportance::Critical), None);
    assert_eq!(policy.retention_days(MemoryImportance::High), Some(365));
    assert_eq!(policy.retention_days(MemoryImportance::Normal), Some(90));
    assert_eq!(policy.retention_days(MemoryImportance::Low), Some(30));

    assert_eq!(policy.score(MemoryImportance::Critical, 10_000.0), 0.0);
    assert_eq!(policy.score(MemoryImportance::Low, 15.0), 0.5);
    assert_eq!(policy.score(MemoryImportance::Low, 60.0), 1.0);
    assert_eq!(policy.score(MemoryImportance::High, 365.0), 1.0);

    let no_protection = MemoryDecayPolicy {
        critical_never_prune: false,
        ..default_policy()
    };
    assert_eq!(
        no_protection.retention_days(MemoryImportance::Critical),
        Some(365)
    );
    assert_eq!(no_protection.score(MemoryImportance::Critical, 365.0), 1.0);
}

#[test]
fn decay_reports_compute_scores_without_writing_in_dry_run() {
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
            body: "recent normal fact",
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

    let reports = super::markdown_memory::decay_reports(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        &default_policy(),
        now_ts(),
        true,
    )
    .unwrap();
    assert_eq!(reports.len(), 3);

    let by_id = |id: &str| reports.iter().find(|r| r.memory.id == id).unwrap();
    assert!(by_id("low").stale);
    assert_eq!(by_id("low").updated_decay_score, 1.0);
    assert_eq!(by_id("low").retention_days, Some(30));
    assert_eq!(by_id("low").age_days, 151.0);

    assert!(!by_id("normal").stale);
    assert!((by_id("normal").updated_decay_score - 31.0 / 90.0).abs() < 1e-9);

    assert!(by_id("critical").protected);
    assert!(!by_id("critical").stale);
    assert_eq!(by_id("critical").updated_decay_score, 0.0);
    assert_eq!(by_id("critical").retention_days, None);

    // Dry-run must not touch the stored scores.
    let stored: f64 = store
        .conn
        .query_row(
            "SELECT decay_score FROM memories WHERE id = 'low'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0.0);
}

#[test]
fn decay_apply_persists_scores_and_never_prunes() {
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

    let reports = super::markdown_memory::decay_reports(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        &default_policy(),
        now_ts(),
        false,
    )
    .unwrap();
    assert_eq!(reports.len(), 2);

    let stored_low: f64 = store
        .conn
        .query_row(
            "SELECT decay_score FROM memories WHERE id = 'low'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_low, 1.0);
    let stored_critical: f64 = store
        .conn
        .query_row(
            "SELECT decay_score FROM memories WHERE id = 'critical'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_critical, 0.0);

    // Decay never deletes rows.
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn disabled_policy_reports_no_decay() {
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
    let disabled = MemoryDecayPolicy {
        enabled: false,
        ..default_policy()
    };
    let reports = super::markdown_memory::decay_reports(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        &disabled,
        now_ts(),
        false,
    )
    .unwrap();
    assert!(reports.is_empty());
}

// ── Stale and prune ──────────────────────────────────────────────────────────
