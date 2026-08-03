use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use rusqlite::params;
use serde_json::Value;
use tempfile::TempDir;

use crate::SessionId;

use super::*;

// Compile-time enforcement: `SessionStore` must not implement `Send` or `Sync`.
//
// `SessionStore` carries `PhantomData<*const ()>` which explicitly opts it out
// of `Send` and `Sync` auto-traits, enforcing thread confinement at the
// compiler level regardless of what `rusqlite::Connection` implements.
static_assertions::assert_not_impl_any!(SessionStore: Send);
static_assertions::assert_not_impl_any!(SessionStore: Sync);

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

fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA table_info('{table}')");
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
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

fn create_durable_task_with_times(
    store: &mut SessionStore,
    task_id: &str,
    created_at: &str,
    updated_at: &str,
) {
    store
        .create_durable_task(&NewDurableTask {
            task_id: task_id.to_owned(),
            originating_method: "tools/call".to_owned(),
            request_id: Some(task_id.to_owned()),
            tool_name: Some("doctor".to_owned()),
            transport_kind: Some("stdio".to_owned()),
            session_id: None,
            status: DurableTaskStatus::Working,
            status_message: Some("working".to_owned()),
            ttl_ms: Some(1000),
        })
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE durable_tasks SET created_at = ?2, updated_at = ?3 WHERE task_id = ?1",
            params![task_id, created_at, updated_at],
        )
        .unwrap();
}

#[test]
fn durable_task_round_trips_input_requests_and_request_state() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .create_durable_task(&NewDurableTask {
            task_id: "task-input".to_owned(),
            originating_method: "tools/call".to_owned(),
            request_id: Some("request-1".to_owned()),
            tool_name: Some("purge_saved_context".to_owned()),
            transport_kind: Some("rmcp".to_owned()),
            session_id: None,
            status: DurableTaskStatus::Working,
            status_message: Some("working".to_owned()),
            ttl_ms: Some(1000),
        })
        .unwrap();
    store
        .update_durable_task(
            "task-input",
            &DurableTaskUpdate {
                status: Some(DurableTaskStatus::InputRequired),
                status_message: Some("input required".to_owned()),
                input_requests: Some(serde_json::json!({
                    "confirmation": {
                        "method": "elicitation/create",
                        "params": {
                            "message": "Confirm destructive action",
                            "requestedSchema": {
                                "type": "object",
                                "properties": {
                                    "confirmation": { "type": "string" }
                                },
                                "required": ["confirmation"]
                            }
                        }
                    }
                })),
                request_state: Some("sealed-request-state".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();

    let task = store.get_durable_task("task-input").unwrap().unwrap();
    assert_eq!(task.status, DurableTaskStatus::InputRequired);
    assert_eq!(task.request_state.as_deref(), Some("sealed-request-state"));
    assert_eq!(
        task.input_requests
            .as_ref()
            .and_then(|value| value.get("confirmation")),
        Some(&serde_json::json!({
            "method": "elicitation/create",
            "params": {
                "message": "Confirm destructive action",
                "requestedSchema": {
                    "type": "object",
                    "properties": {
                        "confirmation": { "type": "string" }
                    },
                    "required": ["confirmation"]
                }
            }
        }))
    );
}

#[test]
fn list_durable_tasks_uses_recency_order_and_stable_cursor() {
    let (_dir, mut store) = open_store(16, 1024);
    create_durable_task_with_times(
        &mut store,
        "task-older-id",
        "2026-01-01T00:00:00Z",
        "2026-01-01T00:00:05Z",
    );
    create_durable_task_with_times(
        &mut store,
        "task-newer-id",
        "2026-01-01T00:00:01Z",
        "2026-01-01T00:00:05Z",
    );
    create_durable_task_with_times(
        &mut store,
        "task-newest",
        "2026-01-01T00:00:02Z",
        "2026-01-01T00:00:06Z",
    );

    let first_page = store.list_durable_tasks(None, 2).unwrap();
    let first_ids = first_page
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(first_ids, vec!["task-newest", "task-newer-id"]);

    let next_cursor = first_page.next_cursor.expect("next cursor present");
    let cursor_json: Value = serde_json::from_str(&next_cursor).unwrap();
    assert_eq!(
        cursor_json["task_id"],
        Value::String("task-newer-id".to_owned())
    );

    let second_page = store.list_durable_tasks(Some(&next_cursor), 2).unwrap();
    let second_ids = second_page
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(second_ids, vec!["task-older-id"]);
    assert!(second_page.next_cursor.is_none());
}

#[test]
fn list_durable_tasks_rejects_malformed_cursor() {
    let (_dir, store) = open_store(16, 1024);
    let error = store.list_durable_tasks(Some("not-json"), 5).unwrap_err();
    assert!(error.to_string().contains("invalid durable task cursor"));
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
fn decision_lookup_fts_ranks_summary_match_first() {
    let (_dir, mut store) = open_store(64, 8192);
    let session_id = session_id();
    seed_session(&mut store, &session_id);

    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: serde_json::json!({
                "summary": "verify token rollout plan",
                "rationale": "migration sequence for auth service",
            }),
            created_at: Some("2026-01-01T00:00:00Z".into()),
        })
        .unwrap();
    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: serde_json::json!({
                "summary": "auth migration plan",
                "rationale": "verify token fallback compatibility",
            }),
            created_at: Some("2026-01-01T00:01:00Z".into()),
        })
        .unwrap();

    let hits = store
        .search_decisions("/repo", "verify token", None, 10)
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].decision.summary, "verify token rollout plan");
}

