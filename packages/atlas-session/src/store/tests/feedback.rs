// ── ICM-C — feedback record storage, search, stats, and matching ─────────────

use super::*;

fn feedback_input(
    id_hint: &str,
    predicted: &str,
    actual: &str,
    kind: &str,
    symbol: Option<&str>,
    file: Option<&str>,
    correction: &str,
) -> NewFeedback {
    NewFeedback {
        repo_root: "/repo".to_owned(),
        session_id: None,
        tool_name: "cli".to_owned(),
        analysis_kind: kind.to_owned(),
        predicted: predicted.to_owned(),
        actual: actual.to_owned(),
        correction: correction.to_owned(),
        related_symbol: symbol.map(str::to_owned),
        related_file: file.map(str::to_owned),
        source_id: Some(format!("src-{id_hint}")),
        metadata: serde_json::json!({ "seed": id_hint }),
    }
}

#[test]
fn store_feedback_validates_predicted_and_actual() {
    let (_dir, mut store) = open_store(16, 1024);

    let missing_predicted = feedback_input("a", "", "alive", "dead_code", None, None, "");
    let error = store
        .store_feedback(&missing_predicted)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("predicted must not be empty"),
        "got: {error}"
    );

    let missing_actual = feedback_input("b", "dead", "   ", "dead_code", None, None, "");
    let error = store
        .store_feedback(&missing_actual)
        .unwrap_err()
        .to_string();
    assert!(error.contains("actual must not be empty"), "got: {error}");

    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM feedback_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0, "rejected writes must not persist rows");
}

#[test]
fn store_feedback_roundtrips_all_fields() {
    let (_dir, mut store) = open_store(16, 1024);
    let record = store
        .store_feedback(&feedback_input(
            "1",
            "dead code",
            "still referenced via macro",
            "dead_code",
            Some("src/lib.rs::fn::legacy"),
            Some("src/lib.rs"),
            "keep the symbol; it is used by the macro registry",
        ))
        .unwrap();

    assert_eq!(record.id.len(), 64);
    assert_eq!(record.analysis_kind, "dead_code");
    assert_eq!(
        record.related_symbol.as_deref(),
        Some("src/lib.rs::fn::legacy")
    );
    assert_eq!(record.related_file.as_deref(), Some("src/lib.rs"));
    assert_eq!(record.source_id.as_deref(), Some("src-1"));
    assert!(!record.created_at.is_empty());
    assert!(record.is_false_positive_evidence());

    // FTS trigger must have indexed the row.
    let fts_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM feedback_records_fts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(fts_count, 1);
}

#[test]
fn search_feedback_finds_by_text_symbol_and_file() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .store_feedback(&feedback_input(
            "1",
            "dead code",
            "used by tests",
            "dead_code",
            Some("src/lib.rs::fn::legacy_parse"),
            Some("src/lib.rs"),
            "",
        ))
        .unwrap();
    store
        .store_feedback(&feedback_input(
            "2",
            "removable",
            "blocked by dynamic dispatch",
            "remove",
            Some("src/auth.rs::fn::verify_token"),
            Some("src/auth.rs"),
            "callers reach it through a trait object",
        ))
        .unwrap();

    // Correction text search.
    let by_correction = store
        .search_feedback(
            "/repo",
            "trait object",
            &FeedbackSearchFilter::default(),
            10,
        )
        .unwrap();
    assert_eq!(by_correction.len(), 1);
    assert_eq!(by_correction[0].feedback.analysis_kind, "remove");
    assert!(by_correction[0].relevance_score > 0.0);

    // Symbol search.
    let by_symbol = store
        .search_feedback(
            "/repo",
            "legacy_parse",
            &FeedbackSearchFilter::default(),
            10,
        )
        .unwrap();
    assert_eq!(by_symbol.len(), 1);
    assert_eq!(by_symbol[0].feedback.source_id.as_deref(), Some("src-1"));

    // Analysis-kind filter narrows results.
    let kind_filter = FeedbackSearchFilter {
        analysis_kind: Some("remove".to_owned()),
        ..Default::default()
    };
    let by_kind = store
        .search_feedback("/repo", "code", &kind_filter, 10)
        .unwrap();
    assert!(
        by_kind
            .iter()
            .all(|hit| hit.feedback.analysis_kind == "remove")
    );

    // Symbol + file filters.
    let file_filter = FeedbackSearchFilter {
        related_file: Some("src/auth.rs".to_owned()),
        ..Default::default()
    };
    let by_file = store
        .search_feedback("/repo", "blocked", &file_filter, 10)
        .unwrap();
    assert_eq!(by_file.len(), 1);
    assert_eq!(
        by_file[0].feedback.related_file.as_deref(),
        Some("src/auth.rs")
    );
}

#[test]
fn feedback_stats_are_stable_on_empty_table() {
    let (_dir, store) = open_store(16, 1024);
    let stats = store.feedback_stats("/repo").unwrap();
    assert_eq!(stats.total_count, 0);
    assert_eq!(stats.correction_count, 0);
    assert_eq!(stats.false_positive_count, 0);
    assert!(stats.by_analysis_kind.is_empty());
    assert!(stats.by_tool.is_empty());
}

