use super::*;

#[test]
fn snapshot_size_cap_trims_bulky_buckets() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
    let mut store = SessionStore::open_with_config(
        path.to_str().unwrap(),
        SessionStoreConfig {
            max_events_per_session: 256,
            max_inline_payload_bytes: 8192,
            max_snapshot_bytes: 512,
            dedup_window_secs: 0,
        },
    )
    .unwrap();

    let session_id = session_id();
    seed_session(&mut store, &session_id);

    for i in 0..50 {
        store
            .append_event(NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::ImpactAnalysis,
                priority: 2,
                payload: serde_json::json!({
                    "symbols": (0..20).map(|j| format!("Symbol_{i}_{j}")).collect::<Vec<_>>(),
                }),
                created_at: None,
            })
            .unwrap();
    }

    let snap = store.build_resume(&session_id).unwrap();
    assert!(
        snap.snapshot.len() <= 512,
        "snapshot len {} exceeds cap 512",
        snap.snapshot.len()
    );
}

#[test]
fn dedup_window_blocks_same_hash_within_window() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
    let mut store = SessionStore::open_with_config(
        path.to_str().unwrap(),
        SessionStoreConfig {
            max_events_per_session: 64,
            max_inline_payload_bytes: 8192,
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
            dedup_window_secs: 60,
        },
    )
    .unwrap();

    let session_id = session_id();
    seed_session(&mut store, &session_id);

    let mk = |label: &str| NewSessionEvent {
        session_id: session_id.clone(),
        event_type: SessionEventType::CommandRun,
        priority: 2,
        payload: serde_json::json!({ "command": label }),
        created_at: None,
    };

    let first = store.append_event(mk("build")).unwrap();
    assert!(first.is_some(), "first event should be stored");

    let second = store.append_event(mk("build")).unwrap();
    assert!(
        second.is_none(),
        "same event hash inside window should be deduped"
    );

    let events = store.list_events(&session_id).unwrap();
    assert_eq!(events.len(), 1, "only one event should be stored");
}

#[test]
fn dedup_window_keeps_different_payloads_within_window() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
    let mut store = SessionStore::open_with_config(
        path.to_str().unwrap(),
        SessionStoreConfig {
            max_events_per_session: 64,
            max_inline_payload_bytes: 8192,
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
            dedup_window_secs: 60,
        },
    )
    .unwrap();

    let session_id = session_id();
    seed_session(&mut store, &session_id);

    let mk = |label: &str| NewSessionEvent {
        session_id: session_id.clone(),
        event_type: SessionEventType::CommandRun,
        priority: 2,
        payload: serde_json::json!({ "command": label }),
        created_at: None,
    };

    assert!(store.append_event(mk("build")).unwrap().is_some());
    assert!(store.append_event(mk("build-again")).unwrap().is_some());

    let events = store.list_events(&session_id).unwrap();
    assert_eq!(events.len(), 2, "distinct payloads should not be dropped");
}