#[test]
fn decision_lookup_falls_back_to_sql_prefilter_when_fts_missing() {
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
                "conclusion": "use saved review for src/lib.rs",
                "query": "review src/lib.rs",
                "files": ["src/lib.rs"],
            }),
            created_at: Some("2026-01-01T00:00:00Z".into()),
        })
        .unwrap();

    store
        .conn
        .execute_batch("DROP TABLE decision_memory_fts")
        .unwrap();

    let hits = store
        .search_decisions("/repo", "src/lib.rs", Some(session_id.as_str()), 10)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].decision.summary, "reuse saved review context");
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

// ── ICM-A1 — shared memory model and storage schema ──────────────────────────

#[test]
fn memories_table_schema_matches_golden() {
    let (_dir, store) = open_store(16, 1024);

    let columns = table_columns(&store.conn, "memories");
    assert_eq!(
        columns,
        vec![
            "id",
            "repo_root",
            "session_id",
            "frontend",
            "scope",
            "topic",
            "title",
            "body",
            "importance",
            "created_at",
            "updated_at",
            "last_accessed_at",
            "decay_score",
            "source_id",
            "metadata_json",
        ]
    );

    let sql: String = store
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sql.contains("CHECK (scope IN ('project', 'session', 'frontend', 'global'))"));
    assert!(sql.contains("CHECK (importance IN ('critical', 'high', 'normal', 'low'))"));
    assert!(sql.contains("scope <> 'frontend' OR (frontend IS NOT NULL AND frontend <> '')"));
    assert!(sql.contains("scope <> 'session' OR (session_id IS NOT NULL AND session_id <> '')"));
    assert!(sql.contains("decay_score      REAL NOT NULL DEFAULT 0"));
}

#[test]
fn memories_indexes_match_golden() {
    let (_dir, store) = open_store(16, 1024);

    for (name, expected) in [
        (
            "idx_memories_repo_topic",
            "CREATE INDEX idx_memories_repo_topic\n    ON memories(repo_root, topic)",
        ),
        (
            "idx_memories_repo_importance",
            "CREATE INDEX idx_memories_repo_importance\n    ON memories(repo_root, importance)",
        ),
        (
            "idx_memories_repo_scope",
            "CREATE INDEX idx_memories_repo_scope\n    ON memories(repo_root, scope)",
        ),
        (
            "idx_memories_repo_session",
            "CREATE INDEX idx_memories_repo_session\n    ON memories(repo_root, session_id)",
        ),
        (
            "idx_memories_repo_accessed",
            "CREATE INDEX idx_memories_repo_accessed\n    ON memories(repo_root, last_accessed_at DESC)",
        ),
    ] {
        let sql: String = store
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params![name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sql, expected, "index {name} must match golden SQL");
    }
}

#[test]
fn memory_schema_issues_are_empty_when_healthy() {
    let (_dir, store) = open_store(16, 1024);
    assert!(store.memory_schema_issues().is_empty());
}

