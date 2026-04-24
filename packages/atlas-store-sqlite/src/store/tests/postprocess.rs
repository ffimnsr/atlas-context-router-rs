use super::*;
use atlas_core::{
    PostprocessExecutionMode, PostprocessRunState, PostprocessRunSummary, PostprocessStageStatus,
    PostprocessStageSummary,
};

#[test]
fn migration_adds_postprocess_state_table() {
    let store = open_in_memory();
    let columns = table_columns(&store.conn, "postprocess_state");
    assert_eq!(
        columns,
        cols(&[
            "repo_root",
            "state",
            "mode",
            "stage_filter",
            "changed_file_count",
            "stages_json",
            "started_at_ms",
            "finished_at_ms",
            "last_error_code",
            "last_error",
            "updated_at_ms",
        ])
    );

    let indexes = schema_indexes(&store.conn);
    assert!(indexes.contains("idx_postprocess_state_state"));
    assert!(indexes.contains("idx_postprocess_state_updated_at_ms"));
}

#[test]
fn postprocess_status_round_trips_stage_summary() {
    let store = open_in_memory();
    store
        .begin_postprocess(
            "/repo",
            PostprocessExecutionMode::ChangedOnly,
            Some("flows"),
            2,
        )
        .unwrap();

    let summary = PostprocessRunSummary {
        repo_root: "/repo".to_string(),
        ok: true,
        noop: false,
        noop_reason: None,
        error_code: "none".to_string(),
        message: "postprocess complete".to_string(),
        suggestions: vec![],
        graph_built: true,
        state: PostprocessRunState::Succeeded,
        requested_mode: PostprocessExecutionMode::ChangedOnly,
        stage_filter: Some("flows".to_string()),
        dry_run: false,
        changed_files: vec!["src/lib.rs".to_string(), "src/api.rs".to_string()],
        started_at_ms: 10,
        finished_at_ms: 25,
        total_elapsed_ms: 15,
        stages: vec![PostprocessStageSummary {
            stage: "flows".to_string(),
            status: PostprocessStageStatus::Completed,
            mode: PostprocessExecutionMode::ChangedOnly,
            affected_file_count: 2,
            item_count: 1,
            elapsed_ms: 5,
            error_code: None,
            message: Some("ok".to_string()),
            details: serde_json::json!({ "flow_count": 1 }),
        }],
        supported_stages: vec!["flows".to_string()],
    };

    store.finish_postprocess(&summary).unwrap();

    let status = store
        .get_postprocess_status("/repo")
        .unwrap()
        .expect("postprocess status");
    assert_eq!(status.state, PostprocessRunState::Succeeded);
    assert_eq!(status.mode, PostprocessExecutionMode::ChangedOnly);
    assert_eq!(status.changed_file_count, 2);
    assert_eq!(status.stages.len(), 1);
    assert_eq!(status.stages[0].stage, "flows");
    assert_eq!(status.stages[0].status, PostprocessStageStatus::Completed);
}

#[test]
fn find_large_functions_filters_by_threshold_and_files() {
    let mut store = open_in_memory();
    let short = Node {
        line_end: 10,
        ..make_node(
            NodeKind::Function,
            "short",
            "src/short.rs::fn::short",
            "src/short.rs",
            "rust",
        )
    };
    let long = Node {
        line_end: 60,
        ..make_node(
            NodeKind::Function,
            "long",
            "src/long.rs::fn::long",
            "src/long.rs",
            "rust",
        )
    };
    store
        .replace_file_graph("src/short.rs", "h1", Some("rust"), Some(10), &[short], &[])
        .unwrap();
    store
        .replace_file_graph("src/long.rs", "h2", Some("rust"), Some(60), &[long], &[])
        .unwrap();

    let all = store.find_large_functions(None, 40, 10).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].qualified_name, "src/long.rs::fn::long");

    let filtered = store
        .find_large_functions(Some(&["src/short.rs".to_string()]), 40, 10)
        .unwrap();
    assert!(filtered.is_empty());
}
