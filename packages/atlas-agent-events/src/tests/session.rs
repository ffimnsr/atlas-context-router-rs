use super::*;

#[test]
fn session_start_hook_bootstraps_frontend_scoped_session_without_session_command() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo, "", "hook");
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let persisted =
        persist_hook_event(&repo, &graph_db_path, "hook", "session-start", Value::Null).unwrap();
    assert_eq!(persisted.session_id, session_id);
    assert!(!persisted.pending_resume);
    assert!(persisted.stored_event_id.is_some());

    let store = SessionStore::open_in_repo(Path::new(&repo)).unwrap();
    let sessions = store.list_sessions().unwrap();
    assert!(
        sessions
            .iter()
            .any(|session| session.session_id == session_id),
        "session-start hook should register session metadata"
    );

    let events = store.list_events(&session_id).unwrap();
    let last = events.last().expect("hook should append an event");
    assert_eq!(last.event_type, SessionEventType::SessionStart);
}

#[test]
fn pre_compact_hook_builds_resume_snapshot() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo, "", "hook");
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    persist_hook_event(&repo, &graph_db_path, "hook", "user-prompt", Value::Null).unwrap();
    let persisted =
        persist_hook_event(&repo, &graph_db_path, "hook", "pre-compact", Value::Null).unwrap();
    assert!(persisted.snapshot.is_some());

    let store = SessionStore::open_in_repo(dir.path()).unwrap();
    let snapshot = store.get_resume_snapshot(&session_id).unwrap();
    assert!(
        snapshot.is_some(),
        "pre-compact should build a resume snapshot"
    );
}

#[test]
fn build_hook_event_redacts_secret_payload_fields() {
    let session_id = SessionId::derive("/repo", "", "hook");
    let payload = redact_payload(json!({ "token": "secret", "safe": "ok" }));
    let event = build_hook_event(
        &session_id,
        HookEventParts {
            frontend: "hook",
            event: "tool-failure",
            payload,
            hook_metadata: json!({}),
            source_id: None,
            storage_kind: None,
            pending_resume: false,
            event_source: "hook",
            agent_id: None,
        },
    );
    assert_eq!(event.event_type, SessionEventType::CommandFail);
    assert_eq!(event.payload["frontend"], "hook");
    assert_eq!(event.payload["payload"]["token"], "[REDACTED]");
    assert_eq!(event.payload["payload"]["safe"], "ok");
}

#[test]
fn build_hook_event_maps_permission_denied_to_decision() {
    let session_id = SessionId::derive("/repo", "", "claude");
    let event = build_hook_event(
        &session_id,
        HookEventParts {
            frontend: "claude",
            event: "permission-denied",
            payload: json!({ "tool": "Bash" }),
            hook_metadata: json!({}),
            source_id: None,
            storage_kind: None,
            pending_resume: false,
            event_source: "hook",
            agent_id: None,
        },
    );
    assert_eq!(event.event_type, SessionEventType::Decision);
    assert_eq!(event.payload["hook_event"], "permission-denied");
}

#[test]
fn build_hook_event_maps_post_tool_use_to_graph_update() {
    let session_id = SessionId::derive("/repo", "", "copilot");
    let event = build_hook_event(
        &session_id,
        HookEventParts {
            frontend: "copilot",
            event: "post-tool-use",
            payload: json!({ "tool": "Edit" }),
            hook_metadata: json!({}),
            source_id: None,
            storage_kind: None,
            pending_resume: false,
            event_source: "hook",
            agent_id: None,
        },
    );
    assert_eq!(event.event_type, SessionEventType::GraphUpdate);
    assert_eq!(event.payload["frontend"], "copilot");
}

#[test]
fn build_hook_event_maps_aliases_through_policy_table() {
    let session_id = SessionId::derive("/repo", "", "copilot");
    let event = build_hook_event(
        &session_id,
        HookEventParts {
            frontend: "copilot",
            event: "userPromptSubmitted",
            payload: json!({ "prompt": "hi" }),
            hook_metadata: json!({}),
            source_id: None,
            storage_kind: None,
            pending_resume: false,
            event_source: "hook",
            agent_id: None,
        },
    );
    assert_eq!(event.event_type, SessionEventType::UserIntent);
    assert_eq!(event.payload["hook_event"], "user-prompt");
}

#[test]
fn persist_hook_event_rejects_unknown_hook_name() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let error = persist_hook_event(&repo, &graph_db_path, "hook", "mystery-event", Value::Null)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unknown hook event: mystery-event")
    );
}

#[test]
fn large_post_tool_use_payload_routes_to_content_store() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let payload = json!({ "output": "x".repeat(6_000) });

    let persisted =
        persist_hook_event(&repo, &graph_db_path, "claude", "post-tool-use", payload).unwrap();

    let source_id = persisted
        .source_id
        .expect("routed hook should store source id");
    assert_eq!(persisted.storage_kind, Some("pointer"));

    let mut session_store = SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
    let session_id = SessionId::derive(&repo, "", "claude");
    let events = session_store.list_events(&session_id).unwrap();
    let last_payload: Value = serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(last_payload["source_id"], source_id);
    assert_eq!(last_payload["payload_storage"]["kind"], "pointer");

    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    assert!(content_store.get_source(&source_id).unwrap().is_some());

    let snapshot = session_store.build_resume(&session_id).unwrap();
    let snapshot_value: Value = serde_json::from_str(&snapshot.snapshot).unwrap();
    assert!(
        snapshot_value["saved_artifact_refs"]
            .as_array()
            .unwrap()
            .contains(&json!(source_id))
    );
}