#[test]
fn memory_schema_issues_detect_missing_table_and_indexes() {
    let (_dir, store) = open_store(16, 1024);
    store.conn.execute_batch("DROP TABLE memories").unwrap();
    let issues = store.memory_schema_issues();
    assert_eq!(issues, vec!["missing table: memories"]);

    // Re-create the table without indexes: every index must be reported.
    store
        .conn
        .execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                repo_root TEXT NOT NULL,
                session_id TEXT,
                frontend TEXT,
                scope TEXT NOT NULL DEFAULT 'project',
                topic TEXT NOT NULL DEFAULT '',
                title TEXT NOT NULL DEFAULT '',
                body TEXT NOT NULL,
                importance TEXT NOT NULL DEFAULT 'normal',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_accessed_at TEXT NOT NULL,
                decay_score REAL NOT NULL DEFAULT 0,
                source_id TEXT,
                metadata_json TEXT NOT NULL DEFAULT '{}'
            )",
        )
        .unwrap();
    let issues = store.memory_schema_issues();
    assert_eq!(
        issues,
        vec![
            "missing index: idx_memories_repo_topic",
            "missing index: idx_memories_repo_importance",
            "missing index: idx_memories_repo_scope",
            "missing index: idx_memories_repo_session",
            "missing index: idx_memories_repo_accessed",
        ]
    );
}

#[test]
fn memory_importance_and_scope_parse_rejects_unknown_values() {
    for (value, parsed) in [
        ("critical", MemoryImportance::Critical),
        ("high", MemoryImportance::High),
        ("normal", MemoryImportance::Normal),
        ("low", MemoryImportance::Low),
    ] {
        assert_eq!(value.parse::<MemoryImportance>().unwrap(), parsed);
    }
    for value in ["urgent", "CRITICAL", "", "normal "] {
        assert!(
            value.parse::<MemoryImportance>().is_err(),
            "{value:?} must be rejected"
        );
    }

    for (value, parsed) in [
        ("project", MemoryScope::Project),
        ("session", MemoryScope::Session),
        ("frontend", MemoryScope::Frontend),
        ("global", MemoryScope::Global),
    ] {
        assert_eq!(value.parse::<MemoryScope>().unwrap(), parsed);
    }
    for value in ["org", "workspace", "PROJECT", ""] {
        assert!(
            value.parse::<MemoryScope>().is_err(),
            "{value:?} must be rejected"
        );
    }
}

