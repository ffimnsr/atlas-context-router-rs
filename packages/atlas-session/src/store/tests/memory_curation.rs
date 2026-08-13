// ── ICM-B — memory decay, stale, prune, health, and consolidation ─────────────

use serde_json::Value;
use time::OffsetDateTime;

use super::*;

/// Fixed "now" for deterministic age math: 2026-06-01T00:00:00Z.
const NOW: i64 = 1_780_272_000;
/// 2026-01-01T00:00:00Z — 151 days before NOW.
const OLD: i64 = 1_767_225_600;
/// 2026-05-01T00:00:00Z — 31 days before NOW.
const RECENT: i64 = 1_777_593_600;

fn ts(seconds: i64) -> String {
    atlas_core::format_rfc3339(
        OffsetDateTime::from_unix_timestamp(seconds)
            .expect("valid unix timestamp")
            .replace_nanosecond(0)
            .expect("0 nanoseconds is always valid"),
    )
}

fn now_ts() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(NOW).expect("valid unix timestamp")
}

fn default_policy() -> MemoryDecayPolicy {
    MemoryDecayPolicy::default()
}

/// Local seed with full control over title, source id, and metadata category.
struct SeedSpec<'a> {
    at: i64,
    topic: &'a str,
    title: &'a str,
    body: &'a str,
    importance: MemoryImportance,
    source: Option<&'a str>,
    category: Option<&'a str>,
}

fn seed(store: &SessionStore, id: &str, spec: SeedSpec<'_>) -> MemoryRecord {
    let mut metadata = serde_json::json!({});
    if let Some(category) = spec.category {
        metadata["category"] = serde_json::json!(category);
    }
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: spec.topic.to_owned(),
        title: spec.title.to_owned(),
        body: spec.body.to_owned(),
        importance: spec.importance,
        source_id: spec.source.map(str::to_owned),
        metadata,
    };
    super::memory::store_memory_at(&store.conn, &input, &ts(spec.at), id).unwrap()
}

// ── Decay scoring and policy ─────────────────────────────────────────────────

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

    let reports = super::memory::decay_reports(
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

    let reports = super::memory::decay_reports(
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
    let reports = super::memory::decay_reports(
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

    let stale = super::memory::stale_memories(
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

    let result = super::memory::prune_memories(
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

    let result = super::memory::prune_memories(
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
    let result = super::memory::prune_memories(
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

    let err = super::memory::prune_memories(
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
    let result = super::memory::prune_memories(
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

    let report = super::memory::health_report(
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

    let report = super::memory::health_report(
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

    let plan =
        super::memory::consolidation_plan(&store.conn, "/repo", &MemoryListFilter::default(), true)
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

    let plan =
        super::memory::consolidation_plan(&store.conn, "/repo", &MemoryListFilter::default(), true)
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

    let plan = super::memory::consolidation_plan(
        &store.conn,
        "/repo",
        &MemoryListFilter::default(),
        false,
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
    let (superseded_by, importance): (String, String) = store
        .conn
        .query_row(
            "SELECT superseded_by, importance FROM memories WHERE id = 'm2'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(superseded_by, consolidated_id);
    assert_eq!(importance, "normal");
    let (old_id, new_id, reason): (String, String, String) = store
        .conn
        .query_row(
            "SELECT old_memory_id, new_memory_id, reason FROM memory_supersessions",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(old_id, "m2");
    assert_eq!(new_id, consolidated_id);
    assert!(!reason.is_empty());

    // Consolidated row preserves source references and merged ids in metadata.
    let (body, source_id, metadata_json): (String, Option<String>, String) = store
        .conn
        .query_row(
            "SELECT body, source_id, metadata_json FROM memories WHERE id = ?1",
            [consolidated_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(body, "deploy uses helm");
    assert_eq!(source_id.as_deref(), Some("src-a"));
    let metadata: Value = serde_json::from_str(&metadata_json).unwrap();
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
    super::memory::consolidation_plan(&store.conn, "/repo", &MemoryListFilter::default(), false)
        .unwrap();

    // Default list hides superseded rows.
    let listed = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert_eq!(listed.len(), 2, "consolidated + kept row");
    assert!(listed.iter().any(|m| m.superseded_by.is_none()));

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

    // Recall ranks the active consolidated row above the superseded one.
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
    let positions = hits
        .iter()
        .enumerate()
        .map(|(index, hit)| (hit.memory.id.as_str(), index))
        .collect::<Vec<_>>();
    let m1_pos = positions.iter().find(|(id, _)| *id == "m1").unwrap().1;
    let m2_pos = positions.iter().find(|(id, _)| *id == "m2").unwrap().1;
    assert!(
        m1_pos < m2_pos,
        "active rows must rank above superseded rows"
    );
}
