use std::path::PathBuf;

use rusqlite::params;
use serde_json::Value;
use tempfile::TempDir;

use crate::SessionId;

use super::*;

fn open_store(
    max_events_per_session: usize,
    max_inline_payload_bytes: usize,
) -> (TempDir, SessionStore) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
    let store = SessionStore::open_with_config(
        path.to_str().unwrap(),
        SessionStoreConfig {
            max_events_per_session,
            max_inline_payload_bytes,
            ..Default::default()
        },
    )
    .unwrap();
    (dir, store)
}

fn session_id() -> SessionId {
    SessionId::derive("/repo", "main", "cli")
}

fn seed_session(store: &mut SessionStore, session_id: &SessionId) {
    store
        .upsert_session_meta(session_id.clone(), "/repo", "cli", Some("main"))
        .unwrap();
}

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

#[test]
fn delete_session_removes_events_and_returns_true_only_when_existed() {
    let (_dir, mut store) = open_store(16, 1024);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::CommandRun,
            priority: 2,
            payload: serde_json::json!({ "command": "build" }),
            created_at: None,
        })
        .unwrap();

    assert!(store.delete_session(&session_id).unwrap());
    assert!(store.get_session_meta(&session_id).unwrap().is_none());
    assert!(store.list_events(&session_id).unwrap().is_empty());
    assert!(!store.delete_session(&session_id).unwrap());
}

#[test]
fn build_resume_persists_and_groups_events() {
    let (_dir, mut store) = open_store(64, 8192);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    let events = vec![
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::UserIntent,
            priority: 3,
            payload: serde_json::json!({ "intent": "review" }),
            created_at: None,
        },
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::CommandRun,
            priority: 2,
            payload: serde_json::json!({ "command": "build", "status": "ok" }),
            created_at: None,
        },
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::ReasoningResult,
            priority: 3,
            payload: serde_json::json!({ "source_id": "src-abc", "summary": "impact analysis" }),
            created_at: None,
        },
    ];
    for ev in events {
        store.append_event(ev).unwrap();
    }

    let snap = store.build_resume(&session_id).unwrap();
    assert!(!snap.consumed);
    assert_eq!(snap.event_count, 3);
    assert_eq!(snap.session_id, session_id);

    let inner: serde_json::Value = serde_json::from_str(&snap.snapshot).unwrap();
    assert_eq!(inner["last_user_intent"], "review");
    assert_eq!(inner["recent_commands"].as_array().unwrap().len(), 1);
    assert!(
        inner["saved_artifact_refs"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("src-abc"))
    );
    assert_eq!(inner["event_count"], 3);
}

#[test]
fn build_resume_captures_decisions_and_deduplicates_rules_by_label() {
    let (_dir, mut store) = open_store(64, 8192);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    let events = vec![
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: serde_json::json!({ "summary": "prefer composition", "rationale": "simpler" }),
            created_at: None,
        },
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::RuleInstruction,
            priority: 4,
            payload: serde_json::json!({
                "label": "no_mut_global",
                "rule": "avoid global mutable state",
                "source": "AGENTS.md",
            }),
            created_at: None,
        },
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::RuleInstruction,
            priority: 4,
            payload: serde_json::json!({
                "label": "no_mut_global",
                "rule": "avoid global mutable state (updated)",
                "source": "AGENTS.md",
            }),
            created_at: None,
        },
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::RuleInstruction,
            priority: 4,
            payload: serde_json::json!({
                "label": "use_result",
                "rule": "use Result and ? for error propagation",
                "source": "AGENTS.md",
            }),
            created_at: None,
        },
    ];
    for ev in events {
        store.append_event(ev).unwrap();
    }

    let snap = store.build_resume(&session_id).unwrap();
    let inner: serde_json::Value = serde_json::from_str(&snap.snapshot).unwrap();

    let decisions = inner["recent_decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["summary"], "prefer composition");

    let rules = inner["active_rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2);
    let no_mut = rules
        .iter()
        .find(|r| r["label"] == "no_mut_global")
        .expect("no_mut_global rule missing");
    assert_eq!(no_mut["rule"], "avoid global mutable state (updated)");
}

#[test]
fn decision_events_are_indexed_for_lookup_with_artifact_links() {
    let (_dir, mut store) = open_store(64, 8192);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: serde_json::json!({
                "summary": "reuse saved review context",
                "rationale": "matching file and symbol overlap",
                "conclusion": "prior review still relevant",
                "query": "review src/lib.rs",
                "source_id": "src-123",
                "files": ["src/lib.rs"],
                "related_symbols": ["crate::lib::compute"],
                "evidence": [{"kind": "saved_context", "source_id": "src-123"}],
            }),
            created_at: None,
        })
        .unwrap();

    let hits = store
        .search_decisions("/repo", "review src/lib.rs", Some(session_id.as_str()), 10)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].decision.summary, "reuse saved review context");
    assert_eq!(hits[0].decision.source_ids, vec!["src-123"]);
    assert_eq!(hits[0].decision.related_files, vec!["src/lib.rs"]);
    assert_eq!(
        hits[0].decision.related_symbols,
        vec!["crate::lib::compute"]
    );
    assert!(hits[0].relevance_score > 0.0);
}

#[test]
fn decision_lookup_matches_conclusion_and_query_text() {
    let (_dir, mut store) = open_store(64, 8192);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: serde_json::json!({
                "summary": "refactor safety verdict",
                "conclusion": "safe to refactor auth::verify_token",
                "query": "verify_token",
            }),
            created_at: None,
        })
        .unwrap();

    let hits = store
        .search_decisions("/repo", "verify_token", None, 10)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(
        hits[0]
            .decision
            .conclusion
            .as_deref()
            .unwrap()
            .contains("verify_token")
    );
    assert!(
        hits[0]
            .matched_terms
            .iter()
            .any(|term| term == "verify_token")
    );
}