#[test]
fn memory_json_deserialization_rejects_unknown_importance_and_scope() {
    let unknown_importance = serde_json::from_str::<NewMemory>(
        r#"{"repo_root":"/repo","body":"x","importance":"urgent"}"#,
    );
    assert!(unknown_importance.is_err(), "unknown importance rejected");

    let unknown_scope =
        serde_json::from_str::<NewMemory>(r#"{"repo_root":"/repo","body":"x","scope":"org"}"#);
    assert!(unknown_scope.is_err(), "unknown scope rejected");
}

#[test]
fn new_memory_defaults_to_normal_importance_and_project_scope() {
    let input =
        serde_json::from_str::<NewMemory>(r#"{"repo_root":"/repo","body":"remember this"}"#)
            .unwrap();
    assert_eq!(input.importance, MemoryImportance::Normal);
    assert_eq!(input.scope, MemoryScope::Project);
    assert_eq!(input.metadata, serde_json::json!({}));
    assert!(input.validate().is_ok());
}

#[test]
fn new_memory_validation_requires_frontend_for_frontend_scope_and_session_for_session_scope() {
    let base = serde_json::from_str::<NewMemory>(r#"{"repo_root":"/repo","body":"remember this"}"#)
        .unwrap();

    let frontend_scoped = NewMemory {
        scope: MemoryScope::Frontend,
        ..base.clone()
    };
    let error = frontend_scoped.validate().unwrap_err().to_string();
    assert!(
        error.contains("frontend identifier"),
        "error must name the missing frontend: {error}"
    );

    let frontend_scoped_ok = NewMemory {
        scope: MemoryScope::Frontend,
        frontend: Some("codex".to_owned()),
        ..base.clone()
    };
    assert!(frontend_scoped_ok.validate().is_ok());

    let session_scoped = NewMemory {
        scope: MemoryScope::Session,
        ..base.clone()
    };
    let error = session_scoped.validate().unwrap_err().to_string();
    assert!(
        error.contains("session_id"),
        "error must name the missing session_id: {error}"
    );

    let session_scoped_ok = NewMemory {
        scope: MemoryScope::Session,
        session_id: Some("s1".to_owned()),
        ..base
    };
    assert!(session_scoped_ok.validate().is_ok());
}

#[test]
fn memory_storage_boundary_rejects_unknown_importance_and_scope() {
    let (_dir, store) = open_store(16, 1024);

    let insert = |importance: &str, scope: &str| -> rusqlite::Result<()> {
        store
            .conn
            .execute(
                "INSERT INTO memories
                (id, repo_root, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, metadata_json)
             VALUES ('m1', '/repo', ?1, 'hooks', 'T', 'body', ?2,
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 0, '{}')",
                params![scope, importance],
            )
            .map(|_| ())
    };

    let unknown_importance = insert("urgent", "project").unwrap_err().to_string();
    assert!(
        unknown_importance.contains("CHECK constraint failed"),
        "unknown importance must fail at storage: {unknown_importance}"
    );
    let unknown_scope = insert("normal", "org").unwrap_err().to_string();
    assert!(
        unknown_scope.contains("CHECK constraint failed"),
        "unknown scope must fail at storage: {unknown_scope}"
    );

    // Session-scoped memory without session id and frontend-scoped memory
    // without frontend identifier are rejected by the storage boundary too.
    let session_without_id = store
        .conn
        .execute(
            "INSERT INTO memories
                (id, repo_root, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, metadata_json)
             VALUES ('m2', '/repo', 'session', 'hooks', 'T', 'body', 'normal',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 0, '{}')",
            [],
        )
        .unwrap_err()
        .to_string();
    assert!(
        session_without_id.contains("CHECK constraint failed"),
        "session scope without session_id must fail: {session_without_id}"
    );
    let frontend_without_name = store
        .conn
        .execute(
            "INSERT INTO memories
                (id, repo_root, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, metadata_json)
             VALUES ('m3', '/repo', 'frontend', 'hooks', 'T', 'body', 'normal',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 0, '{}')",
            [],
        )
        .unwrap_err()
        .to_string();
    assert!(
        frontend_without_name.contains("CHECK constraint failed"),
        "frontend scope without frontend must fail: {frontend_without_name}"
    );

    // Valid rows with every scope and importance value are accepted.
    store
        .conn
        .execute_batch(
            r#"INSERT INTO memories
                (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json)
             VALUES
                ('m4', '/repo', NULL, NULL, 'project', 'hooks', 'T', 'b', 'critical',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0.5, 'src-1', '{"k":1}'),
                ('m5', '/repo', NULL, NULL, 'global', 'hooks', 'T', 'b', 'high',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, NULL, '{}'),
                ('m6', '/repo', 's1', NULL, 'session', 'hooks', 'T', 'b', 'normal',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, NULL, '{}'),
                ('m7', '/repo', NULL, 'codex', 'frontend', 'hooks', 'T', 'b', 'low',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, NULL, '{}')"#,
        )
        .unwrap();
}

#[test]
fn memory_row_round_trips_through_record_shape() {
    let (_dir, store) = open_store(16, 1024);
    store
        .conn
        .execute_batch(
            r#"INSERT INTO memories
                (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json)
             VALUES ('m1', '/repo', 's1', 'codex', 'frontend', 'hooks', 'Hook notes', 'body text',
                     'critical', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z',
                     '2026-01-03T00:00:00Z', 0.25, 'src-9', '{"source_kind":"hook"}')"#,
        )
        .unwrap();

    let record = store
        .conn
        .query_row(
            "SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                    created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json
             FROM memories WHERE id = 'm1'",
            [],
            super::memory::row_to_memory,
        )
        .unwrap();
    assert_eq!(
        record,
        MemoryRecord {
            id: "m1".to_owned(),
            repo_root: "/repo".to_owned(),
            session_id: Some("s1".to_owned()),
            frontend: Some("codex".to_owned()),
            scope: MemoryScope::Frontend,
            topic: "hooks".to_owned(),
            title: "Hook notes".to_owned(),
            body: "body text".to_owned(),
            importance: MemoryImportance::Critical,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-02T00:00:00Z".to_owned(),
            last_accessed_at: "2026-01-03T00:00:00Z".to_owned(),
            decay_score: 0.25,
            source_id: Some("src-9".to_owned()),
            metadata: serde_json::json!({ "source_kind": "hook" }),
        }
    );
}

// ── ICM-A2 — memory CRUD storage layer ────────────────────────────────────────

fn cli_viewer() -> MemoryViewer {
    MemoryViewer {
        frontend: "cli".to_owned(),
        session_id: "s1".to_owned(),
    }
}

#[test]
fn recall_memories_enforces_session_and_frontend_visibility() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "p",
        "2026-01-01T00:00:00Z",
        "project body",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "g",
        "2026-01-01T00:00:00Z",
        "global body",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Global,
    );
    seed_memory(
        &store,
        "s",
        "2026-01-01T00:00:00Z",
        "session body",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Session,
    );
    seed_memory(
        &store,
        "f",
        "2026-01-01T00:00:00Z",
        "frontend body",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Frontend,
    );

    let recall_ids = |viewer: &MemoryViewer| -> Vec<String> {
        let mut ids = store
            .recall_memories(
                "/repo",
                "body",
                &MemoryListFilter::default(),
                false,
                viewer,
                10,
            )
            .unwrap()
            .iter()
            .map(|hit| hit.memory.id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    };

    // Same session + same frontend: everything is visible.
    assert_eq!(
        recall_ids(&MemoryViewer {
            frontend: "codex".to_owned(),
            session_id: "s1".to_owned(),
        }),
        vec!["f", "g", "p", "s"]
    );

    // Same session, different frontend: frontend-scoped memory hidden.
    assert_eq!(
        recall_ids(&cli_viewer()),
        vec!["g", "p", "s"],
        "session visible to same session; frontend hidden from other frontends"
    );

    // Same frontend, different session: session-scoped memory hidden.
    assert_eq!(
        recall_ids(&MemoryViewer {
            frontend: "codex".to_owned(),
            session_id: "other".to_owned(),
        }),
        vec!["f", "g", "p"],
        "frontend visible to same frontend; session hidden from other sessions"
    );

    // Different session AND different frontend: only project + global remain.
    assert_eq!(
        recall_ids(&MemoryViewer {
            frontend: "claude".to_owned(),
            session_id: "other".to_owned(),
        }),
        vec!["g", "p"],
        "unrelated viewers see only project and global memories"
    );
}

#[test]
fn recall_memories_shared_excludes_own_session_and_frontend_memories() {
    let (_dir, store) = open_store(16, 1024);
    for (id, scope) in [
        ("p", MemoryScope::Project),
        ("s", MemoryScope::Session),
        ("f", MemoryScope::Frontend),
    ] {
        seed_memory(
            &store,
            id,
            "2026-01-01T00:00:00Z",
            "hooks body",
            "hooks",
            MemoryImportance::Normal,
            scope,
        );
    }

    // Even the owner's own session/frontend memories are excluded by --shared.
    let hits = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter::default(),
            true,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.id, "p");
}

fn seed_memory(
    store: &SessionStore,
    id: &str,
    now: &str,
    body: &str,
    topic: &str,
    importance: MemoryImportance,
    scope: MemoryScope,
) -> MemoryRecord {
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        session_id: (scope == MemoryScope::Session).then(|| "s1".to_owned()),
        frontend: (scope == MemoryScope::Frontend).then(|| "codex".to_owned()),
        scope,
        topic: topic.to_owned(),
        title: format!("title-{topic}"),
        body: body.to_owned(),
        importance,
        source_id: Some(format!("src-{id}")),
        metadata: serde_json::json!({ "seed": id }),
    };
    super::memory::store_memory_at(&store.conn, &input, now, id).unwrap()
}

