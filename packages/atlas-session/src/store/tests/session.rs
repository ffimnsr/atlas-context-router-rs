use super::*;

#[test]
fn session_meta_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
    let session_id = session_id();

    {
        let mut store = SessionStore::open(path.to_str().unwrap()).unwrap();
        store
            .upsert_session_meta(session_id.clone(), "/repo", "cli", Some("main"))
            .unwrap();
    }

    let store = SessionStore::open(path.to_str().unwrap()).unwrap();
    let meta = store.get_session_meta(&session_id).unwrap().unwrap();
    assert_eq!(meta.repo_root, "/repo");
    assert_eq!(meta.frontend, "cli");
    assert_eq!(meta.worktree_id.as_deref(), Some("main"));
}

#[test]
fn open_stamps_session_migration_history_and_provenance() {
    let (_dir, store) = open_store(16, 1024);
    let version: i32 = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 8);

    let history_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(history_count, 8);

    let (db_kind, created_by): (String, String) = store
        .conn
        .query_row(
            "SELECT db_kind, created_by FROM atlas_provenance WHERE singleton_key = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(db_kind, "session");
    assert_eq!(created_by, format!("atlas v{}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn rollback_and_reupgrade_restore_session_schema() {
    let (_dir, mut store) = open_store(16, 1024);
    store.migrate_to(2).unwrap();
    let version: i32 = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
    let decision_exists: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'decision_memory'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(decision_exists, 0);
    assert!(table_columns(&store.conn, "session_meta").contains(&"session_id".to_string()));

    store.migrate().unwrap();
    let restored_version: i32 = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(restored_version, 8);
    let fts_exists: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'decision_memory_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fts_exists, 1);
    let memories_exists: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(memories_exists, 1);
}

#[test]
fn decision_memory_lookup_index_exists() {
    let (_dir, store) = open_store(16, 1024);

    let sql: String = store
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            params!["idx_decision_memory_repo_session_updated"],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(
        sql,
        "CREATE INDEX idx_decision_memory_repo_session_updated\n    ON decision_memory(repo_root, session_id, updated_at DESC, created_at DESC)"
    );
}

#[test]
fn concurrent_open_in_repo_on_fresh_db_is_safe() {
    let dir = TempDir::new().unwrap();
    let repo_root = Arc::new(dir.path().to_path_buf());

    let handles = (0..4)
        .map(|_| {
            let repo_root = Arc::clone(&repo_root);
            thread::spawn(move || {
                let store = SessionStore::open_in_repo(repo_root.as_path()).unwrap();
                store.schema_version().unwrap()
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        assert_eq!(handle.join().unwrap(), LATEST_VERSION);
    }
}

#[test]
fn decision_memory_fts_table_exists() {
    let (_dir, store) = open_store(16, 1024);

    let sql: String = store
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params!["decision_memory_fts"],
            |row| row.get(0),
        )
        .unwrap();

    assert!(sql.contains("USING fts5"));
}

#[test]
fn duplicate_events_deduplicate_by_hash() {
    let (_dir, mut store) = open_store(16, 1024);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    let event = NewSessionEvent {
        session_id: session_id.clone(),
        event_type: SessionEventType::FileRead,
        priority: 5,
        payload: serde_json::json!({"path":"src/lib.rs","line":12}),
        created_at: Some("2026-01-01T00:00:00Z".into()),
    };

    let first = store.append_event(event.clone()).unwrap();
    let second = store.append_event(event).unwrap();

    assert!(first.is_some());
    assert!(second.is_none());
    assert_eq!(store.list_events(&session_id).unwrap().len(), 1);
}

#[test]
fn duplicate_events_deduplicate_after_path_canonicalization() {
    let (_dir, mut store) = open_store(16, 1024);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    let first = NewSessionEvent {
        session_id: session_id.clone(),
        event_type: SessionEventType::FileRead,
        priority: 5,
        payload: serde_json::json!({"path":"src/lib.rs","line":12}),
        created_at: Some("2026-01-01T00:00:00Z".into()),
    };
    let second = NewSessionEvent {
        session_id: session_id.clone(),
        event_type: SessionEventType::FileRead,
        priority: 5,
        payload: serde_json::json!({"path":"/repo/src/lib.rs","line":12}),
        created_at: Some("2026-01-01T00:00:00Z".into()),
    };

    assert!(store.append_event(first).unwrap().is_some());
    assert!(store.append_event(second).unwrap().is_none());
    assert_eq!(store.list_events(&session_id).unwrap().len(), 1);
}

#[test]
fn retention_evicts_lower_priority_then_older() {
    let (_dir, mut store) = open_store(2, 1024);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    for (priority, created_at, label) in [
        (1, "2026-01-01T00:00:00Z", "low-old"),
        (1, "2026-01-01T00:01:00Z", "low-new"),
        (5, "2026-01-01T00:02:00Z", "high"),
    ] {
        store
            .append_event(NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::CommandRun,
                priority,
                payload: serde_json::json!({ "label": label }),
                created_at: Some(created_at.into()),
            })
            .unwrap();
    }

    let events = store.list_events(&session_id).unwrap();
    let labels = events
        .iter()
        .map(|event| {
            serde_json::from_str::<Value>(&event.payload_json).unwrap()["label"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(labels, vec!["low-new".to_string(), "high".to_string()]);
    assert!(
        store
            .get_session_meta(&session_id)
            .unwrap()
            .unwrap()
            .last_compaction_at
            .is_some()
    );
}

#[test]
fn oversize_payload_rejected() {
    let (_dir, mut store) = open_store(8, 32);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    let error = store
        .append_event(NewSessionEvent {
            session_id,
            event_type: SessionEventType::CommandFail,
            priority: 10,
            payload: serde_json::json!({ "raw_output": "x".repeat(128) }),
            created_at: None,
        })
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("store raw output in content store")
    );
}

#[test]
fn resume_snapshot_round_trip_and_consumption() {
    let (_dir, mut store) = open_store(16, 1024);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    store
        .put_resume_snapshot(&session_id, "{\"summary\":\"resume\"}", 7, false)
        .unwrap();
    store.mark_resume_consumed(&session_id, true).unwrap();

    let resume = store.get_resume_snapshot(&session_id).unwrap().unwrap();
    assert_eq!(resume.snapshot, "{\"summary\":\"resume\"}");
    assert_eq!(resume.event_count, 7);
    assert!(resume.consumed);
    assert!(
        store
            .get_session_meta(&session_id)
            .unwrap()
            .unwrap()
            .last_resume_at
            .is_some()
    );
}

#[test]
fn open_in_repo_creates_default_session_db_path() {
    let dir = TempDir::new().unwrap();
    let session_id = session_id();

    {
        let mut store = SessionStore::open_in_repo(dir.path()).unwrap();
        seed_session(&mut store, &session_id);
    }

    let expected_path: PathBuf = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
    assert!(expected_path.exists());
}

#[test]
fn list_sessions_returns_all_in_recency_order() {
    let (_dir, mut store) = open_store(16, 1024);

    let id_a = SessionId::derive("/repo/a", "", "cli");
    let id_b = SessionId::derive("/repo/b", "", "mcp");
    store
        .upsert_session_meta(id_a.clone(), "/repo/a", "cli", None)
        .unwrap();
    store
        .upsert_session_meta(id_b.clone(), "/repo/b", "mcp", None)
        .unwrap();

    let sessions = store.list_sessions().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].session_id, id_b);
    assert_eq!(sessions[1].session_id, id_a);
}
