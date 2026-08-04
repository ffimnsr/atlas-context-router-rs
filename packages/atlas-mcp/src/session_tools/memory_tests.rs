//! Unit tests for the global-memory and memory store/recall tools.

use super::test_util::{append_session_event, open_session_store, setup_db_path, tool_body};
use super::*;
use atlas_session::SessionEventType;
use tempfile::TempDir;

#[test]
fn get_global_memory_without_focus_returns_normalized_shape() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let store = open_session_store(&db_path);
    store
        .record_symbol_access(repo_root, "crate::compute")
        .unwrap();
    store.record_file_access(repo_root, "src/lib.rs").unwrap();
    store
        .record_workflow_pattern(
            repo_root,
            &["query_graph".to_string(), "get_context".to_string()],
        )
        .unwrap();

    let result = tool_get_global_memory(None, repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["repo_root"], repo_root);
    assert!(body["focus"].is_null());
    assert!(!body["frequent_symbols"].as_array().unwrap().is_empty());
    assert!(!body["frequent_files"].as_array().unwrap().is_empty());
    assert!(!body["workflow_patterns"].as_array().unwrap().is_empty());
    assert_eq!(body["summary"]["relevant_session_count"], 0);
}

#[test]
fn get_global_memory_with_focus_returns_relevant_sessions() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let mut store = open_session_store(&db_path);
    let session_id = SessionId::derive(repo_root, "", "cli");
    store
        .upsert_session_meta(session_id.clone(), repo_root, "cli", None)
        .unwrap();
    append_session_event(
        &mut store,
        &session_id,
        SessionEventType::ContextRequest,
        serde_json::json!({"query": "crate::focused_symbol"}),
    );
    append_session_event(
        &mut store,
        &session_id,
        SessionEventType::FileRead,
        serde_json::json!({"file": "src/focused.rs"}),
    );
    store
        .record_symbol_access(repo_root, "crate::focused_symbol")
        .unwrap();
    store
        .record_file_access(repo_root, "src/focused.rs")
        .unwrap();
    drop(store);

    let result = tool_get_global_memory(
        Some(&serde_json::json!({
            "focus_symbols": ["crate::focused_symbol"],
            "focus_files": ["src/focused.rs"],
            "limit": 5
        })),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let body = tool_body(&result);
    assert_eq!(body["focus"]["symbols"][0], "crate::focused_symbol");
    assert_eq!(body["focus"]["files"][0], "src/focused.rs");
    assert!(!body["relevant_sessions"].as_array().unwrap().is_empty());
    assert!(body["summary"]["relevant_session_count"].as_u64().unwrap() >= 1);
}

// ── ICM-A — memory_store / memory_recall ─────────────────────────────────

fn store_via_tool(repo_root: &str, db_path: &str, args: &serde_json::Value) -> Value {
    let result = tool_memory_store(Some(args), repo_root, db_path, OutputFormat::Json).unwrap();
    tool_body(&result)
}

