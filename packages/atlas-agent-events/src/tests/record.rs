use super::*;

#[test]
fn resolve_hook_policy_resolves_native_and_alias_names() {
    for event in ["session-start", "SessionStart", "sessionStart"] {
        assert_eq!(
            resolve_hook_policy(event).unwrap().canonical_event,
            "session-start"
        );
    }
    for event in ["user-prompt", "UserPromptSubmit", "userPromptSubmitted"] {
        assert_eq!(
            resolve_hook_policy(event).unwrap().canonical_event,
            "user-prompt"
        );
    }
    for event in ["post-tool-use", "PostToolUse", "postToolUse"] {
        assert_eq!(
            resolve_hook_policy(event).unwrap().canonical_event,
            "post-tool-use"
        );
    }
    for event in ["file-changed", "FileChanged"] {
        assert_eq!(
            resolve_hook_policy(event).unwrap().canonical_event,
            "file-changed"
        );
    }
    for event in ["stop", "Stop"] {
        assert_eq!(resolve_hook_policy(event).unwrap().canonical_event, "stop");
    }
    assert!(resolve_hook_policy("mystery-event").is_err());
}

#[test]
fn record_agent_event_returns_normalized_result() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result: AgentEventResult = record_agent_event(record_request(
        &repo,
        &graph_db_path,
        "userPromptSubmitted",
        json!({ "prompt": "review billing flow" }),
    ))
    .unwrap();

    assert_eq!(result.event, "userPromptSubmitted");
    assert_eq!(result.canonical_event, "user-prompt");
    assert_eq!(result.frontend, "hook");
    assert!(result.stored);
    assert!(result.event_id.is_some());
    assert_eq!(
        result.session_id,
        SessionId::derive(&repo, "", "hook").as_str()
    );
    assert_eq!(result.actions["prompt_routing"]["status"], "routed");

    let store = SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
    let session_id = SessionId::derive(&repo, "", "hook");
    let events = store.list_events(&session_id).unwrap();
    let persisted: Value = serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(persisted["hook_event"], "user-prompt");
}

#[test]
fn mcp_fallback_records_source_and_agent_id_metadata() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result = record_agent_event(AgentEventRequest {
        repo_root: repo.clone(),
        graph_db_path: graph_db_path.clone(),
        frontend: "zed".to_owned(),
        event: "user-prompt".to_owned(),
        session_id: None,
        agent_id: Some("agent-7".to_owned()),
        payload: json!({ "prompt": "review continuity fallback" }),
        source: AgentEventSource::McpFallback,
    })
    .unwrap();

    let store = SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
    let session_id = SessionId::derive(&repo, "", "zed");
    let events = store.list_events(&session_id).unwrap();
    let persisted: Value = serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(result.frontend, "zed");
    assert_eq!(persisted["event_source"], "mcp_fallback");
    assert_eq!(persisted["agent_id"], "agent-7");
    assert_eq!(persisted["hook_metadata"]["event_source"], "mcp_fallback");
    assert_eq!(persisted["hook_metadata"]["agent_id"], "agent-7");
}

#[test]
fn record_agent_event_rejects_unknown_event() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let error = record_agent_event(record_request(
        &repo,
        &graph_db_path,
        "mystery-event",
        Value::Null,
    ))
    .unwrap_err();

    assert!(error.to_string().contains("unknown hook event"));
}

#[test]
fn session_start_hook_records_resume_when_snapshot_pending() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo, "", "hook");

    let mut store = SessionStore::open_in_repo(dir.path()).unwrap();
    store
        .upsert_session_meta(session_id.clone(), &repo, "cli", None)
        .unwrap();
    store
        .append_event(
            normalize_event(
                SessionEventType::CommandRun,
                2,
                json!({ "command": "cargo test", "status": "ok" }),
            )
            .bind(session_id.clone()),
        )
        .unwrap();
    store.build_resume(&session_id).unwrap();

    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    persist_hook_event(&repo, &graph_db_path, "hook", "session-start", Value::Null).unwrap();

    let store = SessionStore::open_in_repo(dir.path()).unwrap();
    let events = store.list_events(&session_id).unwrap();
    let last = events.last().expect("hook should append an event");
    assert_eq!(last.event_type, SessionEventType::SessionResume);
}
