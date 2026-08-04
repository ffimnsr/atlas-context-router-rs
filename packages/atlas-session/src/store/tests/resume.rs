use super::*;

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
