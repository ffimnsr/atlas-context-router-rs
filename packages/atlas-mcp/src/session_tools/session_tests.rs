//! Unit tests for the session identity and continuity tools.

use super::session::continuity_event_spec;
use super::test_util::{
    append_session_event, open_session_store, seed_session_meta, setup_db_path, tool_body,
};
use super::*;
use atlas_session::SessionEventType;
use tempfile::TempDir;

#[test]
fn test_get_session_status_no_session() {
    let dir = TempDir::new().unwrap();
    let repo_root = dir.path().to_str().unwrap();
    let db_path = setup_db_path(&dir);
    let result = tool_get_session_status(None, repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["status"], "no_session");
    assert_eq!(body["summary"]["status"], "no_session");
    assert_eq!(body["repo_root"], repo_root);
    assert_eq!(body["resume_snapshot_exists"], false);
    assert!(body["warnings"].as_array().is_some());
}

#[test]
fn get_session_status_active_and_resumable_share_shape() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let repo_root = dir.path().to_str().unwrap();
    let db_path = setup_db_path(&dir);
    let mut store = open_session_store(&db_path);
    let session_id = seed_session_meta(&mut store, repo_root);
    append_session_event(
        &mut store,
        &session_id,
        SessionEventType::ContextRequest,
        serde_json::json!({"query": "compute"}),
    );
    store.build_resume(&session_id).unwrap();

    let result = tool_get_session_status(None, repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["session_id"], session_id.as_str());
    assert_eq!(body["status"], "active");
    assert_eq!(body["resume_snapshot_exists"], true);
    assert_eq!(body["summary"]["has_session"], true);
    assert!(body["event_count"].as_i64().unwrap() >= 1);
}

#[test]
fn compact_session_no_op_returns_normalized_shape() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let repo_root = dir.path().to_str().unwrap();
    let db_path = setup_db_path(&dir);

    let result = tool_compact_session(None, repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(
        body["session_id"],
        SessionId::derive(repo_root, "", "mcp").as_str()
    );
    assert_eq!(body["summary"]["no_op"], true);
    assert_eq!(body["before_counts"]["events"], 0);
    assert_eq!(body["after_counts"]["events"], 0);
}

#[test]
fn compact_session_effective_returns_normalized_shape() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let repo_root = dir.path().to_str().unwrap();
    let db_path = setup_db_path(&dir);
    let mut store = open_session_store(&db_path);
    let session_id = seed_session_meta(&mut store, repo_root);
    for run in 0..5 {
        append_session_event(
            &mut store,
            &session_id,
            SessionEventType::CommandRun,
            serde_json::json!({"command": "cargo build", "run": run}),
        );
    }
    drop(store);

    let result = tool_compact_session(None, repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["session_id"], session_id.as_str());
    assert!(body["merged_groups"].as_i64().unwrap() >= 1);
    assert!(
        body["removed_events"].as_i64().unwrap() >= 1
            || body["after_counts"]["events"].as_i64().unwrap()
                < body["before_counts"]["events"].as_i64().unwrap()
    );
    assert_eq!(body["summary"]["status"], "ok");
}

#[test]
fn resume_session_builds_snapshot_when_missing() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let repo_root = dir.path().to_str().unwrap();
    let db_path = setup_db_path(&dir);
    let mut store = open_session_store(&db_path);
    let session_id = seed_session_meta(&mut store, repo_root);
    append_session_event(
        &mut store,
        &session_id,
        SessionEventType::UserIntent,
        serde_json::json!({"intent": "review"}),
    );
    drop(store);

    let result = tool_resume_session(
        Some(&serde_json::json!({"mark_consumed": false})),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let body = tool_body(&result);
    assert_eq!(body["session_id"], session_id.as_str());
    assert_eq!(body["snapshot_status"], "built_snapshot");
    assert_eq!(body["consumed"], false);
    assert!(body["snapshot"].is_object());
    assert!(body["event_count"].as_i64().unwrap() >= 1);
}

#[test]
fn resume_session_reuses_existing_snapshot_and_marks_consumed() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let repo_root = dir.path().to_str().unwrap();
    let db_path = setup_db_path(&dir);
    let mut store = open_session_store(&db_path);
    let session_id = seed_session_meta(&mut store, repo_root);
    append_session_event(
        &mut store,
        &session_id,
        SessionEventType::Decision,
        serde_json::json!({"summary": "reuse context"}),
    );
    store.build_resume(&session_id).unwrap();
    drop(store);

    let result = tool_resume_session(None, repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["snapshot_status"], "existing_snapshot");
    assert_eq!(body["consumed"], true);
    assert_eq!(body["summary"]["snapshot_consumed"], true);

    let store = open_session_store(&db_path);
    let snapshot = store.get_resume_snapshot(&session_id).unwrap().unwrap();
    assert!(snapshot.consumed);
}

#[test]
fn test_continuity_event_spec_known_tools() {
    let args = serde_json::json!({"text": "find foo"});
    let spec = continuity_event_spec("query_graph", Some(&args));
    assert!(spec.is_some());
    let (et, payload) = spec.unwrap();
    assert_eq!(et, SessionEventType::ContextRequest);
    assert_eq!(payload["query"].as_str().unwrap(), "find foo");
}

#[test]
fn test_continuity_event_spec_unknown_tool() {
    let spec = continuity_event_spec("list_graph_stats", None);
    assert!(spec.is_none());
}
