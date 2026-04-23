use super::*;

// ---------------------------------------------------------------------------
// Graph build lifecycle state tests
// ---------------------------------------------------------------------------

#[test]
fn begin_build_sets_state_building() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::Building);
    assert_eq!(status.nodes_written, 0);
    assert!(status.last_error.is_none());
}

#[test]
fn finish_build_after_begin_sets_built_with_counters() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    store
        .finish_build(
            "/repo",
            BuildFinishStats {
                state: GraphBuildState::Built,
                files_discovered: 10,
                files_processed: 9,
                files_accepted: 9,
                files_skipped_by_byte_budget: 1,
                files_failed: 1,
                bytes_accepted: 1024,
                bytes_skipped: 2048,
                nodes_written: 50,
                edges_written: 30,
                budget_stop_reason: Some("max_file_bytes".to_owned()),
            },
        )
        .unwrap();
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::Built);
    assert_eq!(status.files_discovered, 10);
    assert_eq!(status.files_processed, 9);
    assert_eq!(status.files_accepted, 9);
    assert_eq!(status.files_skipped_by_byte_budget, 1);
    assert_eq!(status.files_failed, 1);
    assert_eq!(status.bytes_accepted, 1024);
    assert_eq!(status.bytes_skipped, 2048);
    assert_eq!(status.nodes_written, 50);
    assert_eq!(status.edges_written, 30);
    assert_eq!(status.budget_stop_reason.as_deref(), Some("max_file_bytes"));
    assert!(status.last_built_at.is_some());
    assert!(status.last_error.is_none());
}

#[test]
fn fail_build_after_begin_sets_build_failed_with_error() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    store.fail_build("/repo", "disk full").unwrap();
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::BuildFailed);
    assert_eq!(status.last_error.as_deref(), Some("disk full"));
}

#[test]
fn get_build_status_returns_none_when_no_row() {
    let store = open_in_memory();
    let result = store.get_build_status("/nonexistent").unwrap();
    assert!(result.is_none());
}

#[test]
fn list_build_statuses_returns_all_repos() {
    let store = open_in_memory();
    store.begin_build("/repo/a").unwrap();
    store.begin_build("/repo/b").unwrap();
    store
        .finish_build(
            "/repo/b",
            BuildFinishStats {
                state: GraphBuildState::Built,
                files_discovered: 5,
                files_processed: 5,
                files_accepted: 5,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 500,
                bytes_skipped: 0,
                nodes_written: 20,
                edges_written: 10,
                budget_stop_reason: None,
            },
        )
        .unwrap();
    let statuses = store.list_build_statuses().unwrap();
    assert_eq!(statuses.len(), 2);
    // Ordered by repo_root
    assert_eq!(statuses[0].repo_root, "/repo/a");
    assert_eq!(statuses[0].state, GraphBuildState::Building);
    assert_eq!(statuses[1].repo_root, "/repo/b");
    assert_eq!(statuses[1].state, GraphBuildState::Built);
}

#[test]
fn interrupted_build_state_stays_building() {
    // Simulate a crash: begin_build called but finish/fail never called.
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    // Reopen — state must still be 'building', detectable by doctor.
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::Building);
}

#[test]
fn counters_overwritten_on_repeated_finish() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    store
        .finish_build(
            "/repo",
            BuildFinishStats {
                state: GraphBuildState::Built,
                files_discovered: 5,
                files_processed: 5,
                files_accepted: 5,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 500,
                bytes_skipped: 0,
                nodes_written: 10,
                edges_written: 5,
                budget_stop_reason: None,
            },
        )
        .unwrap();
    // Second build run
    store.begin_build("/repo").unwrap();
    store
        .finish_build(
            "/repo",
            BuildFinishStats {
                state: GraphBuildState::Built,
                files_discovered: 20,
                files_processed: 18,
                files_accepted: 18,
                files_skipped_by_byte_budget: 2,
                files_failed: 2,
                bytes_accepted: 1800,
                bytes_skipped: 200,
                nodes_written: 80,
                edges_written: 40,
                budget_stop_reason: Some("max_total_bytes_per_run".to_owned()),
            },
        )
        .unwrap();
    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.files_discovered, 20);
    assert_eq!(status.files_accepted, 18);
    assert_eq!(status.nodes_written, 80);
}

#[test]
fn finish_build_persists_degraded_state_and_stop_reason() {
    let store = open_in_memory();
    store.begin_build("/repo").unwrap();
    store
        .finish_build(
            "/repo",
            BuildFinishStats {
                state: GraphBuildState::Degraded,
                files_discovered: 6,
                files_processed: 4,
                files_accepted: 4,
                files_skipped_by_byte_budget: 2,
                files_failed: 0,
                bytes_accepted: 400,
                bytes_skipped: 200,
                nodes_written: 12,
                edges_written: 8,
                budget_stop_reason: Some("max_total_bytes_per_run".to_owned()),
            },
        )
        .unwrap();

    let status = store.get_build_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, GraphBuildState::Degraded);
    assert_eq!(
        status.budget_stop_reason.as_deref(),
        Some("max_total_bytes_per_run")
    );
    assert_eq!(status.files_skipped_by_byte_budget, 2);
}
