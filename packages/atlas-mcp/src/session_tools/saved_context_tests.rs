//! Unit tests for the saved-context search/save/stats/purge tools.

use super::test_util::{
    install_purge_request_context, large_content, medium_content, medium_secret_content,
    oversized_content, purge_request_params, save_indexed_artifact, setup_db_path,
    setup_multi_repo_registry, tool_body,
};
use super::*;
use atlas_adapters::{derive_bridge_dir, derive_content_db_path};
use atlas_contentstore::ContentStore;
use atlas_session::{NewSessionEvent, SessionEventType};
use tempfile::TempDir;

#[test]
fn test_search_saved_context_missing_query() {
    let dir = TempDir::new().unwrap();
    let db_path = setup_db_path(&dir);
    let err = tool_search_saved_context(
        Some(&serde_json::json!({})),
        dir.path().to_str().unwrap(),
        &db_path,
        OutputFormat::Json,
    );
    assert!(err.is_err());
}

#[test]
fn test_save_and_search_artifact() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let args = serde_json::json!({
        "content": "hello world",
        "label": "test artifact",
        "source_type": "test",
    });
    let result =
        tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["storage_mode"], "raw_inline");
    assert!(body["source_id"].is_null());
    assert_eq!(body["inline_content"], "hello world");
    assert_eq!(body["summary"]["inline"], true);
}

#[test]
fn save_context_artifact_routes_medium_output_to_preview() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let args = serde_json::json!({
        "content": medium_content("preview"),
        "label": "preview artifact",
    });
    let result =
        tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);

    assert_eq!(body["storage_mode"], "indexed_preview");
    assert!(body["source_id"].as_str().is_some());
    assert!(body["preview"].as_str().unwrap().contains("preview:"));
    assert!(body["inline_content"].is_null());
}

#[test]
fn save_context_artifact_routes_large_output_to_pointer() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let args = serde_json::json!({
        "content": large_content("pointer"),
        "label": "pointer artifact",
    });
    let result =
        tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);

    let source_id = body["source_id"].as_str().unwrap();
    assert_eq!(body["storage_mode"], "indexed_pointer");
    assert!(
        body["retrieval_hint"]
            .as_str()
            .unwrap()
            .contains("read_saved_context")
    );
    assert_eq!(
        body["resource_link"]["uri"],
        serde_json::json!(format!("atlas://saved-context/{source_id}"))
    );
}

#[test]
fn save_context_artifact_caps_oversized_output_chunks() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let args = serde_json::json!({
        "content": oversized_content(700),
        "label": "oversized artifact",
    });
    let result =
        tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    let source_id = body["source_id"].as_str().expect("indexed source id");

    let content_db = derive_content_db_path(&db_path);
    let store = ContentStore::open(&content_db).expect("open content store");
    let chunks = store.get_chunks(source_id).expect("get stored chunks");
    assert_eq!(body["storage_mode"], "indexed_pointer");
    assert!(!chunks.is_empty());
    assert!(chunks.len() <= 500, "default per-file chunk cap must apply");
}

