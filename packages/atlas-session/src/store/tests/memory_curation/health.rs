use super::*;

/// Only `src-present` exists in the (fake) content store.
fn existing_sources(source_id: &str) -> bool {
    source_id == "src-present"
}

#[test]
fn health_report_categories_are_deterministic() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "stale",
        SeedSpec {
            at: OLD,
            topic: "hooks",
            title: "",
            body: "old low fact",
            importance: MemoryImportance::Low,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "dup-a",
        SeedSpec {
            at: RECENT,
            topic: "hooks",
            title: "",
            body: "deploy uses helm",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "dup-b",
        SeedSpec {
            at: RECENT,
            topic: "hooks",
            title: "",
            body: "deploy uses helm",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "orphan",
        SeedSpec {
            at: RECENT,
            topic: "hooks",
            title: "",
            body: "links to removed artifact",
            importance: MemoryImportance::Normal,
            source: Some("src-gone"),
            category: None,
        },
    );
    seed(
        &store,
        "present",
        SeedSpec {
            at: RECENT,
            topic: "hooks",
            title: "",
            body: "links to live artifact",
            importance: MemoryImportance::Normal,
            source: Some("src-present"),
            category: None,
        },
    );
    seed(
        &store,
        "huge",
        SeedSpec {
            at: RECENT,
            topic: "hooks",
            title: "",
            body: &"x".repeat(OVERSIZED_BODY_CHARS + 10),
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "healthy",
        SeedSpec {
            at: RECENT,
            topic: "hooks",
            title: "",
            body: "a single healthy fact",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );

    let report = super::markdown_memory::health_report(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        &default_policy(),
        now_ts(),
        &existing_sources,
    )
    .unwrap();
    assert_eq!(report.total_memories, 7);

    let kinds = report
        .findings
        .iter()
        .map(|f| (f.kind.as_str(), f.memory_id.as_deref()))
        .collect::<Vec<_>>();
    assert!(kinds.contains(&("stale_memory", Some("stale"))));
    assert!(kinds.contains(&("duplicated_memory", Some("dup-b"))));
    assert!(!kinds.contains(&("duplicated_memory", Some("dup-a"))));
    assert!(kinds.contains(&("orphaned_source", Some("orphan"))));
    assert!(!kinds.contains(&("orphaned_source", Some("present"))));
    assert!(kinds.contains(&("oversized_memory", Some("huge"))));
    assert!(!kinds.contains(&("stale_memory", Some("healthy"))));

    // Every finding carries an exact follow-up command.
    for finding in &report.findings {
        assert!(
            finding.command.starts_with("atlas memory "),
            "finding must be actionable: {}",
            finding.command
        );
    }
    assert_eq!(report.by_category.get("stale").copied().unwrap_or(0), 1);
    assert_eq!(
        report.by_category.get("duplicated").copied().unwrap_or(0),
        1
    );
    assert_eq!(report.by_category.get("orphaned").copied().unwrap_or(0), 1);
    assert_eq!(report.by_category.get("oversized").copied().unwrap_or(0), 1);
}

#[test]
fn health_report_flags_noisy_topics_and_missing_critical_decisions() {
    let (_dir, store) = open_store(16, 1024);
    for index in 0..NOISY_TOPIC_ENTRIES + 1 {
        seed(
            &store,
            &format!("noisy-{index}"),
            SeedSpec {
                at: RECENT,
                topic: "noisy-topic",
                title: "",
                body: &format!("fact number {index}"),
                importance: MemoryImportance::Normal,
                source: None,
                category: None,
            },
        );
    }
    seed(
        &store,
        "tiny",
        SeedSpec {
            at: RECENT,
            topic: "tiny-topic",
            title: "",
            body: "only fact",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );

    let report = super::markdown_memory::health_report(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        &default_policy(),
        now_ts(),
        &existing_sources,
    )
    .unwrap();

    let noisy = report
        .findings
        .iter()
        .find(|f| f.kind == "noisy_topic")
        .expect("noisy topic finding");
    assert_eq!(noisy.topic.as_deref(), Some("noisy-topic"));
    assert!(
        noisy
            .detail
            .contains(&format!("{} memories", NOISY_TOPIC_ENTRIES + 1))
    );

    let no_critical = report
        .findings
        .iter()
        .filter(|f| f.kind == "topic_without_critical")
        .map(|f| f.topic.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(no_critical.len(), 2, "both topics lack a critical decision");
    assert!(no_critical.contains(&Some("noisy-topic")));
    assert!(no_critical.contains(&Some("tiny-topic")));
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.command.contains("--importance critical")),
        "suggestion must show the critical store command"
    );
}

// ── Consolidation ─────────────────────────────────────────────────────────────
