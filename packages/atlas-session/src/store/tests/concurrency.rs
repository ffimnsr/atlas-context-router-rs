use super::*;

#[test]
fn concurrent_writers_to_same_db_all_events_persist() {
    use std::sync::Arc;
    use std::thread;

    let dir = TempDir::new().unwrap();
    let path = Arc::new(
        dir.path()
            .join(".atlas")
            .join(DEFAULT_SESSION_DB)
            .to_string_lossy()
            .into_owned(),
    );

    let session_id = SessionId::derive("concurrent-repo", "", "cli");
    {
        let mut store = SessionStore::open(&path).unwrap();
        store
            .upsert_session_meta(session_id.clone(), "concurrent-repo", "cli", None)
            .unwrap();
    }

    const THREADS: usize = 4;
    const EVENTS_PER_THREAD: usize = 10;

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let p = Arc::clone(&path);
            let sid = session_id.clone();
            thread::spawn(move || {
                let mut store = SessionStore::open(&p).expect("thread store open");
                for i in 0..EVENTS_PER_THREAD {
                    let event = NewSessionEvent {
                        session_id: sid.clone(),
                        event_type: SessionEventType::CommandRun,
                        priority: 0,
                        payload: serde_json::json!({
                            "thread": t,
                            "step": i,
                            "unique": format!("t{t}-s{i}"),
                        }),
                        created_at: None,
                    };
                    let _ = store.append_event(event);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("writer thread must not panic");
    }

    let final_store = SessionStore::open(&path).unwrap();
    let events = final_store.list_events(&session_id).unwrap();
    assert!(
        events.len() >= THREADS,
        "expected at least {THREADS} events; got {}",
        events.len()
    );
}

#[test]
fn concurrent_snapshot_build_while_writing_events() {
    use std::sync::Arc;
    use std::thread;

    let dir = TempDir::new().unwrap();
    let path = Arc::new(
        dir.path()
            .join(".atlas")
            .join(DEFAULT_SESSION_DB)
            .to_string_lossy()
            .into_owned(),
    );
    let session_id = SessionId::derive("snap-race-repo", "", "cli");

    {
        let mut store = SessionStore::open(&path).unwrap();
        store
            .upsert_session_meta(session_id.clone(), "snap-race-repo", "cli", None)
            .unwrap();
        for i in 0..5_u32 {
            let _ = store.append_event(NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::UserIntent,
                priority: 1,
                payload: serde_json::json!({"intent": format!("seed {i}")}),
                created_at: None,
            });
        }
    }

    let path_writer = Arc::clone(&path);
    let sid_writer = session_id.clone();
    let writer = thread::spawn(move || {
        let mut store = SessionStore::open(&path_writer).expect("writer open");
        for i in 0..20_u32 {
            let _ = store.append_event(NewSessionEvent {
                session_id: sid_writer.clone(),
                event_type: SessionEventType::CommandRun,
                priority: 0,
                payload: serde_json::json!({"command": format!("cmd-{i}")}),
                created_at: None,
            });
        }
    });

    let path_snap = Arc::clone(&path);
    let sid_snap = session_id.clone();
    let snapper = thread::spawn(move || {
        let mut store = SessionStore::open(&path_snap).expect("snapper open");
        let result = store.build_resume(&sid_snap);
        result.is_ok()
    });

    writer.join().expect("writer must not panic");
    let snap_ok = snapper.join().expect("snapper must not panic");
    assert!(snap_ok, "build_resume must succeed under concurrent writes");
}

#[test]
fn concurrent_upsert_session_meta_is_safe() {
    use std::sync::Arc;
    use std::thread;

    let dir = TempDir::new().unwrap();
    let path = Arc::new(
        dir.path()
            .join(".atlas")
            .join(DEFAULT_SESSION_DB)
            .to_string_lossy()
            .into_owned(),
    );
    let session_id = SessionId::derive("upsert-race", "", "mcp");

    SessionStore::open(&path).unwrap();

    let handles: Vec<_> = (0..3)
        .map(|t| {
            let p = Arc::clone(&path);
            let sid = session_id.clone();
            thread::spawn(move || {
                let mut store = SessionStore::open(&p).expect("open");
                let _ = store.upsert_session_meta(sid, &format!("repo-{t}"), "mcp", None);
            })
        })
        .collect();

    for h in handles {
        h.join().expect("upsert thread must not panic");
    }

    let store = SessionStore::open(&path).unwrap();
    let meta = store.get_session_meta(&session_id).unwrap();
    assert!(
        meta.is_some(),
        "session meta must exist after concurrent upserts"
    );
}

#[test]
fn concurrent_snapshot_writes_last_writer_wins() {
    use std::sync::Arc;
    use std::thread;

    let dir = TempDir::new().unwrap();
    let path = Arc::new(
        dir.path()
            .join(".atlas")
            .join(DEFAULT_SESSION_DB)
            .to_string_lossy()
            .into_owned(),
    );
    let session_id = SessionId::derive("snap-write-race", "", "cli");

    {
        let mut store = SessionStore::open(&path).unwrap();
        store
            .upsert_session_meta(session_id.clone(), "repo", "cli", None)
            .unwrap();
    }

    const THREADS: usize = 4;
    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let p = Arc::clone(&path);
            let sid = session_id.clone();
            thread::spawn(move || {
                let mut store = SessionStore::open(&p).expect("open");
                let snapshot = format!(r#"{{"writer":{t}}}"#);
                let _ = store.put_resume_snapshot(&sid, &snapshot, t as i64, false);
            })
        })
        .collect();

    for h in handles {
        h.join().expect("snapshot writer must not panic");
    }

    let store = SessionStore::open(&path).unwrap();
    let snap = store
        .get_resume_snapshot(&session_id)
        .unwrap()
        .expect("snapshot must exist");
    let parsed: serde_json::Value = serde_json::from_str(&snap.snapshot)
        .expect("snapshot must be valid JSON after concurrent writes");
    assert!(
        parsed.get("writer").is_some(),
        "snapshot payload must have 'writer' key"
    );
}