#[test]
fn save_context_artifact_redacts_secret_bearing_output_with_runtime_rules() {
    let dir = TempDir::new().unwrap();
    let atlas_dir = dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    std::fs::write(
            atlas_dir.join("redaction-rules.toml"),
            "token_prefixes = [\"zz-\"]\nsecret_key_patterns = [\"sessionid\"]\ntoken_min_len = 3\nhex_secret_min_len = 32\nbase64_secret_min_len = 40\n",
        )
        .unwrap();
    std::fs::write(
        atlas_dir.join("config.toml"),
        "[sanitization]\nredaction_rules_file = \"redaction-rules.toml\"\n",
    )
    .unwrap();

    let secret_one = medium_secret_content("sessionId=abc123", "zz-123456789");
    let args_one = serde_json::json!({
        "content": secret_one,
        "label": "secret artifact one",
    });
    let saved_one =
        tool_save_context_artifact(Some(&args_one), repo_root, &db_path, OutputFormat::Json)
            .unwrap();
    let body_one: Value =
        serde_json::from_str(saved_one["content"][0]["text"].as_str().unwrap()).unwrap();
    let preview_one = body_one["preview"].as_str().unwrap();
    assert!(preview_one.contains("sessionId=[REDACTED]"));
    assert!(preview_one.contains("[REDACTED]"));
    assert!(!preview_one.contains("abc123"));
    assert!(!preview_one.contains("zz-123456789"));

    std::fs::write(
            atlas_dir.join("redaction-rules.toml"),
            "token_prefixes = [\"yy-\"]\nsecret_key_patterns = [\"sessionid\"]\ntoken_min_len = 3\nhex_secret_min_len = 32\nbase64_secret_min_len = 40\n",
        )
        .unwrap();

    let secret_two = medium_secret_content("sessionId=def456", "yy-987654321");
    let args_two = serde_json::json!({
        "content": secret_two,
        "label": "secret artifact two",
    });
    let saved_two =
        tool_save_context_artifact(Some(&args_two), repo_root, &db_path, OutputFormat::Json)
            .unwrap();
    let body_two: Value =
        serde_json::from_str(saved_two["content"][0]["text"].as_str().unwrap()).unwrap();
    let source_id = body_two["source_id"].as_str().expect("preview source id");
    assert!(body_two["preview"].as_str().unwrap().contains("[REDACTED]"));
    assert!(
        !body_two["preview"]
            .as_str()
            .unwrap()
            .contains("yy-987654321")
    );

    let read_args = serde_json::json!({"source_id": source_id});
    let read_result =
        tool_read_saved_context(Some(&read_args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let read_body: Value =
        serde_json::from_str(read_result["content"][0]["text"].as_str().unwrap()).unwrap();
    let stored = read_body["content"].as_str().unwrap();
    assert!(stored.contains("sessionId=[REDACTED]"));
    assert!(stored.contains("[REDACTED]"));
    assert!(!stored.contains("def456"));
    assert!(!stored.contains("yy-987654321"));
}

#[test]
fn search_saved_context_returns_identity_metadata() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let content = medium_content("identity");
    let _source_id = save_indexed_artifact(repo_root, &db_path, "my label", &content, None);

    let args = serde_json::json!({"query": "identity"});
    let result =
        tool_search_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    let hit = &body["matches"].as_array().unwrap()[0];
    assert!(hit["chunk_id"].as_str().is_some());
    assert_eq!(hit["identity_kind"].as_str().unwrap(), "artifact_label");
    assert_eq!(hit["identity_value"].as_str().unwrap(), "my label");
}

#[test]
fn saved_context_tools_accept_canonical_repo_scope_and_reject_legacy_fields() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let dep_repo_id = setup_multi_repo_registry(repo_root);

    let save_args = serde_json::json!({
        "content": medium_content("repo-scope"),
        "label": "repo-scope artifact",
        "repo_scope": { "kind": "repo_id", "repo_id": dep_repo_id.clone() },
    });
    let save_result =
        tool_save_context_artifact(Some(&save_args), repo_root, &db_path, OutputFormat::Json)
            .unwrap();
    assert!(
        save_result["_meta"]
            .get("deprecated_input_fields")
            .is_none()
    );

    let search_args = serde_json::json!({
        "query": "repo-scope",
        "cross_session": true,
        "all_repos": true,
    });
    let search_result =
        tool_search_saved_context(Some(&search_args), repo_root, &db_path, OutputFormat::Json)
            .unwrap();
    assert_eq!(search_result["isError"], serde_json::json!(true));
    assert_eq!(
        search_result["structuredContent"]["message"],
        serde_json::json!("legacy repo scope fields are no longer supported")
    );

    let cross_args = serde_json::json!({
        "query": "repo-scope",
        "repo_id": setup_multi_repo_registry(repo_root),
    });
    let cross_result =
        tool_cross_session_search(Some(&cross_args), repo_root, &db_path, OutputFormat::Json)
            .unwrap();
    assert_eq!(cross_result["isError"], serde_json::json!(true));
    assert_eq!(
        cross_result["structuredContent"]["message"],
        serde_json::json!("legacy repo scope fields are no longer supported")
    );
}

#[test]
fn search_decisions_returns_linked_artifacts_and_evidence() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let session_db = derive_session_db_path(&db_path);
    let mut store = SessionStore::open(&session_db).unwrap();
    let session_id = SessionId::derive(repo_root, "", "mcp");
    store
        .upsert_session_meta(session_id.clone(), repo_root, "mcp", None)
        .unwrap();
    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: serde_json::json!({
                "summary": "reuse prior review context",
                "rationale": "same file changed again",
                "conclusion": "prior review still relevant",
                "query": "src/lib.rs",
                "source_id": "src-123",
                "evidence": [{"kind": "saved_context", "source_id": "src-123"}],
            }),
            created_at: None,
        })
        .unwrap();

    let result = tool_search_decisions(
        Some(&serde_json::json!({
            "query": "src/lib.rs",
            "session_id": session_id.as_str()
        })),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let body = tool_body(&result);
    assert_eq!(body["query"]["text"], "src/lib.rs");
    assert_eq!(body["query"]["session_id"], session_id.as_str());
    assert_eq!(body["summary"]["match_count"], 1);
    let hit = &body["matches"][0];
    assert_eq!(hit["decision"]["summary"], "reuse prior review context");
    assert_eq!(hit["decision"]["source_ids"][0], "src-123");
    assert_eq!(hit["decision"]["evidence"][0]["kind"], "saved_context");
    assert_eq!(hit["decision"]["evidence"][0]["source_id"], "src-123");
}