#[test]
fn store_memory_persists_record_with_defaults() {
    let (_dir, mut store) = open_store(16, 1024);
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        body: "remember to run the hook tests".to_owned(),
        ..Default::default()
    };
    let record = store.store_memory(&input).unwrap();

    assert_eq!(record.repo_root, "/repo");
    assert_eq!(record.importance, MemoryImportance::Normal);
    assert_eq!(record.scope, MemoryScope::Project);
    assert_eq!(record.decay_score, 0.0);
    assert_eq!(record.session_id, None);
    assert_eq!(record.frontend, None);
    assert_eq!(record.source_id, None);
    assert_eq!(record.metadata, serde_json::json!({}));
    assert_eq!(record.topic, "");
    assert_eq!(record.title, "");
    assert_eq!(record.body, "remember to run the hook tests");
    assert_eq!(record.id.len(), 64, "id must be a stable sha256 hex id");
    assert_eq!(record.created_at, record.updated_at);
    assert_eq!(record.updated_at, record.last_accessed_at);

    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn store_memory_preserves_exact_text_and_metadata() {
    let (_dir, mut store) = open_store(16, 1024);
    let body = "  spaced text with\nnewline and \"quotes\"  ";
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        topic: "hooks".to_owned(),
        title: "Hook notes".to_owned(),
        body: body.to_owned(),
        importance: MemoryImportance::Critical,
        scope: MemoryScope::Frontend,
        frontend: Some("codex".to_owned()),
        source_id: Some("artifact-1".to_owned()),
        metadata: serde_json::json!({ "source_kind": "hook", "count": 3 }),
        ..Default::default()
    };
    let stored = store.store_memory(&input).unwrap();

    let record = store
        .conn
        .query_row(
            "SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                    created_at, updated_at, last_accessed_at, decay_score, source_id,
                    metadata_json
             FROM memories WHERE id = ?1",
            [&stored.id],
            super::memory::row_to_memory,
        )
        .unwrap();
    assert_eq!(record.body, body, "body must be stored exactly as provided");
    assert_eq!(record.importance, MemoryImportance::Critical);
    assert_eq!(record.scope, MemoryScope::Frontend);
    assert_eq!(record.frontend.as_deref(), Some("codex"));
    assert_eq!(record.source_id.as_deref(), Some("artifact-1"));
    assert_eq!(
        record.metadata,
        serde_json::json!({ "source_kind": "hook", "count": 3 })
    );
}

