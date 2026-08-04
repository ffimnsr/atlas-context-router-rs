use super::*;

// ── Curation tests ──────────────────────────────────────────────────────────

fn open_curation_store() -> (TempDir, SessionStore, SessionId) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
    let mut store = SessionStore::open_with_config(
        path.to_str().unwrap(),
        SessionStoreConfig {
            max_events_per_session: 512,
            max_inline_payload_bytes: 8192,
            ..Default::default()
        },
    )
    .unwrap();
    let session_id = SessionId::derive("/curation-repo", "main", "test");
    store
        .upsert_session_meta(session_id.clone(), "/curation-repo", "test", Some("main"))
        .unwrap();
    (dir, store, session_id)
}

fn append(
    store: &mut SessionStore,
    session_id: &SessionId,
    event_type: SessionEventType,
    priority: i32,
    payload: serde_json::Value,
) {
    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type,
            priority,
            payload,
            created_at: None,
        })
        .unwrap();
}

#[test]
fn compact_session_decays_excess_file_read_events_per_path() {
    let (_dir, mut store, session_id) = open_curation_store();

    for i in 0..5u32 {
        append(
            &mut store,
            &session_id,
            SessionEventType::FileRead,
            1,
            serde_json::json!({"file": "src/lib.rs", "run": i}),
        );
    }

    let result = store.compact_session(&session_id).unwrap();
    let events = store.list_events(&session_id).unwrap();
    let file_read_count = events
        .iter()
        .filter(|e| e.event_type == SessionEventType::FileRead)
        .count();

    assert!(
        file_read_count <= 3,
        "expected ≤3 FILE_READ events after compaction; got {file_read_count}"
    );
    assert!(result.decayed_count > 0, "decayed_count must be non-zero");
}

#[test]
fn compact_session_keeps_only_latest_graph_state_event() {
    let (_dir, mut store, session_id) = open_curation_store();

    for i in 0..4u32 {
        append(
            &mut store,
            &session_id,
            SessionEventType::GraphBuild,
            1,
            serde_json::json!({"run": i, "seq": i}),
        );
    }

    let result = store.compact_session(&session_id).unwrap();
    let events = store.list_events(&session_id).unwrap();
    let graph_count = events
        .iter()
        .filter(|e| e.event_type == SessionEventType::GraphBuild)
        .count();

    assert_eq!(graph_count, 1, "only latest GRAPH_BUILD must survive");
    assert!(result.decayed_count > 0);
}

#[test]
fn compact_session_merges_repeated_command_runs() {
    let (_dir, mut store, session_id) = open_curation_store();

    for i in 0..5u32 {
        append(
            &mut store,
            &session_id,
            SessionEventType::CommandRun,
            1,
            serde_json::json!({"command": "cargo build", "run": i}),
        );
    }

    store.compact_session(&session_id).unwrap();
    let events = store.list_events(&session_id).unwrap();
    let cmd_count = events
        .iter()
        .filter(|e| e.event_type == SessionEventType::CommandRun)
        .count();

    assert!(
        cmd_count <= 3,
        "expected ≤3 CommandRun events after compaction; got {cmd_count}"
    );
}

#[test]
fn compact_session_deduplicates_reasoning_by_source_id() {
    let (_dir, mut store, session_id) = open_curation_store();

    for i in 0..3u32 {
        append(
            &mut store,
            &session_id,
            SessionEventType::ReasoningResult,
            1,
            serde_json::json!({"source_id": "abc123", "result": i, "seq": i}),
        );
    }

    let result = store.compact_session(&session_id).unwrap();
    let events = store.list_events(&session_id).unwrap();
    let reasoning_count = events
        .iter()
        .filter(|e| e.event_type == SessionEventType::ReasoningResult)
        .count();

    assert_eq!(
        reasoning_count, 1,
        "duplicate REASONING_RESULT must collapse to 1"
    );
    assert!(result.deduplicated_count > 0);
}

#[test]
fn compact_session_promotes_decision_event_priority() {
    let (_dir, mut store, session_id) = open_curation_store();

    append(
        &mut store,
        &session_id,
        SessionEventType::Decision,
        10,
        serde_json::json!({"decision": "use tokio"}),
    );

    let result = store.compact_session(&session_id).unwrap();
    let events = store.list_events(&session_id).unwrap();
    let decision = events
        .iter()
        .find(|e| e.event_type == SessionEventType::Decision)
        .expect("decision event must still exist");

    assert_eq!(
        decision.priority, 90,
        "decision priority must be promoted to 90"
    );
    assert!(result.promoted_count > 0);
}

#[test]
fn compact_session_returns_zero_change_when_nothing_to_do() {
    let (_dir, mut store, session_id) = open_curation_store();

    // Single unique events — nothing to compact.
    append(
        &mut store,
        &session_id,
        SessionEventType::UserIntent,
        90,
        serde_json::json!({}),
    );

    let result = store.compact_session(&session_id).unwrap();
    assert_eq!(result.decayed_count, 0);
    assert_eq!(result.merged_count, 0);
    assert_eq!(result.deduplicated_count, 0);
}

#[test]
fn compact_session_updates_last_compaction_at() {
    let (_dir, mut store, session_id) = open_curation_store();
    append(
        &mut store,
        &session_id,
        SessionEventType::CommandRun,
        1,
        serde_json::json!({}),
    );

    store.compact_session(&session_id).unwrap();
    let meta = store
        .get_session_meta(&session_id)
        .unwrap()
        .expect("meta must exist");
    assert!(
        meta.last_compaction_at.is_some(),
        "last_compaction_at must be set after compaction"
    );
}