#[test]
fn linked_decision_lookup_falls_back_to_repo_scope_when_session_misses() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let session_db = derive_session_db_path(&db_path);
    let mut store = SessionStore::open(&session_db).unwrap();
    let mcp_session = SessionId::derive(repo_root, "", "mcp");
    let other_session = SessionId::derive(repo_root, "", "cli");

    store
        .upsert_session_meta(mcp_session.clone(), repo_root, "mcp", None)
        .unwrap();
    store
        .upsert_session_meta(other_session.clone(), repo_root, "cli", None)
        .unwrap();
    store
        .append_event(NewSessionEvent {
            session_id: other_session,
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: serde_json::json!({
                "summary": "reuse repo-wide decision",
                "conclusion": "fallback matched repo memory",
                "query": "verify_token",
                "source_id": "artifact-42",
                "evidence": [{"kind": "saved_context", "source_id": "artifact-42"}],
            }),
            created_at: None,
        })
        .unwrap();

    let hits = search_decisions_best_effort(
        repo_root,
        &db_path,
        Some(mcp_session.as_str()),
        "verify_token",
        5,
    );

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].decision.summary, "reuse repo-wide decision");
    assert_eq!(hits[0].decision.source_ids, vec!["artifact-42"]);
}

#[test]
fn search_saved_context_limit_is_clamped_by_central_budget_policy() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    for i in 0..30 {
        let content = medium_content(&format!("budget-{i}"));
        let _ = save_indexed_artifact(
            repo_root,
            &db_path,
            &format!("budget artifact {i}"),
            &content,
            None,
        );
    }

    let args = serde_json::json!({"query": "budget", "limit": 9999});
    let result =
        tool_search_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);

    assert_eq!(result["budget_status"], "partial_result");
    assert_eq!(result["budget_hit"], true);
    assert_eq!(
        result["budget_name"],
        "content_saved_context_lookup.max_sources"
    );
    assert_eq!(result["budget_limit"], 25);
    assert_eq!(result["budget_observed"], 30);
    assert_eq!(body["summary"]["match_count"], 25);
    assert_eq!(body["matches"].as_array().unwrap().len(), 25);
}

#[test]
fn test_get_context_stats_empty() {
    let dir = TempDir::new().unwrap();
    let db_path = setup_db_path(&dir);
    let result = tool_get_context_stats(
        None,
        dir.path().to_str().unwrap(),
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let content = result["content"][0]["text"].as_str().unwrap();
    let body: Value = serde_json::from_str(content).unwrap();
    assert_eq!(body["source_count"].as_u64().unwrap(), 0);
    assert_eq!(body["chunk_count"].as_u64().unwrap(), 0);
    assert_eq!(body["event_count"].as_u64().unwrap(), 0);
    // Bridge dir does not exist yet → count must be 0.
    assert_eq!(body["bridge_file_count"].as_u64().unwrap(), 0);
    assert!(body["bridge_dir_path"].is_string());
}

#[test]
fn test_purge_saved_context_age_based() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    // Nothing to purge — should return 0 deleted.
    let args = serde_json::json!({"keep_days": 30});
    let result =
        tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["mode"], "age_based");
    assert_eq!(body["deleted_sources"], 0);
    assert_eq!(body["deleted_bridge_files"], 0);
}

#[test]
fn test_purge_saved_context_requires_confirmation_when_mcp_context_is_active() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let args = serde_json::json!({"keep_days": 30});

    install_purge_request_context(purge_request_params(&args));
    let result =
        tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    crate::runtime_context::uninstall();

    let body = tool_body(&result);
    assert_eq!(result["resultType"], serde_json::json!("input_required"));
    assert_eq!(body["resultType"], serde_json::json!("input_required"));
    assert!(
        body["inputRequests"]["confirmation"].is_object(),
        "confirmation request must be present by official request id key"
    );
    assert!(body["requestState"].as_str().is_some());
}

