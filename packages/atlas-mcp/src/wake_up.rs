//! MCP `wake_up` tool — bounded session-start recall for hookless agents.
//!
//! Mirror of the session-start portion of native hook capture: assembles a
//! compact, bounded context pack from the resume snapshot, decision memory,
//! memories, feedback records, saved-context hints, changed files, and graph
//! readiness via the shared [`atlas_agent_events::wake_pack`] builder, then
//! records the wake-up through the shared agent event service so native hooks
//! and the MCP fallback share one session-start pipeline.
//!
//! Contract guarantees:
//! - every list is bounded by `max_items` (hard-clamped) through the central
//!   wake-up budget policy (config-backed `[memory.wake_up]`)
//! - large saved artifacts are referenced by `source_id` only, never inlined
//! - the recorded `session-start` event keeps LoadRestore parity with native
//!   hooks (pending resume snapshots are consumed on wake-up)

use anyhow::Result;
use serde_json::{Value, json};

use atlas_agent_events::{
    AgentEventRequest, AgentEventSource, WakeBudget, WakePackOptions, build_wake_pack,
    record_agent_event,
};
use atlas_engine::Config;

use crate::output::OutputFormat;
use crate::session_tools::tool_result_value;
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};
use crate::tools::shared::{inject_deprecated_input_fields, resolve_repo_scope_selection};

/// Derive the session id for wake-up: explicit `session_id` wins, otherwise the
/// stable MCP session for the repo + frontend.
fn wake_session_id(repo_root: &str, frontend: &str, args: Option<&Value>) -> Option<String> {
    args.and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .filter(|sid| !sid.trim().is_empty())
        .map(|sid| sid.trim().to_owned())
        .or_else(|| {
            let derived = atlas_session::SessionId::derive(repo_root, "", frontend);
            Some(derived.as_str().to_owned())
        })
}