#[test]
fn feedback_stats_break_down_by_kind_tool_and_corrections() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .store_feedback(&feedback_input(
            "1",
            "dead code",
            "used by tests",
            "dead_code",
            None,
            None,
            "",
        ))
        .unwrap();
    store
        .store_feedback(&feedback_input(
            "2",
            "dead code",
            "alive",
            "dead_code",
            None,
            None,
            "still used",
        ))
        .unwrap();
    store
        .store_feedback(&feedback_input(
            "3",
            "removable",
            "blocked",
            "remove",
            None,
            None,
            "dynamic",
        ))
        .unwrap();

    let stats = store.feedback_stats("/repo").unwrap();
    assert_eq!(stats.total_count, 3);
    assert_eq!(stats.correction_count, 2);
    assert_eq!(
        stats.false_positive_count, 3,
        "all three records correct a wrong prediction"
    );
    assert_eq!(
        stats
            .by_analysis_kind
            .get("dead_code")
            .copied()
            .unwrap_or(0),
        2
    );
    assert_eq!(
        stats.by_analysis_kind.get("remove").copied().unwrap_or(0),
        1
    );
    assert_eq!(stats.by_tool.get("cli").copied().unwrap_or(0), 3);
}

#[test]
fn feedback_matching_returns_records_for_symbol_file_or_kind() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .store_feedback(&feedback_input(
            "1",
            "dead code",
            "used by tests",
            "dead_code",
            Some("src/lib.rs::fn::legacy_parse"),
            None,
            "",
        ))
        .unwrap();
    store
        .store_feedback(&feedback_input(
            "2",
            "safe to remove",
            "used by plugin",
            "remove",
            None,
            Some("src/auth.rs"),
            "plugin registry loads it by name",
        ))
        .unwrap();

    // Symbol match.
    let by_symbol = store
        .feedback_matching(
            "/repo",
            "dead_code",
            Some("src/lib.rs::fn::legacy_parse"),
            None,
        )
        .unwrap();
    assert_eq!(by_symbol.len(), 1);
    assert_eq!(by_symbol[0].source_id.as_deref(), Some("src-1"));

    // File match.
    let by_file = store
        .feedback_matching("/repo", "remove", None, Some("src/auth.rs"))
        .unwrap();
    assert_eq!(by_file.len(), 1);

    // Kind-only match.
    let by_kind = store
        .feedback_matching("/repo", "remove", None, None)
        .unwrap();
    assert_eq!(by_kind.len(), 1);

    // No match at all.
    let none = store
        .feedback_matching("/repo", "safety", None, None)
        .unwrap();
    assert!(none.is_empty());
}

#[test]
fn feedback_schema_issues_detect_missing_table() {
    let (_dir, store) = open_store(16, 1024);
    assert!(store.feedback_schema_issues().is_empty());

    store
        .conn
        .execute_batch("DROP TABLE feedback_records")
        .unwrap();
    let issues = store.feedback_schema_issues();
    assert_eq!(issues, vec!["missing table: feedback_records"]);
}

#[test]
fn feedback_false_positive_flag_marks_identical_texts() {
    let (_dir, mut store) = open_store(16, 1024);
    // identical predicted/actual but explicitly flagged false positive
    let mut input = feedback_input("1", "dead code", "dead code", "dead_code", None, None, "");
    input.metadata = serde_json::json!({ "false_positive": true });
    let record = store.store_feedback(&input).unwrap();
    assert!(record.is_false_positive_evidence());

    let input = feedback_input("2", "dead code", "dead code", "dead_code", None, None, "");
    let record = store.store_feedback(&input).unwrap();
    assert!(
        !record.is_false_positive_evidence(),
        "identical texts are not evidence"
    );
}

#[test]
fn recent_feedback_returns_newest_records_bounded_by_limit() {
    let (_dir, mut store) = open_store(16, 1024);
    assert!(store.recent_feedback("/repo", 10).unwrap().is_empty());

    let ids: Vec<String> = (0..3)
        .map(|i| {
            store
                .store_feedback(&feedback_input(
                    &format!("seed-{i}"),
                    &format!("prediction {i}"),
                    "reality",
                    "dead_code",
                    None,
                    None,
                    "",
                ))
                .unwrap()
                .id
        })
        .collect();

    let bounded = store.recent_feedback("/repo", 2).unwrap();
    assert_eq!(bounded.len(), 2);
    for record in &bounded {
        assert!(ids.contains(&record.id), "unexpected record {}", record.id);
    }

    let all = store.recent_feedback("/repo", 10).unwrap();
    assert_eq!(all.len(), 3);
    let returned: Vec<&str> = all.iter().map(|record| record.id.as_str()).collect();
    assert!(
        ids.iter().all(|id| returned.contains(&id.as_str())),
        "all stored records must come back when the limit is high enough"
    );
    assert!(
        all.windows(2)
            .all(|pair| pair[0].created_at >= pair[1].created_at),
        "recent feedback must be newest-first"
    );
}
