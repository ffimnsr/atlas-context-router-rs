use super::*;

#[test]
fn consolidation_plan_dry_run_groups_and_preserves_source_ids() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "m1",
        SeedSpec {
            at: RECENT,
            topic: "deploy",
            title: "Deploy process",
            body: "deploy uses helm",
            importance: MemoryImportance::High,
            source: Some("src-a"),
            category: None,
        },
    );
    seed(
        &store,
        "m2",
        SeedSpec {
            at: OLD,
            topic: "deploy",
            title: "Deploy process",
            body: "deploy uses helm charts",
            importance: MemoryImportance::Normal,
            source: Some("src-b"),
            category: None,
        },
    );
    seed(
        &store,
        "m3",
        SeedSpec {
            at: OLD,
            topic: "deploy",
            title: "Deploy process",
            body: "rollback via helm",
            importance: MemoryImportance::Low,
            source: Some("src-b"),
            category: None,
        },
    );
    seed(
        &store,
        "solo",
        SeedSpec {
            at: RECENT,
            topic: "other",
            title: "Solo",
            body: "unrelated fact",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );

    let plan = super::markdown_memory::consolidation_plan(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        true,
        None,
    )
    .unwrap();
    assert!(plan.dry_run);
    assert_eq!(plan.groups.len(), 1);
    let group = &plan.groups[0];
    assert_eq!(group.topic, "deploy");
    assert_eq!(
        group.kept_memory_id, "m1",
        "most recent row is the kept row"
    );
    assert_eq!(group.merged_memory_ids, vec!["m2", "m3"]);
    assert_eq!(group.source_ids, vec!["src-a", "src-b"]);
    assert_eq!(group.consolidated_id, None);
    assert_eq!(plan.merged_ids, vec!["m2", "m3"]);

    // Dry-run is fully read-only: no superseded rows.
    let active: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE superseded_by IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, 0);
}

#[test]
fn consolidation_plan_groups_by_same_body_and_metadata_category() {
    let (_dir, store) = open_store(16, 1024);
    // Same body, different titles and sources → body grouping.
    seed(
        &store,
        "b1",
        SeedSpec {
            at: RECENT,
            topic: "errors",
            title: "Title one",
            body: "postgres jsonb cast error",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "b2",
        SeedSpec {
            at: OLD,
            topic: "errors",
            title: "Title two",
            body: "Postgres  JSONB cast error ",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );
    // Same metadata category, different bodies → category grouping.
    seed(
        &store,
        "c1",
        SeedSpec {
            at: RECENT,
            topic: "decisions",
            title: "",
            body: "use postgres",
            importance: MemoryImportance::High,
            source: None,
            category: Some("decision"),
        },
    );
    seed(
        &store,
        "c2",
        SeedSpec {
            at: OLD,
            topic: "decisions",
            title: "",
            body: "use sqlite for cache",
            importance: MemoryImportance::Normal,
            source: None,
            category: Some("decision"),
        },
    );
    seed(
        &store,
        "c3",
        SeedSpec {
            at: OLD,
            topic: "decisions",
            title: "",
            body: "use redis cache",
            importance: MemoryImportance::Normal,
            source: None,
            category: Some("preference"),
        },
    );

    let plan = super::markdown_memory::consolidation_plan(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        true,
        None,
    )
    .unwrap();
    let by_topic = |topic: &str| {
        plan.groups
            .iter()
            .find(|group| group.topic == topic)
            .unwrap()
    };

    let errors = by_topic("errors");
    assert_eq!(errors.group_key, "same_body");
    assert_eq!(errors.merged_memory_ids, vec!["b2"]);

    let decisions = by_topic("decisions");
    assert_eq!(decisions.group_key, "same_category");
    assert_eq!(decisions.merged_memory_ids, vec!["c2"]);
}

#[test]
fn consolidation_apply_supersedes_merged_rows_and_links_them() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "m1",
        SeedSpec {
            at: RECENT,
            topic: "deploy",
            title: "Deploy process",
            body: "deploy uses helm",
            importance: MemoryImportance::High,
            source: Some("src-a"),
            category: None,
        },
    );
    seed(
        &store,
        "m2",
        SeedSpec {
            at: OLD,
            topic: "deploy",
            title: "Deploy process",
            body: "deploy uses helm charts",
            importance: MemoryImportance::Normal,
            source: Some("src-b"),
            category: None,
        },
    );

    let plan = super::markdown_memory::consolidation_plan(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        false,
        None,
    )
    .unwrap();
    assert!(!plan.dry_run);
    let group = &plan.groups[0];
    let consolidated_id = group
        .consolidated_id
        .as_deref()
        .expect("apply mode returns the consolidated id");
    assert_eq!(group.kept_memory_id, "m1");
    assert_eq!(group.merged_memory_ids, vec!["m2"]);

    // Merged row is marked superseded with a link row.
    let merged = store.get_memory("/repo", "m2").unwrap().unwrap();
    assert_eq!(merged.superseded_by.as_deref(), Some(consolidated_id));
    assert_eq!(merged.importance, MemoryImportance::Normal);
    let consolidated = store.get_memory("/repo", consolidated_id).unwrap().unwrap();
    assert!(
        consolidated
            .links
            .iter()
            .any(|link| { link.id == "m2" && link.relation == "supersedes" })
    );

    // Consolidated row preserves source references and merged ids in metadata.
    assert_eq!(
        consolidated.body,
        "deploy uses helm charts\ndeploy uses helm"
    );
    assert_eq!(consolidated.source_id.as_deref(), Some("src-a"));
    let metadata = consolidated.metadata;
    assert_eq!(metadata["consolidated"], serde_json::json!(true));
    assert_eq!(metadata["merged_ids"], serde_json::json!(["m2"]));
    assert_eq!(
        metadata["merged_source_ids"],
        serde_json::json!(["src-a", "src-b"])
    );
}

#[test]
fn consolidation_keeps_merged_rows_visible_behind_include_superseded() {
    let (_dir, store) = open_store(16, 1024);
    seed(
        &store,
        "m1",
        SeedSpec {
            at: RECENT,
            topic: "deploy",
            title: "Deploy process",
            body: "deploy uses helm",
            importance: MemoryImportance::High,
            source: None,
            category: None,
        },
    );
    seed(
        &store,
        "m2",
        SeedSpec {
            at: OLD,
            topic: "deploy",
            title: "Deploy process",
            body: "deploy uses helm charts",
            importance: MemoryImportance::Normal,
            source: None,
            category: None,
        },
    );
    super::markdown_memory::consolidation_plan(
        &store.markdown,
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        false,
        None,
    )
    .unwrap();

    // Default list hides superseded rows: only the consolidated file is active.
    let listed = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert_eq!(listed.len(), 1, "only the consolidated memory stays active");
    assert!(listed[0].superseded_by.is_none());

    // Explicit inspection shows the superseded row with its link target.
    let all = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                include_superseded: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(all.len(), 3);
    let m2 = all.iter().find(|m| m.id == "m2").unwrap();
    assert!(m2.superseded_by.is_some());

    // Recall rejects superseded rows; the active consolidated row keeps every
    // distinct fact from the merged member.
    let viewer = MemoryViewer {
        frontend: "cli".to_owned(),
        session_id: "s1".to_owned(),
    };
    let hits = store
        .recall_memories(
            "/repo",
            "helm",
            &MemoryListFilter::default(),
            false,
            &viewer,
            10,
        )
        .unwrap();
    assert!(
        hits.iter().all(|hit| hit.memory.id != "m2"),
        "superseded rows must not be surfaced by recall"
    );
}