#[test]
fn memory_store_uses_cli_defaults_and_validation() {
    let dir = TempDir::new().unwrap();
    let db_path = setup_db_path(&dir);
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let repo_root = dir.path().to_string_lossy().into_owned();

    let body = store_via_tool(
        &repo_root,
        &db_path,
        &serde_json::json!({
            "text": "remember hooks",
            "topic": "hooks"
        }),
    );
    let memory = &body["memory"];
    // Same defaults as `atlas memory store`: normal importance, project scope.
    assert_eq!(memory["importance"], "normal");
    assert_eq!(memory["scope"], "project");
    assert_eq!(memory["frontend"], serde_json::Value::Null);
    assert_eq!(memory["session_id"], serde_json::Value::Null);
    assert_eq!(memory["body"], "remember hooks");
    assert_eq!(memory["decay_score"], serde_json::json!(0.0));
    assert_eq!(body["summary"]["memory_id"], memory["id"]);

    // Validation errors match the CLI message contract exactly.
    let error = tool_memory_store(
        Some(&serde_json::json!({
            "text": "x",
            "importance": "urgent"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("unknown memory importance: urgent"),
        "got: {error}"
    );

    let error = tool_memory_store(
        Some(&serde_json::json!({
            "text": "x",
            "scope": "frontend"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("frontend identifier"), "got: {error}");
}

#[test]
fn memory_store_persists_frontend_normalization_and_source_id() {
    let dir = TempDir::new().unwrap();
    let db_path = setup_db_path(&dir);
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let repo_root = dir.path().to_string_lossy().into_owned();

    let body = store_via_tool(
        &repo_root,
        &db_path,
        &serde_json::json!({
            "text": "codex deploy note",
            "topic": "deploy",
            "importance": "critical",
            "scope": "frontend",
            "frontend": "Codex",
            "source_id": "artifact-9"
        }),
    );
    let memory = &body["memory"];
    assert_eq!(
        memory["frontend"], "codex",
        "frontend must normalize like the CLI"
    );
    assert_eq!(memory["scope"], "frontend");
    assert_eq!(memory["source_id"], "artifact-9");

    // Unknown frontends are rejected unless config allows custom ones.
    let error = tool_memory_store(
        Some(&serde_json::json!({
            "text": "x",
            "scope": "frontend",
            "frontend": "zed"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("unknown frontend: zed"), "got: {error}");
}

#[test]
fn memory_recall_applies_visibility_and_retrieval_hints() {
    let dir = TempDir::new().unwrap();
    let db_path = setup_db_path(&dir);
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let repo_root = dir.path().to_string_lossy().into_owned();

    // A project-scoped memory, an mcp-session memory, and a codex-frontend memory.
    store_via_tool(
        &repo_root,
        &db_path,
        &serde_json::json!({
            "text": "deploy pipeline runs weekly",
            "topic": "hooks",
            "importance": "critical",
            "source_id": "src-hooks"
        }),
    );
    let session_body = store_via_tool(
        &repo_root,
        &db_path,
        &serde_json::json!({
            "text": "deploy session note",
            "topic": "deploy",
            "scope": "session"
        }),
    );
    store_via_tool(
        &repo_root,
        &db_path,
        &serde_json::json!({
            "text": "deploy secrets",
            "topic": "deploy",
            "scope": "frontend",
            "frontend": "codex"
        }),
    );

    let result = tool_memory_recall(
        Some(&serde_json::json!({
            "query": "deploy"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let body = tool_body(&result);

    // The mcp viewer sees project + own-session memories, never codex's.
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        !results
            .iter()
            .any(|hit| hit["memory"]["scope"] == "frontend"),
        "codex-frontend memory must be hidden from the mcp viewer"
    );
    assert!(
        results
            .iter()
            .any(|hit| { hit["memory"]["id"] == session_body["memory"]["id"] })
    );
    assert_eq!(body["summary"]["match_count"], 2);
    assert_eq!(body["truncated"], false);

    // Compact retrieval hints expose topic + source_id for follow-up recall.
    let hints = body["retrieval_hints"].as_array().unwrap();
    assert!(
        hints
            .iter()
            .any(|hint| hint["kind"] == "topic" && hint["value"] == "deploy")
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint["kind"] == "source_id" && hint["value"] == "src-hooks")
    );

    // shared=true returns project + global only.
    let shared = tool_memory_recall(
        Some(&serde_json::json!({
            "query": "deploy",
            "shared": true
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let shared_body = tool_body(&shared);
    assert_eq!(shared_body["summary"]["match_count"], 1);

    // Invalid recall filters fail with the CLI validation contract.
    let error = tool_memory_recall(
        Some(&serde_json::json!({
            "query": "deploy",
            "scope": "org"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("unknown memory scope: org"), "got: {error}");
}

#[test]
fn memory_recall_without_session_db_returns_empty() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("atlas.db").to_string_lossy().into_owned();
    let repo_root = dir.path().to_string_lossy().into_owned();

    let result = tool_memory_recall(
        Some(&serde_json::json!({
            "query": "anything"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let body = tool_body(&result);
    assert_eq!(body["summary"]["match_count"], 0);
    assert_eq!(body["retrieval_hints"], serde_json::json!([]));
}