#[test]
fn large_session_only_hook_routes_to_content_store() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let payload = json!({ "output": "x".repeat(6_000) });

    let persisted =
        persist_hook_event(&repo, &graph_db_path, "codex", "pre-tool-use", payload).unwrap();

    let source_id = persisted
        .source_id
        .expect("oversized session-only hook should store source id");
    assert_eq!(persisted.storage_kind, Some("pointer"));

    let session_store = SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
    let session_id = SessionId::derive(&repo, "", "codex");
    let events = session_store.list_events(&session_id).unwrap();
    let last_payload: Value = serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(last_payload["source_id"], source_id);
    assert_eq!(last_payload["payload_storage"]["kind"], "pointer");

    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    assert!(content_store.get_source(&source_id).unwrap().is_some());
}

#[test]
fn session_start_hook_loads_resume_and_marks_snapshot_consumed() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo, "", "hook");
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let mut store = SessionStore::open_in_repo(dir.path()).unwrap();
    store
        .upsert_session_meta(session_id.clone(), &repo, "hook", None)
        .unwrap();
    store
        .append_event(
            normalize_event(
                SessionEventType::UserIntent,
                3,
                json!({ "prompt": "review billing flow", "files": ["src/lib.rs"] }),
            )
            .bind(session_id.clone()),
        )
        .unwrap();
    store.build_resume(&session_id).unwrap();

    let persisted =
        persist_hook_event(&repo, &graph_db_path, "hook", "session-start", Value::Null).unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("session-start").unwrap(),
        &persisted,
        &Value::Null,
    );

    assert_eq!(actions["lifecycle"]["status"], "loaded");
    assert_eq!(actions["lifecycle"]["resume_loaded"], true);

    let store = SessionStore::open_in_repo(dir.path()).unwrap();
    let snapshot = store.get_resume_snapshot(&session_id).unwrap().unwrap();
    assert!(
        snapshot.consumed,
        "restore should mark pending snapshot consumed"
    );

    let events = store.list_events(&session_id).unwrap();
    let persisted_payload: Value =
        serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(
        persisted_payload["hook_metadata"]["restore_metadata"]["pending_resume"],
        true
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["restore_metadata"]["has_resume_snapshot"],
        true
    );
}

#[test]
fn user_prompt_hook_routes_intent_and_finds_saved_context() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let identity = ArtifactIdentity::artifact_label("review-context");
    let source_id = generate_source_id(&identity, "BillingService review context");

    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();

    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    content_store
        .index_artifact(
            SourceMeta {
                id: source_id.clone(),
                session_id: None,
                agent_id: None,
                source_type: "review_context".to_owned(),
                label: "review context".to_owned(),
                repo_root: Some(repo.clone()),
                repo_roots: vec![repo.clone()],
                repo_id: None,
                repo_ids: vec![],
                identity_kind: identity.kind_str().to_owned(),
                identity_value: identity.value().to_owned(),
            },
            "BillingService review context and call graph",
            "text/plain",
        )
        .unwrap();

    let payload = json!({ "prompt": "who calls BillingService" });
    let persisted = persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("user-prompt").unwrap(),
        &persisted,
        &payload,
    );

    assert_eq!(actions["prompt_routing"]["status"], "routed");
    assert_eq!(actions["prompt_routing"]["intent"], "usage_lookup");
    assert_eq!(
        actions["prompt_routing"]["saved_context_hits"][0]["source_id"],
        source_id
    );

    let session_store = SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
    let session_id = SessionId::derive(&repo, "", "hook");
    let events = session_store.list_events(&session_id).unwrap();
    let persisted_payload: Value =
        serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(
        persisted_payload["hook_metadata"]["saved_artifact_refs"][0],
        source_id
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["source_summaries"][0]["source_id"],
        source_id
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["retrieval_hints"][0]["kind"],
        "prompt_query"
    );
}

#[test]
fn stop_hook_persists_handoff_artifact() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        json!({ "prompt": "review src/lib.rs", "files": ["src/lib.rs"] }),
    )
    .unwrap();
    let persisted = persist_hook_event(&repo, &graph_db_path, "hook", "stop", Value::Null).unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("stop").unwrap(),
        &persisted,
        &Value::Null,
    );

    let source_id = actions["lifecycle"]["resume_source_id"]
        .as_str()
        .expect("stop should persist a handoff artifact");
    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    assert!(content_store.get_source(source_id).unwrap().is_some());

    assert_eq!(actions["lifecycle"]["status"], "persisted");
    assert_eq!(actions["lifecycle"]["snapshot_event_count"], 2);
    assert!(
        actions["lifecycle"]["context_hints"]["recent_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs"))
    );
    assert!(
        actions["lifecycle"]["context_hints"]["recent_hook_events"]
            .as_array()
            .unwrap()
            .contains(&json!("user-prompt"))
    );
}

#[test]
fn session_end_hook_persists_handoff_artifact() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        json!({ "prompt": "handoff active plan", "files": ["src/lib.rs"] }),
    )
    .unwrap();
    let persisted =
        persist_hook_event(&repo, &graph_db_path, "hook", "session-end", Value::Null).unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("session-end").unwrap(),
        &persisted,
        &Value::Null,
    );

    assert_eq!(actions["lifecycle"]["status"], "persisted");
    assert_eq!(actions["lifecycle"]["snapshot_event_count"], 2);
    assert!(
        actions["lifecycle"]["context_hints"]["recent_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs"))
    );
    assert!(
        actions["lifecycle"]["context_hints"]["recent_hook_events"]
            .as_array()
            .unwrap()
            .contains(&json!("user-prompt"))
    );
}
