//! MCP `record_session_event` fallback tool.
//!
//! Hook-equivalent session capture for agents whose host does not expose native
//! LLM hooks. The handler is a thin adapter over the shared agent event service
//! (`atlas_agent_events::record_agent_event`), so event semantics, redaction,
//! storage routing, lifecycle actions, graph freshness, and review refresh are
//! identical to native `atlas hook` capture.

use anyhow::Result;
use serde_json::{Value, json};

use atlas_agent_events::{
    AgentEventRequest, AgentEventSource, record_agent_event, resolve_hook_policy,
};

use crate::output::OutputFormat;
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};
use crate::tools::shared::{inject_deprecated_input_fields, resolve_repo_scope_selection};

/// Hook-equivalent event names accepted by `record_session_event` (canonical
/// forms; native PascalCase aliases like `SessionStart` / `PostToolUse` are
/// also accepted through the shared policy table).
pub const SUPPORTED_EVENTS: &[&str] = &[
    "session-start",
    "user-prompt",
    "user-prompt-expansion",
    "pre-tool-use",
    "post-tool-use",
    "pre-compact",
    "post-compact",
    "stop",
    "session-end",
    "permission-request",
    "permission-denied",
    "tool-failure",
    "stop-failure",
    "error",
    "elicitation",
    "elicitation-result",
    "instructions-loaded",
    "notification",
    "subagent-start",
    "subagent-stop",
    "task-created",
    "task-completed",
    "config-change",
    "cwd-changed",
    "file-changed",
    "worktree-create",
    "worktree-remove",
];

/// Canonical event names the MCP fallback surface accepts. Single source of
/// truth for instructions and docs parity tests.
pub fn supported_event_names() -> &'static [&'static str] {
    SUPPORTED_EVENTS
}

/// True when `name` is a supported fallback event name or alias (kebab-case
/// canonical names or native PascalCase hook aliases).
pub fn is_supported_event_name(name: &str) -> bool {
    resolve_hook_policy(name).is_ok()
}

