use super::*;

#[test]
fn begin_indexing_sets_state_to_indexing() {
    let mut store = open_store();
    store.begin_indexing("/repo/a", 42).unwrap();
    let status = store.get_index_status("/repo/a").unwrap().unwrap();
    assert_eq!(status.state, IndexState::Indexing);
    assert_eq!(status.files_discovered, 42);
    assert_eq!(status.files_indexed, 0);
    assert_eq!(status.chunks_written, 0);
    assert!(status.last_indexed_at.is_none());
    assert!(status.last_error.is_none());
}

#[test]
fn finish_indexing_marks_indexed_and_stamps_time() {
    let mut store = open_store();
    store.begin_indexing("/repo/b", 10).unwrap();
    store
        .finish_indexing(
            "/repo/b",
            &IndexingStats {
                files_indexed: 9,
                chunks_written: 30,
                chunks_reused: 1,
            },
        )
        .unwrap();
    let status = store.get_index_status("/repo/b").unwrap().unwrap();
    assert_eq!(status.state, IndexState::Indexed);
    assert_eq!(status.files_indexed, 9);
    assert_eq!(status.chunks_written, 30);
    assert_eq!(status.chunks_reused, 1);
    assert!(status.last_indexed_at.is_some());
    assert!(status.last_error.is_none());
}

#[test]
fn fail_indexing_sets_error_state() {
    let mut store = open_store();
    store.begin_indexing("/repo/c", 5).unwrap();
    store
        .fail_indexing("/repo/c", "parse error on main.rs")
        .unwrap();
    let status = store.get_index_status("/repo/c").unwrap().unwrap();
    assert_eq!(status.state, IndexState::IndexFailed);
    assert_eq!(status.last_error.unwrap(), "parse error on main.rs");
}

#[test]
fn missing_repo_returns_none() {
    let store = open_store();
    let status = store.get_index_status("/nonexistent/repo").unwrap();
    assert!(status.is_none());
}

#[test]
fn list_index_statuses_returns_all_repos() {
    let mut store = open_store();
    store.begin_indexing("/repo/x", 1).unwrap();
    store
        .finish_indexing("/repo/x", &IndexingStats::default())
        .unwrap();
    store.begin_indexing("/repo/y", 2).unwrap();
    let statuses = store.list_index_statuses().unwrap();
    assert_eq!(statuses.len(), 2);
    let roots: Vec<&str> = statuses
        .iter()
        .map(|status| status.repo_root.as_str())
        .collect();
    assert!(roots.contains(&"/repo/x"));
    assert!(roots.contains(&"/repo/y"));
}

#[test]
fn begin_indexing_resets_counters_on_restart() {
    let mut store = open_store();
    store.begin_indexing("/repo/d", 20).unwrap();
    store
        .finish_indexing(
            "/repo/d",
            &IndexingStats {
                files_indexed: 18,
                chunks_written: 60,
                chunks_reused: 2,
            },
        )
        .unwrap();
    store.begin_indexing("/repo/d", 25).unwrap();
    let status = store.get_index_status("/repo/d").unwrap().unwrap();
    assert_eq!(status.state, IndexState::Indexing);
    assert_eq!(status.files_discovered, 25);
    assert_eq!(status.files_indexed, 0);
    assert_eq!(status.chunks_written, 0);
}

#[test]
fn index_artifact_auto_increments_index_counters() {
    let mut store = open_store();
    store
        .index_artifact(meta("src-ai"), "auto increment test content", "text/plain")
        .unwrap();
    let status = store.get_index_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, IndexState::Indexed);
    assert!(status.files_indexed >= 1);
    assert!(status.chunks_written >= 1);
}

#[test]
fn interrupted_indexing_visible_as_indexing_state() {
    let mut store = open_store();
    store.begin_indexing("/repo/interrupted", 100).unwrap();
    let status = store
        .get_index_status("/repo/interrupted")
        .unwrap()
        .unwrap();
    assert_eq!(
        status.state,
        IndexState::Indexing,
        "interrupted run must show as 'indexing' to signal recovery needed"
    );
}

// ---------------------------------------------------------------------------
// Patch R2 — Retrieval batching and chunk explosion guardrails
// ---------------------------------------------------------------------------