/// Assemble the bounded session-start context pack and record it through the
/// shared agent event service.
pub fn tool_wake_up(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let scope = match resolve_repo_scope_selection("wake_up", args, repo_root) {
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
            "wake_up requires exactly one repo scope; use repo_scope={kind:'current'} or a single repo_scope={kind:'repo_id',...}",
        )
        .with_tool("wake_up")
        .with_details(json!({ "resolved_repos": repo_roots }));
        return tool_execution_error_value(output_format, &payload);
    }
    let repo = repo_roots.into_iter().next().expect("one repo checked");

    let topic = args
        .and_then(|a| a.get("topic"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|topic| !topic.is_empty())
        .map(str::to_owned);
    let frontend = args
        .and_then(|a| a.get("frontend"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|frontend| !frontend.is_empty())
        .unwrap_or("mcp")
        .to_owned();
    let agent_id = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .filter(|agent_id| !agent_id.trim().is_empty())
        .map(str::to_owned);
    let session_id = wake_session_id(&repo, &frontend, args);
    let budget = WakeBudget::from_config(
        &Config::load(&atlas_engine::paths::atlas_dir(&repo)).unwrap_or_default(),
    );
    let budget = match args
        .and_then(|a| a.get("max_items"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
    {
        Some(max_items) => budget.with_max_items(max_items),
        None => budget,
    };

    let build = build_wake_pack(&WakePackOptions {
        repo_root: &repo,
        graph_db_path: db_path,
        frontend: &frontend,
        session_id: session_id.clone(),
        agent_id: agent_id.as_deref(),
        topic: topic.as_deref(),
        budget,
    });
    let pack = build.pack.to_json();
    let mut warnings = build.warnings;

    // ── record wake-up through the shared event service ─────────────────────
    let wake_status = if build.session_status == "unavailable" {
        "degraded"
    } else {
        "ok"
    };
    let event_recorded = match record_agent_event(AgentEventRequest {
        repo_root: repo.clone(),
        graph_db_path: db_path.to_owned(),
        frontend: frontend.clone(),
        event: "session-start".to_owned(),
        session_id: Some(pack["session_id"].as_str().unwrap_or_default().to_owned()),
        agent_id: agent_id.clone(),
        payload: json!({
            "tool": "wake_up",
            "topic": topic,
            "wake_up": { "status": wake_status, "max_items": budget.max_items },
        }),
        source: AgentEventSource::McpFallback,
    }) {
        Ok(result) => {
            let lifecycle_status = result
                .actions
                .pointer("/lifecycle/status")
                .and_then(|v| v.as_str())
                .unwrap_or("none")
                .to_owned();
            let resume_loaded = result
                .actions
                .pointer("/lifecycle/resume_loaded")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            json!({
                "event": result.canonical_event,
                "stored": result.stored,
                "event_id": result.event_id,
                "pending_resume": result.pending_resume,
                "lifecycle_status": lifecycle_status,
                "resume_loaded": resume_loaded,
            })
        }
        Err(e) => {
            warnings.push(format!("wake-up session event recording failed: {e}"));
            json!({ "stored": false, "error": e.to_string() })
        }
    };

    let result = json!({
        "tool": "wake_up",
        "repo_root": repo,
        "session_id": pack["session_id"],
        "frontend": frontend,
        "agent_id": agent_id,
        "current_focus": pack["current_focus"],
        "recent_decisions": pack["recent_decisions"],
        "critical_memories": pack["critical_memories"],
        "recent_feedback": pack["recent_feedback"],
        "active_memoir_concepts": pack["active_memoir_concepts"],
        "changed_files": pack["changed_files"],
        "graph_readiness": pack["graph_readiness"],
        "retrieval_hints": pack["retrieval_hints"],
        "generated_at": pack["generated_at"],
        "event_recorded": event_recorded,
        "summary": {
            "status": build.session_status,
            "pending_resume": build.pending_resume,
            "event_count": build.event_count,
            "decision_count": pack["recent_decisions"]
                .as_array()
                .map(|items| items.len())
                .unwrap_or(0),
            "critical_memory_count": pack["critical_memories"]
                .as_array()
                .map(|items| items.len())
                .unwrap_or(0),
            "feedback_count": pack["recent_feedback"]
                .as_array()
                .map(|items| items.len())
                .unwrap_or(0),
            "concept_count": pack["active_memoir_concepts"]
                .as_array()
                .map(|items| items.len())
                .unwrap_or(0),
            "changed_file_count": pack["changed_files"]
                .as_array()
                .map(|items| items.len())
                .unwrap_or(0),
            "retrieval_hint_count": pack["retrieval_hints"]
                .as_array()
                .map(|items| items.len())
                .unwrap_or(0),
            "recorded": wake_status,
        },
        "warnings": warnings,
    });

    let mut response = tool_result_value(&result, output_format)?;
    inject_deprecated_input_fields(&mut response, &scope.deprecated_input_fields);
    Ok(response)
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
    use crate::session_events::tool_record_session_event;
    use crate::session_tools::{record_mcp_decision_best_effort, tool_save_context_artifact};

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

    /// Build content above the 512 B raw threshold so `route_output` indexes
    /// the artifact and returns a `source_id`.
    fn medium_content(label: &str) -> String {
        let payload = std::iter::repeat_n("safe medium artifact payload", 40)
            .collect::<Vec<_>>()
            .join(" ");
        format!("{label}: {payload}")
    }

    fn large_content(label: &str) -> String {
        let payload = std::iter::repeat_n("safe large artifact payload with spacing", 180)
            .collect::<Vec<_>>()
            .join(" ");
        format!("{label}: {payload}")
    }

    fn save_artifact(
        repo: &str,
        db_path: &str,
        label: &str,
        content: &str,
        source_type: &str,
    ) -> String {
        // ContentStore::open does not create parent directories; the storage
        // dir normally exists by the time tools run in a real server.
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let result = tool_save_context_artifact(
            Some(&json!({
                "content": content,
                "label": label,
                "source_type": source_type,
                "content_type": "text/plain",
            })),
            repo,
            db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        body["source_id"].as_str().unwrap_or("").to_string()
    }

    /// Seed a real feedback record (ICM-C) so wake-up surfaces it in
    /// `recent_feedback`.
    fn seed_feedback(repo: &str, db_path: &str, predicted: &str, actual: &str) -> String {
        let session_db = derive_session_db_path(db_path);
        let mut store = SessionStore::open(&session_db).unwrap();
        let record = store
            .store_feedback(&atlas_session::NewFeedback {
                repo_root: repo.to_owned(),
                session_id: None,
                tool_name: "cli".to_owned(),
                analysis_kind: "dead_code".to_owned(),
                predicted: predicted.to_owned(),
                actual: actual.to_owned(),
                correction: "".to_owned(),
                related_symbol: None,
                related_file: None,
                source_id: None,
                metadata: json!({}),
            })
            .unwrap();
        record.id
    }

    #[test]
    fn wake_up_empty_repo_memory_returns_normalized_shape() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_wake_up(Some(&json!({})), &repo, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(body["tool"], "wake_up");
        assert_eq!(body["repo_root"], repo);
        assert_eq!(body["frontend"], "mcp");
        assert_eq!(
            body["session_id"],
            SessionId::derive(&repo, "", "mcp").as_str()
        );
        assert_eq!(body["current_focus"]["intent"], Value::Null);
        assert!(
            body["current_focus"]["reasoning"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(body["recent_decisions"].as_array().unwrap().is_empty());
        assert!(body["critical_memories"].as_array().unwrap().is_empty());
        assert!(body["recent_feedback"].as_array().unwrap().is_empty());
        assert!(
            body["active_memoir_concepts"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(body["changed_files"].as_array().unwrap().is_empty());
        assert!(body["retrieval_hints"].as_array().unwrap().is_empty());
        assert_eq!(body["graph_readiness"]["graph_built"], false);
        assert_eq!(body["graph_readiness"]["execution_state"], "missing");
        assert_eq!(body["summary"]["status"], "no_session");
        assert_eq!(body["summary"]["pending_resume"], false);
        assert_eq!(body["event_recorded"]["stored"], true);
        assert_eq!(body["event_recorded"]["lifecycle_status"], "loaded");
        assert!(body["generated_at"].as_str().is_some());
        assert!(
            body["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w.as_str().unwrap().contains("graph has not been built")),
            "expected graph-not-built warning, got {:?}",
            body["warnings"]
        );
        assert_eq!(
            last_event_type(&db_path, &repo),
            SessionEventType::SessionStart
        );
    }

    #[test]
    fn wake_up_normal_memory_returns_bounded_context() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        tool_record_session_event(
            Some(&json!({
                "event": "user-prompt",
                "payload": { "prompt": "refactor billing flow" },
            })),
            &repo,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        record_mcp_decision_best_effort(
            &repo,
            &db_path,
            "use cached auth token",
            Some("token cached after first fetch"),
            json!({}),
        );
        let pref_id = save_artifact(
            &repo,
            &db_path,
            "user-preference-note",
            &medium_content("preference"),
            "preference",
        );
        let design_id = save_artifact(
            &repo,
            &db_path,
            "design-note",
            &medium_content("design"),
            "decision",
        );
        let feedback_id = seed_feedback(&repo, &db_path, "dead code", "still referenced");
        assert!(!pref_id.is_empty() && !design_id.is_empty());
        assert!(!feedback_id.is_empty());
        // Stop builds a resume snapshot; wake-up then loads and consumes it.
        tool_record_session_event(
            Some(&json!({ "event": "stop" })),
            &repo,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();

        let result = tool_wake_up(Some(&json!({})), &repo, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(body["summary"]["status"], "active");
        assert_eq!(body["summary"]["pending_resume"], true);
        assert_eq!(body["current_focus"]["intent"], "refactor billing flow");
        assert!(
            body["recent_decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["summary"] == "use cached auth token")
        );
        let memories = body["critical_memories"].as_array().unwrap();
        assert!(
            memories
                .iter()
                .any(|m| m["source_id"] == pref_id && m["source_type"] == "preference")
        );
        assert!(memories.iter().any(|m| m["source_id"] == design_id));
        // recent_feedback surfaces real feedback records (ICM-C), not
        // preference artifacts.
        let feedback = body["recent_feedback"].as_array().unwrap();
        assert!(
            feedback
                .iter()
                .any(|f| f["record_id"] == feedback_id && f["predicted"] == "dead code"),
            "recent_feedback must carry real feedback records, got {:?}",
            feedback
        );
        assert!(
            !feedback.iter().any(|f| f["source_id"] == pref_id),
            "preference artifacts must not appear as feedback"
        );
        assert_eq!(body["event_recorded"]["resume_loaded"], true);
        assert_eq!(body["event_recorded"]["lifecycle_status"], "loaded");
        assert_eq!(body["summary"]["recorded"], "ok");
        // Pending snapshot existed, so the wake-up event is a SessionResume.
        assert_eq!(
            last_event_type(&db_path, &repo),
            SessionEventType::SessionResume
        );

        // Second wake-up: snapshot was consumed, no pending resume.
        let result = tool_wake_up(Some(&json!({})), &repo, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(body["summary"]["pending_resume"], false);
        assert_eq!(body["event_recorded"]["resume_loaded"], false);
    }

    #[test]
    fn wake_up_stale_graph_reports_readiness() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"wakeup-stale\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::create_dir_all(repo.join(".atlas")).unwrap();
        git(repo, &["init", "--quiet"]);
        git(repo, &["add", "Cargo.toml", "src/lib.rs"]);
        git(repo, &["commit", "--quiet", "-m", "initial"]);

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

        let result =
            tool_wake_up(Some(&json!({})), &repo_str, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(body["graph_readiness"]["graph_built"], true);
        assert_eq!(body["graph_readiness"]["stale_index"], true);
        assert_eq!(body["graph_readiness"]["execution_state"], "stale");
        assert!(
            body["graph_readiness"]["pending_graph_change_count"]
                .as_i64()
                .unwrap()
                >= 1
        );
        assert!(
            body["graph_readiness"]["pending_graph_changes"]
                .as_array()
                .is_some()
        );
        assert!(
            body["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w.as_str().unwrap().contains("stale"))
        );
    }

    #[test]
    fn wake_up_oversized_saved_artifacts_reference_by_source_id_only() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let body_text = large_content("oversized-handoff");
        let source_id = save_artifact(&repo, &db_path, "oversized-handoff", &body_text, "handoff");
        assert!(!source_id.is_empty(), "large artifact must be indexed");

        let result = tool_wake_up(Some(&json!({})), &repo, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        let memories = body["critical_memories"].as_array().unwrap();
        let entry = memories
            .iter()
            .find(|m| m["source_id"] == source_id)
            .expect("oversized artifact referenced by source_id");
        assert!(
            entry.get("content").is_none() && entry.get("body").is_none(),
            "wake_up must never inline artifact bodies: {entry}"
        );
        assert!(entry["chunk_count"].as_i64().unwrap() >= 1);
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(
            !serialized.contains("safe large artifact payload"),
            "oversized artifact body must not appear anywhere in wake-up output"
        );
    }

    #[test]
    fn wake_up_explicit_session_id_and_frontend_honored() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_wake_up(
            Some(&json!({ "session_id": "custom-session", "frontend": "zed" })),
            &repo,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["session_id"], "custom-session");
        assert_eq!(body["frontend"], "zed");

        let session_db = derive_session_db_path(&db_path);
        let store = SessionStore::open(&session_db).unwrap();
        let events = store
            .list_events(&SessionId("custom-session".to_owned()))
            .unwrap();
        assert_eq!(
            events.last().unwrap().event_type,
            SessionEventType::SessionStart
        );
    }

    #[test]
    fn wake_up_multi_repo_scope_rejected() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_wake_up(
            Some(&json!({ "repo_scope": { "kind": "all" } })),
            &repo,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(result["isError"], true);
        let body = tool_body(&result);
        assert_eq!(body["code"], "invalid_input");
        assert_eq!(body["tool"], "wake_up");
    }

    #[test]
    fn wake_up_topic_searches_decision_memory_across_sessions() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        // Seed a decision under a different session id so it is not visible in
        // the current session's resume snapshot; only topic search finds it.
        let session_db = derive_session_db_path(&db_path);
        let mut store = SessionStore::open(&session_db).unwrap();
        let other = SessionId("other-session".to_owned());
        store
            .upsert_session_meta(other.clone(), &repo, "mcp", None)
            .unwrap();
        store
            .append_event(
                atlas_adapters::extract_decision_event_with_details(
                    "use a connection pool for the billing gateway",
                    Some("pool reduces latency and connection churn"),
                    json!({}),
                )
                .bind(other),
            )
            .unwrap();
        drop(store);

        let result = tool_wake_up(
            Some(&json!({ "topic": "connection pool" })),
            &repo,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let body = tool_body(&result);
        assert!(
            body["recent_decisions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["summary"] == "use a connection pool for the billing gateway"),
            "topic must surface cross-session decision memory, got {:?}",
            body["recent_decisions"]
        );
    }

    #[test]
    fn wake_up_max_items_clamps_critical_memories() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        for i in 0..6 {
            let label = format!("clamp-artifact-{i}");
            let source_id = save_artifact(
                &repo,
                &db_path,
                &label,
                &medium_content(&label),
                "mcp_artifact",
            );
            assert!(!source_id.is_empty());
        }

        let result = tool_wake_up(
            Some(&json!({ "max_items": 3 })),
            &repo,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["critical_memories"].as_array().unwrap().len(), 3);
        assert_eq!(body["summary"]["critical_memory_count"], 3);
    }

    #[test]
    fn wake_up_topic_prioritizes_memories_and_feedback() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let session_db = derive_session_db_path(&db_path);
        let mut store = SessionStore::open(&session_db).unwrap();
        // Critical memory on the topic.
        store
            .store_memory(&atlas_session::NewMemory {
                repo_root: repo.clone(),
                session_id: None,
                frontend: Some("cli".to_owned()),
                scope: atlas_session::MemoryScope::Project,
                topic: "hooks".to_owned(),
                title: "hook lifecycle".to_owned(),
                body: "session-start wakes the agent with a bounded pack".to_owned(),
                importance: atlas_session::MemoryImportance::Critical,
                source_id: None,
                metadata: json!({}),
            })
            .unwrap();
        // Feedback on the topic.
        let feedback_id = store
            .store_feedback(&atlas_session::NewFeedback {
                repo_root: repo.clone(),
                session_id: None,
                tool_name: "cli".to_owned(),
                analysis_kind: "dead_code".to_owned(),
                predicted: "hooks are dead".to_owned(),
                actual: "hooks still fire".to_owned(),
                correction: "session-start hooks are active".to_owned(),
                related_symbol: None,
                related_file: None,
                source_id: None,
                metadata: json!({}),
            })
            .unwrap()
            .id;
        drop(store);

        let result = tool_wake_up(
            Some(&json!({ "topic": "hooks" })),
            &repo,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let body = tool_body(&result);
        let memories = body["critical_memories"].as_array().unwrap();
        assert!(
            memories
                .iter()
                .any(|m| m["kind"] == "memory" && m["topic"] == "hooks"),
            "topic must surface topic-relevant memories, got {:?}",
            memories
        );
        let feedback = body["recent_feedback"].as_array().unwrap();
        assert!(
            feedback.iter().any(|f| f["record_id"] == feedback_id),
            "topic must surface topic-relevant feedback, got {:?}",
            feedback
        );
    }
}
