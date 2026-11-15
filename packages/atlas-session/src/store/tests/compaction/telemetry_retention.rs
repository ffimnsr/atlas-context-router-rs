use super::*;
use crate::store::curation::MAX_TOOL_TELEMETRY_EVENTS;

fn append_telemetry(store: &mut SessionStore, session_id: &SessionId, status: &str) {
    append(
        store,
        session_id,
        SessionEventType::ContextRequest,
        0,
        serde_json::json!({"tool": "status", "status": status, "extra": null}),
    );
}

/// Per-call tool telemetry is not curated memory: a compaction triggered right
/// after another one must stay a no-op even though the wrapper recorded
/// `start`/`ok` rows for the previous call.
#[test]
fn compact_session_immediate_repeat_is_no_op_with_tool_telemetry() {
    let (_dir, mut store, session_id) = open_curation_store();

    append_telemetry(&mut store, &session_id, "start");
    append_telemetry(&mut store, &session_id, "ok");
    let first = store.compact_session(&session_id).unwrap();
    assert!(
        first.no_op,
        "tool telemetry alone must not be curated: {first:?}"
    );

    append_telemetry(&mut store, &session_id, "start");
    append_telemetry(&mut store, &session_id, "ok");
    let second = store.compact_session(&session_id).unwrap();
    assert!(
        second.no_op,
        "repeat compaction over unchanged curated ledger must be a no-op: {second:?}"
    );
    assert_eq!(second.decayed_count, 0);
}

/// Telemetry retention is bounded so the ledger cannot grow without limit.
#[test]
fn compact_session_bounds_tool_telemetry_retention() {
    let (_dir, mut store, session_id) = open_curation_store();
    let overflow = 5;
    for index in 0..(MAX_TOOL_TELEMETRY_EVENTS + overflow) {
        append(
            &mut store,
            &session_id,
            SessionEventType::ContextRequest,
            0,
            serde_json::json!({
                "tool": "status",
                "status": "ok",
                "seq": index,
            }),
        );
    }

    let result = store.compact_session(&session_id).unwrap();
    let remaining = store
        .list_events(&session_id)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == SessionEventType::ContextRequest)
        .count();

    assert_eq!(result.decayed_count, overflow);
    assert_eq!(remaining, MAX_TOOL_TELEMETRY_EVENTS);
    assert!(
        store
            .list_events(&session_id)
            .unwrap()
            .iter()
            .all(|event| event.event_type != SessionEventType::SessionSummary),
        "telemetry-only eviction must not write a summary row"
    );
}