#[test]
fn store_memory_rejects_invalid_inputs() {
    let (_dir, mut store) = open_store(16, 1024);

    let empty = NewMemory {
        repo_root: "/repo".to_owned(),
        body: "   ".to_owned(),
        ..Default::default()
    };
    let error = store.store_memory(&empty).unwrap_err().to_string();
    assert!(error.contains("must not be empty"), "got: {error}");

    let frontend_without_name = NewMemory {
        repo_root: "/repo".to_owned(),
        body: "x".to_owned(),
        scope: MemoryScope::Frontend,
        ..Default::default()
    };
    let error = store
        .store_memory(&frontend_without_name)
        .unwrap_err()
        .to_string();
    assert!(error.contains("frontend identifier"), "got: {error}");

    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "rejected writes must not persist rows");
}

#[test]
fn recall_memories_ranks_exact_topic_above_broad_matches() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m1",
        "2026-01-01T00:00:00Z",
        "deploy pipeline runs weekly",
        "hooks",
        MemoryImportance::Critical,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m2",
        "2026-01-02T00:00:00Z",
        "deploy notes",
        "deploy",
        MemoryImportance::Low,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m3",
        "2026-01-03T00:00:00Z",
        "unrelated body",
        "deployment",
        MemoryImportance::High,
        MemoryScope::Project,
    );

    let hits = store
        .recall_memories(
            "/repo",
            "deploy",
            &MemoryListFilter::default(),
            false,
            &MemoryViewer {
                frontend: "cli".to_owned(),
                session_id: "s1".to_owned(),
            },
            10,
        )
        .unwrap();
    let order = hits
        .iter()
        .map(|hit| hit.memory.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(order, vec!["m2", "m3", "m1"]);
    assert_eq!(
        hits.iter()
            .map(|hit| hit.relevance_score)
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "exact topic must rank above contains matches above body-only matches"
    );
}