pub fn tool_record_session_event(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let event = args
        .and_then(|a| a.get("event"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|event| !event.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "missing required argument: event (one of {})",
                SUPPORTED_EVENTS.join(", ")
            )
        })?;
    if resolve_hook_policy(event).is_err() {
        let payload = ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            format!("unknown hook event: {event}"),
        )
        .with_tool("record_session_event")
        .with_retry_guidance("Use one of the supported hook event names or PascalCase aliases.")
        .with_details(json!({ "supported_events": SUPPORTED_EVENTS }));
        return tool_execution_error_value(output_format, &payload);
    }

    // Resolve the single repo the event is recorded against. Event capture is
    // per-repo, so a multi-repo scope is rejected up front.
    let scope = match resolve_repo_scope_selection("record_session_event", args, repo_root) {
        Ok(scope) => scope,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let mut repo_roots = scope
        .selection
        .as_ref()
        .map(|selection| {
            selection
                .registrations
                .iter()
                .map(|entry| entry.root.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![repo_root.to_owned()]);
    repo_roots.sort();
    repo_roots.dedup();
    if repo_roots.len() != 1 {
        let payload = ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            "record_session_event requires exactly one repo scope; use repo_scope={kind:'current'} or a single repo_scope={kind:'repo_id',...}",
        )
        .with_tool("record_session_event")
        .with_details(json!({ "resolved_repos": repo_roots }));
        return tool_execution_error_value(output_format, &payload);
    }
    let repo = repo_roots.into_iter().next().expect("one repo checked");

    let payload = args
        .and_then(|a| a.get("payload"))
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let frontend = args
        .and_then(|a| a.get("frontend"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|frontend| !frontend.is_empty())
        .unwrap_or("mcp")
        .to_owned();
    let session_id = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let agent_id = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let result = record_agent_event(AgentEventRequest {
        repo_root: repo.clone(),
        graph_db_path: db_path.to_owned(),
        frontend,
        event: event.to_owned(),
        session_id,
        agent_id: agent_id.clone(),
        payload,
        source: AgentEventSource::McpFallback,
    })?;

    let mut response = json!({
        "tool": "record_session_event",
        "event": result.event,
        "canonical_event": result.canonical_event,
        "frontend": result.frontend,
        "session_id": result.session_id,
        "agent_id": agent_id,
        "pending_resume": result.pending_resume,
        "stored": result.stored,
        "event_id": result.event_id,
        "source_id": result.source_id,
        "storage_kind": result.storage_kind,
        "snapshot": result.snapshot,
        "actions": result.actions,
        "warnings": result.warnings,
    });
    inject_deprecated_input_fields(&mut response, &scope.deprecated_input_fields);
    crate::session_tools::tool_result_value(&response, output_format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_adapters::derive_session_db_path;
    use atlas_session::{SessionEventType, SessionId, SessionStore};
    use atlas_store_sqlite::Store;
    use camino::Utf8Path;
    use tempfile::TempDir;

    use crate::output::OutputFormat;

    fn setup_db_path(dir: &TempDir) -> String {
        dir.path()
            .join(".atlas")
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned()
    }

    const GIT_LOCAL_ENV_VARS: &[&str] = &[
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_KEY_0",
        "GIT_CONFIG_VALUE_0",
        "GIT_DIR",
        "GIT_GRAFT_FILE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_INTERNAL_SUPER_PREFIX",
        "GIT_NAMESPACE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
        "GIT_WORK_TREE",
    ];

    fn git(dir: &std::path::Path, args: &[&str]) {
        let mut command = std::process::Command::new("git");
        command
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Atlas Test")
            .env("GIT_AUTHOR_EMAIL", "test@atlas")
            .env("GIT_COMMITTER_NAME", "Atlas Test")
            .env("GIT_COMMITTER_EMAIL", "test@atlas");
        for env_var in GIT_LOCAL_ENV_VARS {
            command.env_remove(env_var);
        }
        let status = command.status().expect("git command");
        assert!(status.success(), "git {args:?} failed in {}", dir.display());
    }

    fn tool_body(result: &Value) -> Value {
        result
            .get("structuredContent")
            .cloned()
            .or_else(|| {
                result
                    .get("content")
                    .and_then(|content| content.get(0))
                    .and_then(|item| item.get("text"))
                    .and_then(|text| text.as_str())
                    .and_then(|text| serde_json::from_str(text).ok())
            })
            .expect("tool body")
    }

    fn last_event_type(db_path: &str, repo: &str) -> SessionEventType {
        let store = SessionStore::open(&derive_session_db_path(db_path)).unwrap();
        let session_id = SessionId::derive(repo, "", "mcp");
        let events = store.list_events(&session_id).unwrap();
        events.last().unwrap().event_type.clone()
    }

    #[test]
    fn record_session_event_persists_session_start() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({ "event": "session-start" })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["tool"], "record_session_event");
        assert_eq!(body["event"], "session-start");
        assert_eq!(body["canonical_event"], "session-start");
        assert_eq!(body["frontend"], "mcp");
        assert_eq!(body["stored"], true);
        assert!(body["event_id"].is_number());
        assert_eq!(
            body["session_id"],
            SessionId::derive(&repo, "", "mcp").as_str()
        );
        assert_eq!(body["actions"]["lifecycle"]["status"], "loaded");
        assert_eq!(
            last_event_type(&db_path, &repo),
            SessionEventType::SessionStart
        );
    }

    #[test]
    fn record_session_event_persists_user_prompt() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({
                "event": "UserPromptSubmit",
                "payload": { "prompt": "review billing flow" },
            })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["canonical_event"], "user-prompt");
        assert_eq!(body["actions"]["prompt_routing"]["status"], "routed");
        assert_eq!(
            last_event_type(&db_path, &repo),
            SessionEventType::UserIntent
        );
    }

    #[test]
    fn record_session_event_persists_stop_with_snapshot() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({ "event": "stop" })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["actions"]["lifecycle"]["status"], "persisted");
        assert!(body["snapshot"].is_object());
        assert_eq!(
            last_event_type(&db_path, &repo),
            SessionEventType::CommandRun
        );
    }

    #[test]
    fn record_session_event_persists_file_changed_freshness() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({
                "event": "file-changed",
                "payload": { "changed_files": ["src/lib.rs"] },
            })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["actions"]["freshness"]["status"], "stale");
        assert_eq!(
            last_event_type(&db_path, &repo),
            SessionEventType::FileWrite
        );
    }

    #[test]
    fn record_session_event_explicit_session_id_is_honored() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({ "event": "user-prompt", "session_id": "custom-session" })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["session_id"], "custom-session");

        let session_db = derive_session_db_path(&db_path);
        let store = SessionStore::open(&session_db).unwrap();
        let events = store
            .list_events(&SessionId("custom-session".to_owned()))
            .unwrap();
        assert_eq!(
            events.last().unwrap().event_type,
            SessionEventType::UserIntent
        );
    }

    #[test]
    fn record_session_event_unknown_event_returns_structured_error() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({ "event": "mystery-event" })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        assert_eq!(result["isError"], true);
        let body = tool_body(&result);
        assert_eq!(body["code"], "invalid_input");
        assert_eq!(body["tool"], "record_session_event");
        assert!(body["details"]["supported_events"].is_array());
        assert!(body["message"].as_str().unwrap().contains("mystery-event"));
    }

    #[test]
    fn record_session_event_multi_repo_scope_rejected() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({ "event": "user-prompt", "repo_scope": { "kind": "all" } })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn record_session_event_post_tool_use_refreshes_graph_for_changed_files() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"mcp-hook-refresh\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::create_dir_all(repo.join(".atlas")).unwrap();
        git(repo, &["init", "--quiet"]);
        git(repo, &["add", "Cargo.toml", "src/lib.rs"]);

        let repo_str = repo.to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);
        Store::open(&db_path).unwrap();
        atlas_engine::build_graph(
            Utf8Path::new(&repo_str),
            &db_path,
            &atlas_engine::BuildOptions::default(),
        )
        .unwrap();

        std::fs::write(
            repo.join("src/lib.rs"),
            "pub fn alpha() {}\npub fn beta() {}\n",
        )
        .unwrap();

        let result = tool_record_session_event(
            Some(&json!({
                "event": "post-tool-use",
                "payload": {
                    "tool_name": "Write",
                    "changed_files": [repo.join("src/lib.rs").to_string_lossy().into_owned()],
                },
            })),
            &repo_str,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["canonical_event"], "post-tool-use");
        let status = body["actions"]["graph_refresh"]["status"].as_str().unwrap();
        assert!(
            matches!(status, "updated" | "skipped" | "error"),
            "unexpected graph_refresh status: {status}"
        );
        let store = Store::open(&db_path).unwrap();
        let nodes = store.nodes_by_file("src/lib.rs").unwrap();
        assert!(
            nodes
                .iter()
                .any(|node| node.qualified_name.ends_with("::fn::beta"))
        );
    }

    #[test]
    fn record_session_event_emits_mcp_frontend_in_event_payload() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({ "event": "user-prompt", "payload": { "prompt": "hello" } })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["frontend"], "mcp");

        let session_db = derive_session_db_path(&db_path);
        let store = SessionStore::open(&session_db).unwrap();
        let session_id = SessionId::derive(&repo, "", "mcp");
        let events = store.list_events(&session_id).unwrap();
        let persisted: Value = serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
        assert_eq!(persisted["frontend"], "mcp");
    }

    #[test]
    fn record_session_event_skips_graph_refresh_for_read_only_tool() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_record_session_event(
            Some(&json!({
                "event": "post-tool-use",
                "payload": { "tool_name": "read_file" },
            })),
            &repo,
            &db_path,
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(
            body["actions"]["graph_refresh"]["reason"],
            "tool_not_graph_relevant"
        );
    }
}
