use super::*;

fn append(store: &mut SessionStore, event_type: SessionEventType, payload: serde_json::Value) {
    store
        .append_event(NewSessionEvent {
            session_id: session_id(),
            event_type,
            priority: 1,
            payload,
            created_at: None,
        })
        .unwrap();
}

fn summary_events(store: &SessionStore) -> Vec<SessionEventRow> {
    store
        .list_events(&session_id())
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == SessionEventType::SessionSummary)
        .collect()
}

#[test]
fn compaction_writes_summary_before_deleting_events() {
    let (_dir, mut store) = open_store(64, 1024);
    seed_session(&mut store, &session_id());
    seed_memory(
        &store,
        "topic-memory",
        "2026-01-01T00:00:00Z",
        "deploy fact",
        "deploy",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    for index in 0..4 {
        append(
            &mut store,
            SessionEventType::FileRead,
            serde_json::json!({ "path": "src/lib.rs", "topic": "deploy", "seq": index }),
        );
    }
    for index in 0..4 {
        append(
            &mut store,
            SessionEventType::CommandRun,
            serde_json::json!({ "command": "cargo test --quiet", "topic": "deploy", "seq": index }),
        );
    }
    append(
        &mut store,
        SessionEventType::CommandFail,
        serde_json::json!({ "command": "cargo build", "stderr": "error" }),
    );
    append(
        &mut store,
        SessionEventType::Decision,
        serde_json::json!({ "summary": "keep postgres" }),
    );
    let decision_id = store
        .list_events(&session_id())
        .unwrap()
        .into_iter()
        .find(|event| event.event_type == SessionEventType::Decision)
        .unwrap()
        .id
        .to_string();
    append(
        &mut store,
        SessionEventType::UserIntent,
        serde_json::json!({ "prompt": "ship vm2" }),
    );

    let result = store.compact_session(&session_id()).unwrap();
    assert!(result.decayed_count + result.merged_count > 0);

    let summaries = summary_events(&store);
    assert_eq!(summaries.len(), 1, "one summary per compaction");
    let payload: serde_json::Value = serde_json::from_str(&summaries[0].payload_json).unwrap();
    assert_eq!(payload["total_reads"], serde_json::json!(1));
    assert_eq!(payload["command_runs"], serde_json::json!(1));
    assert_eq!(payload["failure_count"], serde_json::json!(0));
    assert_eq!(payload["topics"], serde_json::json!(["deploy"]));
    assert!(payload["first_seen"].is_string());
    assert!(payload["last_seen"].is_string());
    assert!(payload["source_ids"].as_array().unwrap().is_empty());

    // Protected event types survive compaction and never enter the summary.
    let remaining = store.list_events(&session_id()).unwrap();
    assert!(
        remaining
            .iter()
            .any(|event| event.event_type == SessionEventType::Decision)
    );
    assert!(
        remaining
            .iter()
            .any(|event| event.event_type == SessionEventType::UserIntent)
    );
    assert!(
        remaining
            .iter()
            .any(|event| event.event_type == SessionEventType::CommandFail),
        "unrelated failure events stay in the ledger"
    );
    let event_ids = payload["event_ids"].as_array().unwrap();
    assert!(
        !event_ids
            .iter()
            .any(|id| id.as_str() == Some(decision_id.as_str())),
        "DECISION rows must never enter a compressed group"
    );

    // TimeCompress stamps a deterministic rollup on the topic entity.
    let metadata_json: String = store
        .conn
        .query_row(
            "SELECT metadata_json FROM memories WHERE id = 'topic-memory'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata_json).unwrap();
    assert_eq!(
        metadata["time_compress"]["window_label"],
        serde_json::json!("session_compaction")
    );
}

#[test]
fn second_compaction_is_idempotent() {
    let (_dir, mut store) = open_store(64, 1024);
    seed_session(&mut store, &session_id());
    for index in 0..5 {
        append(
            &mut store,
            SessionEventType::FileRead,
            serde_json::json!({ "path": "src/lib.rs", "seq": index }),
        );
    }

    let result = store.compact_session(&session_id()).unwrap();
    assert_eq!(result.decayed_count, 2);
    assert_eq!(summary_events(&store).len(), 1);

    let second = store.compact_session(&session_id()).unwrap();
    assert_eq!(second.decayed_count, 0);
    assert_eq!(second.merged_count, 0);
    assert_eq!(
        summary_events(&store).len(),
        1,
        "no new summary when nothing is evicted"
    );
}