#[test]
fn recall_memories_respects_scope_and_shared_filters() {
    let (_dir, store) = open_store(16, 1024);
    for (id, scope) in [
        ("p", MemoryScope::Project),
        ("g", MemoryScope::Global),
        ("s", MemoryScope::Session),
        ("f", MemoryScope::Frontend),
    ] {
        seed_memory(
            &store,
            id,
            "2026-01-01T00:00:00Z",
            "shared topic body",
            "hooks",
            MemoryImportance::Normal,
            scope,
        );
    }

    let shared = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter::default(),
            true,
            &cli_viewer(),
            10,
        )
        .unwrap();
    let mut shared_ids = shared
        .iter()
        .map(|hit| hit.memory.id.as_str())
        .collect::<Vec<_>>();
    shared_ids.sort();
    assert_eq!(
        shared_ids,
        vec!["g", "p"],
        "--shared must exclude session/frontend"
    );

    let frontend_only = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter {
                scope: Some(MemoryScope::Frontend),
                ..Default::default()
            },
            false,
            &MemoryViewer {
                frontend: "codex".to_owned(),
                session_id: "s1".to_owned(),
            },
            10,
        )
        .unwrap();
    assert_eq!(frontend_only.len(), 1);
    assert_eq!(frontend_only[0].memory.id, "f");

    // Visibility: the codex viewer sees everything (session s1 + codex), the
    // cli viewer does not see codex-frontend memories.
    let codex_view = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter::default(),
            false,
            &MemoryViewer {
                frontend: "codex".to_owned(),
                session_id: "s1".to_owned(),
            },
            10,
        )
        .unwrap();
    assert_eq!(codex_view.len(), 4);
    let cli_view = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    let mut cli_ids = cli_view
        .iter()
        .map(|hit| hit.memory.id.as_str())
        .collect::<Vec<_>>();
    cli_ids.sort();
    assert_eq!(
        cli_ids,
        vec!["g", "p", "s"],
        "cli viewer sees project/global/session"
    );
}

#[test]
fn recall_memories_applies_topic_and_importance_filters_and_escapes_like() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m1",
        "2026-01-01T00:00:00Z",
        "progress is 100% done",
        "hooks",
        MemoryImportance::Critical,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m2",
        "2026-01-01T00:00:00Z",
        "progress is 10000",
        "hooks",
        MemoryImportance::Low,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "m3",
        "2026-01-01T00:00:00Z",
        "deploy",
        "deploy",
        MemoryImportance::Critical,
        MemoryScope::Project,
    );

    // Literal substring match: the `%` in the query must not act as a wildcard.
    let literal = store
        .recall_memories(
            "/repo",
            "100%",
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert_eq!(literal.len(), 1);
    assert_eq!(literal[0].memory.id, "m1");

    // Topic and importance filters restrict recall.
    let filtered = store
        .recall_memories(
            "/repo",
            "deploy",
            &MemoryListFilter {
                topic: Some("HOOKS".to_owned()),
                ..Default::default()
            },
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert!(
        filtered.is_empty(),
        "topic filter is exact and case-insensitive"
    );

    let critical = store
        .recall_memories(
            "/repo",
            "deploy",
            &MemoryListFilter {
                importance: Some(MemoryImportance::Critical),
                ..Default::default()
            },
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert_eq!(critical.len(), 1);
    assert_eq!(critical[0].memory.id, "m3");
}

#[test]
fn list_memories_sorts_by_updated_at_desc_and_filters() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "old",
        "2026-01-01T00:00:00Z",
        "b",
        "hooks",
        MemoryImportance::Low,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "mid",
        "2026-01-02T00:00:00Z",
        "b",
        "hooks",
        MemoryImportance::Critical,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "new",
        "2026-01-03T00:00:00Z",
        "b",
        "other",
        MemoryImportance::Critical,
        MemoryScope::Global,
    );

    let all = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert_eq!(
        all.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["new", "mid", "old"],
        "list must sort by updated_at DESC"
    );

    let critical = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                importance: Some(MemoryImportance::Critical),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        critical.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["new", "mid"]
    );

    let hooks = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                topic: Some("HOOKS".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        hooks.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["mid", "old"],
        "topic filter must be case-insensitive exact match"
    );

    let range = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                older_than: Some("2026-01-03T00:00:00Z".to_owned()),
                newer_than: Some("2026-01-01T00:00:00Z".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        range.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["mid"]
    );
}

#[test]
fn delete_memory_requires_exact_id_and_respects_dry_run() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m1",
        "2026-01-01T00:00:00Z",
        "b",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    let dry = store.delete_memory("/repo", "m1", true).unwrap();
    assert!(dry.found);
    assert!(!dry.deleted);
    assert!(dry.dry_run);
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "dry-run must not mutate storage");

    let missing = store.delete_memory("/repo", "nope", false).unwrap();
    assert!(!missing.found);
    assert!(!missing.deleted);

    let other_repo = store.delete_memory("/elsewhere", "m1", false).unwrap();
    assert!(!other_repo.found, "delete is repo-scoped by exact id");

    let removed = store.delete_memory("/repo", "m1", false).unwrap();
    assert!(removed.found);
    assert!(removed.deleted);
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