#[test]
fn test_purge_saved_context_accept_retry_executes_purge() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let args = serde_json::json!({"keep_days": 30});

    install_purge_request_context(purge_request_params(&args));
    let first =
        tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    crate::runtime_context::uninstall();
    let request_state = tool_body(&first)["requestState"]
        .as_str()
        .unwrap()
        .to_owned();

    install_purge_request_context(serde_json::json!({
        "name": "purge_saved_context",
        "arguments": args,
        "requestState": request_state,
        "inputResponses": {
            "confirmation": {
                "action": "accept",
                "content": {
                    "confirmation": "confirm"
                }
            }
        }
    }));
    let result = tool_purge_saved_context(
        Some(&serde_json::json!({"keep_days": 30})),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    crate::runtime_context::uninstall();

    let body = tool_body(&result);
    assert_eq!(body["mode"], serde_json::json!("age_based"));
    assert_eq!(body["summary"]["status"], serde_json::json!("ok"));
}

#[test]
fn test_purge_saved_context_cancel_retry_returns_cancelled_error() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let args = serde_json::json!({"keep_days": 30});

    install_purge_request_context(purge_request_params(&args));
    let first =
        tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    crate::runtime_context::uninstall();
    let request_state = tool_body(&first)["requestState"]
        .as_str()
        .unwrap()
        .to_owned();

    install_purge_request_context(serde_json::json!({
        "name": "purge_saved_context",
        "arguments": args,
        "requestState": request_state,
        "inputResponses": {
            "confirmation": {
                "action": "cancel"
            }
        }
    }));
    let error = tool_purge_saved_context(
        Some(&serde_json::json!({"keep_days": 30})),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err();
    crate::runtime_context::uninstall();

    assert!(error.to_string().contains("cancelled by client"));
}

#[test]
fn test_purge_saved_context_rejects_tampered_request_state() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let args = serde_json::json!({"keep_days": 30});

    install_purge_request_context(purge_request_params(&args));
    let first =
        tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    crate::runtime_context::uninstall();
    let first_body = tool_body(&first);
    let request_state = first_body["requestState"].as_str().unwrap();
    let mut tampered_chars = request_state.chars().collect::<Vec<_>>();
    let last = tampered_chars
        .last_mut()
        .expect("sealed requestState must not be empty");
    *last = if *last == 'A' { 'B' } else { 'A' };
    let tampered_state = tampered_chars.into_iter().collect::<String>();

    install_purge_request_context(serde_json::json!({
        "name": "purge_saved_context",
        "arguments": args,
        "requestState": tampered_state,
        "inputResponses": {
            "confirmation": {
                "action": "accept",
                "content": {
                    "confirmation": "confirm"
                }
            }
        }
    }));
    let error = tool_purge_saved_context(
        Some(&serde_json::json!({"keep_days": 30})),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err();
    crate::runtime_context::uninstall();

    assert!(
        error
            .to_string()
            .contains("requestState signature mismatch")
    );
}

#[test]
fn test_purge_saved_context_purges_bridge_files() {
    use atlas_adapters::bridge::{BridgeEvent, write_bridge_file};
    use atlas_session::SessionId;

    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let bridge_dir = derive_bridge_dir(&db_path);
    let sid = SessionId::derive(repo_root, "", "mcp");
    let ev = BridgeEvent {
        event_type: "COMMAND_RUN".to_string(),
        priority: 0,
        payload_json: r#"{"command":"atlas build"}"#.to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
    };
    write_bridge_file(&bridge_dir, &sid, "mcp", std::slice::from_ref(&ev)).unwrap();
    write_bridge_file(&bridge_dir, &sid, "mcp", &[ev]).unwrap();

    let args = serde_json::json!({"purge_bridge_files": true, "keep_days": 30});
    let result =
        tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["mode"], "age_based");
    assert_eq!(body["deleted_bridge_files"], 2);
}

#[test]
fn purge_saved_context_session_target_returns_normalized_shape() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let session_id = "session-purge";
    let content = medium_content("session purge target");
    let source_id =
        save_indexed_artifact(repo_root, &db_path, "purge me", &content, Some(session_id));
    assert!(!source_id.is_empty());

    let result = tool_purge_saved_context(
        Some(&serde_json::json!({"session_id": session_id})),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let body = tool_body(&result);
    assert_eq!(body["mode"], "session");
    assert_eq!(body["session_id"], session_id);
    assert!(body["deleted_sources"].as_u64().unwrap() >= 1);
    assert!(body["deleted_chunks"].as_u64().unwrap() >= 1);
}