#[test]
fn build_resume_canonicalizes_changed_files() {
    let (_dir, mut store) = open_store(64, 8192);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::ReviewContext,
            priority: 3,
            payload: serde_json::json!({
                "files": ["/repo/src/lib.rs", "src/lib.rs", "./src/../src/lib.rs"]
            }),
            created_at: None,
        })
        .unwrap();

    let snap = store.build_resume(&session_id).unwrap();
    let inner: serde_json::Value = serde_json::from_str(&snap.snapshot).unwrap();
    assert_eq!(inner["changed_files"], serde_json::json!(["src/lib.rs"]));
}

#[test]
fn stats_returns_accurate_counts() {
    let (_dir, mut store) = open_store(16, 1024);
    let id_a = SessionId::derive("/repo/a", "", "cli");
    let id_b = SessionId::derive("/repo/b", "", "mcp");

    store
        .upsert_session_meta(id_a.clone(), "/repo/a", "cli", None)
        .unwrap();
    store
        .upsert_session_meta(id_b.clone(), "/repo/b", "mcp", None)
        .unwrap();

    store
        .append_event(NewSessionEvent {
            session_id: id_a.clone(),
            event_type: SessionEventType::CommandRun,
            priority: 2,
            payload: serde_json::json!({ "command": "build" }),
            created_at: None,
        })
        .unwrap();
    store
        .append_event(NewSessionEvent {
            session_id: id_b.clone(),
            event_type: SessionEventType::CommandRun,
            priority: 2,
            payload: serde_json::json!({ "command": "update" }),
            created_at: None,
        })
        .unwrap();

    store.put_resume_snapshot(&id_a, "{}", 1, false).unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.session_count, 2);
    assert_eq!(stats.total_events, 2);
    assert_eq!(stats.snapshot_count, 1);
}

#[test]
fn cleanup_stale_sessions_removes_old_entries() {
    let (_dir, mut store) = open_store(16, 1024);
    let id = SessionId::derive("/repo/stale", "", "cli");

    store
        .conn
        .execute(
            "INSERT INTO session_meta
             (session_id, repo_root, frontend, worktree_id, created_at, updated_at)
             VALUES (?1, ?2, 'cli', NULL, ?3, ?3)",
            params![id.as_str(), "/repo/stale", "2020-01-01T00:00:00Z"],
        )
        .unwrap();

    let removed = store.cleanup_stale_sessions(30).unwrap();
    assert_eq!(removed, 1, "old session should be removed");
    assert!(store.get_session_meta(&id).unwrap().is_none());
}

#[test]
fn cleanup_stale_sessions_keeps_recent_sessions() {
    let (_dir, mut store) = open_store(16, 1024);
    let id = SessionId::derive("/repo/fresh", "", "cli");
    store
        .upsert_session_meta(id.clone(), "/repo/fresh", "cli", None)
        .unwrap();
    let removed = store.cleanup_stale_sessions(30).unwrap();
    assert_eq!(removed, 0, "recent session must not be removed");
    assert!(store.get_session_meta(&id).unwrap().is_some());
}

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
fn dedup_window_blocks_same_type_within_window() {
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

    let second = store.append_event(mk("build-again")).unwrap();
    assert!(
        second.is_none(),
        "same-type event inside window should be deduped"
    );

    let events = store.list_events(&session_id).unwrap();
    assert_eq!(events.len(), 1, "only one event should be stored");
}

#[test]
fn best_effort_open_in_nonexistent_dir_creates_path() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("deep").join("nested").join(".atlas");
    let path = nested.join(DEFAULT_SESSION_DB);
    let result = SessionStore::open(path.to_str().unwrap());
    assert!(result.is_ok(), "store open must create missing dirs");
}

#[test]
fn corrupt_db_is_quarantined_on_open() {
    let dir = TempDir::new().unwrap();
    let atlas_dir = dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).unwrap();
    let path = atlas_dir.join(DEFAULT_SESSION_DB);

    std::fs::write(&path, b"this is not a sqlite database").unwrap();

    let result = SessionStore::open(path.to_str().unwrap());
    assert!(result.is_err(), "corrupt DB must return error");

    let quarantine = atlas_dir.join(format!("{}.quarantine", DEFAULT_SESSION_DB));
    assert!(
        quarantine.exists(),
        "quarantine file must be created for corrupt DB"
    );
}

#[test]
fn quarantine_allows_fresh_open_after_corruption() {
    let dir = TempDir::new().unwrap();
    let atlas_dir = dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).unwrap();
    let path = atlas_dir.join(DEFAULT_SESSION_DB);

    std::fs::write(&path, b"not a database").unwrap();
    let _ = SessionStore::open(path.to_str().unwrap());

    let store = SessionStore::open(path.to_str().unwrap());
    assert!(
        store.is_ok(),
        "fresh open after quarantine must succeed: {:?}",
        store.err()
    );
}

#[test]
fn is_corruption_error_matches_known_strings() {
    let cases = [
        "database disk image is malformed",
        "file is not a database",
        "not a database",
    ];
    for msg in cases {
        let err = atlas_core::AtlasError::Db(msg.to_string());
        assert!(
            util::is_corruption_error(&err),
            "must detect corruption in: {msg}"
        );
    }
}

#[test]
fn is_corruption_error_does_not_match_normal_errors() {
    let err = atlas_core::AtlasError::Db("disk I/O error (SQLITE_IOERR)".to_string());
    assert!(!util::is_corruption_error(&err));
}

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
