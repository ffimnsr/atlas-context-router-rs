//! MCP `wake_up` tool — bounded session-start recall for hookless agents.
//!
//! Mirror of the session-start portion of native hook capture: assembles a
//! compact, bounded context pack from the resume snapshot, decision memory,
//! saved-context hints, global memory, changed files, and graph readiness, then
//! records the wake-up through the shared agent event service so native hooks
//! and the MCP fallback share one session-start pipeline.
//!
//! Contract guarantees:
//! - every list is bounded by `max_items` (hard-clamped)
//! - large saved artifacts are referenced by `source_id` only, never inlined
//! - the recorded `session-start` event keeps LoadRestore parity with native
//!   hooks (pending resume snapshots are consumed on wake-up)

use anyhow::Result;
use serde_json::{Value, json};

use atlas_adapters::derive_session_db_path;
use atlas_agent_events::payload::extract_prompt_text;
use atlas_agent_events::{AgentEventRequest, AgentEventSource, record_agent_event};
use atlas_contentstore::{ContentStore, SearchFilters};
use atlas_session::{DecisionSearchHit, SessionEventType, SessionId, SessionStore};

use crate::output::OutputFormat;
use crate::session_tools::tool_result_value;
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};
use crate::tools::shared::{
    derive_graph_readiness, derive_graph_readiness_open_failed, inject_deprecated_input_fields,
    open_store, resolve_repo_scope_selection,
};

/// Default cap for every list in the wake-up pack.
const DEFAULT_MAX_ITEMS: usize = 10;
/// Hard ceiling for `max_items`; protects the response from unbounded growth.
const HARD_MAX_ITEMS: usize = 25;
/// `pending_graph_changes` is capped separately from `max_items` so the
/// readiness block stays small even when many files changed.
const MAX_PENDING_CHANGES: usize = 20;
/// Feedback (user-preference artifacts) is intentionally the smallest list.
const MAX_FEEDBACK_ITEMS: usize = 3;

/// Derive the session id for wake-up: explicit `session_id` wins, otherwise the
/// stable MCP session for the repo + frontend.
fn wake_session_id(repo_root: &str, frontend: &str, args: Option<&Value>) -> SessionId {
    if let Some(sid) = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .filter(|sid| !sid.trim().is_empty())
    {
        SessionId(sid.trim().to_owned())
    } else {
        SessionId::derive(repo_root, "", frontend)
    }
}

/// Normalize a resume-snapshot `recent_decisions` entry (or a topic-matched
/// decision hit) into one compact decision shape.
fn normalize_decision(
    summary: Option<&Value>,
    rationale: Option<&Value>,
    at: Option<&Value>,
    decision_id: Option<&str>,
    source_ids: &[String],
) -> Value {
    json!({
        "summary": summary.cloned().unwrap_or(Value::Null),
        "rationale": rationale.cloned().unwrap_or(Value::Null),
        "at": at.cloned().unwrap_or(Value::Null),
        "decision_id": decision_id.map(str::to_owned),
        "source_ids": source_ids,
    })
}

fn normalize_decision_hits(hits: &[DecisionSearchHit]) -> Vec<Value> {
    hits.iter()
        .map(|hit| {
            normalize_decision(
                Some(&Value::String(hit.decision.summary.clone())),
                hit.decision
                    .rationale
                    .as_deref()
                    .map(|r| Value::String(r.to_owned()))
                    .as_ref(),
                None,
                Some(&hit.decision.decision_id),
                &hit.decision.source_ids,
            )
        })
        .collect()
}

/// Merge snapshot decisions with topic-matched decision-memory hits,
/// deduplicated by summary and bounded by `max_items`.
fn merge_decisions(
    snapshot_entries: &[Value],
    topic_hits: Vec<Value>,
    max_items: usize,
) -> Vec<Value> {
    let mut merged: Vec<Value> = snapshot_entries.to_vec();
    for hit in topic_hits {
        let summary = hit.get("summary").and_then(|v| v.as_str());
        let already_present = summary.is_some_and(|s| {
            merged
                .iter()
                .any(|entry| entry.get("summary").and_then(|v| v.as_str()) == Some(s))
        });
        if !already_present {
            merged.push(hit);
        }
        if merged.len() >= max_items {
            break;
        }
    }
    merged.truncate(max_items);
    merged
}

/// Collect distinct concept strings (symbols, rules, workflows) for
/// `active_memoir_concepts`. There is no dedicated memoir store yet; this is
/// the closest bounded proxy until the ICM-D memoir surface ships.
fn collect_concepts(
    snapshot_view: Option<&Value>,
    store: Option<&SessionStore>,
    repo_root: &str,
    max_items: usize,
) -> Vec<String> {
    let mut concepts: Vec<String> = Vec::new();
    let push_unique = |concepts: &mut Vec<String>, value: String| {
        if !value.trim().is_empty() && !concepts.contains(&value) {
            concepts.push(value);
        }
    };

    if let Some(view) = snapshot_view {
        if let Some(symbols) = view.get("impacted_symbols").and_then(|v| v.as_array()) {
            for symbol in symbols {
                if let Some(s) = symbol.as_str() {
                    push_unique(&mut concepts, s.to_owned());
                }
                if concepts.len() >= max_items {
                    break;
                }
            }
        }
        if concepts.len() < max_items
            && let Some(rules) = view.get("active_rules").and_then(|v| v.as_array())
        {
            for rule in rules {
                if let Some(label) = rule.get("label").and_then(|v| v.as_str()) {
                    push_unique(&mut concepts, label.to_owned());
                }
                if concepts.len() >= max_items {
                    break;
                }
            }
        }
    }

    if let Some(store) = store {
        if concepts.len() < max_items
            && let Ok(symbols) = store.get_frequent_symbols(repo_root, max_items as u32)
        {
            for entry in symbols {
                push_unique(&mut concepts, entry.value);
                if concepts.len() >= max_items {
                    break;
                }
            }
        }
        if concepts.len() < max_items
            && let Ok(workflows) = store.get_recurring_workflows(repo_root, 3)
        {
            for workflow in workflows {
                push_unique(&mut concepts, workflow.pattern.join(" → "));
                if concepts.len() >= max_items {
                    break;
                }
            }
        }
    }

    concepts.truncate(max_items);
    concepts
}

/// Compact graph-readiness block for the wake-up pack.
fn graph_readiness_value(readiness: &atlas_core::GraphReadiness) -> Value {
    let pending: Vec<String> = readiness
        .pending_graph_changes
        .iter()
        .take(MAX_PENDING_CHANGES)
        .cloned()
        .collect();
    json!({
        "graph_built": readiness.graph_built,
        "graph_queryable": readiness.graph_queryable,
        "graph_current": readiness.graph_current,
        "stale_index": readiness.stale_index,
        "execution_state": readiness.execution_state.as_str(),
        "pending_graph_change_count": readiness.pending_graph_changes.len(),
        "pending_graph_changes": pending,
        "indexed_file_count": readiness.indexed_file_count,
        "last_indexed_at": readiness.last_indexed_at,
        "message": readiness.message,
    })
}

/// ISO-8601 UTC timestamp without an external time crate dependency.
fn format_now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86_400;
    // Approximate Gregorian date from epoch days.
    let (y, mo, da) = epoch_days_to_ymd(days);
    format!("{y:04}-{mo:02}-{da:02}T{h:02}:{m:02}:{s:02}Z")
}

fn epoch_days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    days += 719_468;
    let era = days / 146_097;
    let doe = days % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let da = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, da)
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
    let max_items = args
        .and_then(|a| a.get("max_items"))
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_MAX_ITEMS)
        .clamp(1, HARD_MAX_ITEMS);

    let mut warnings: Vec<String> = Vec::new();

    // ── graph readiness (independent of the session store) ──────────────────
    // SessionStore creates its parent dirs on open; Store does not, so create
    // the storage directory first so first-run wake-ups report `missing`
    // readiness instead of an open error.
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let readiness = match open_store(db_path) {
        Ok(store) => derive_graph_readiness(&store, &repo, db_path),
        Err(e) => derive_graph_readiness_open_failed(&repo, db_path, &e.to_string()),
    };
    if !readiness.graph_built {
        warnings.push(
            "graph has not been built yet; run build_or_update_graph before graph-backed queries"
                .to_owned(),
        );
    } else if readiness.stale_index {
        warnings.push(format!(
            "graph index is stale; {} graph-relevant file(s) changed since the last index",
            readiness.pending_graph_changes.len()
        ));
    }

    // ── session store: resume snapshot view, pending resume, global memory ───
    let session_db = derive_session_db_path(db_path);
    let store = match SessionStore::open(&session_db) {
        Ok(store) => Some(store),
        Err(e) => {
            warnings.push(format!("session store unavailable: {e}"));
            None
        }
    };

    let (snapshot_view, pending_resume, event_count, session_status) = match store.as_ref() {
        Some(store) => match store.get_session_meta(&session_id) {
            Ok(Some(_)) => {
                let pending_resume = store
                    .get_resume_snapshot(&session_id)
                    .ok()
                    .flatten()
                    .is_some_and(|snapshot| !snapshot.consumed);
                match store.build_resume_view(&session_id, agent_id.as_deref(), agent_id.is_none())
                {
                    Ok(view) => {
                        let event_count = view
                            .get("event_count")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        (Some(view), pending_resume, event_count, "active")
                    }
                    Err(e) => {
                        warnings.push(format!("resume snapshot unavailable: {e}"));
                        (None, pending_resume, 0, "unavailable")
                    }
                }
            }
            Ok(None) => (None, false, 0, "no_session"),
            Err(e) => {
                warnings.push(format!("session metadata unavailable: {e}"));
                (None, false, 0, "unavailable")
            }
        },
        None => (None, false, 0, "unavailable"),
    };

    // ── current focus: last user intent + bounded recent reasoning ───────────
    let last_intent = snapshot_view
        .as_ref()
        .and_then(|view| view.get("last_user_intent"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            // Events recorded through the shared service store prompt text under
            // `payload.prompt`, so `last_user_intent` stays empty; scan the most
            // recent UserIntent event for the prompt as a fallback.
            let events = store
                .as_ref()
                .and_then(|store| store.list_events(&session_id).ok())?;
            events.iter().rev().find_map(|event| {
                if event.event_type != SessionEventType::UserIntent {
                    return None;
                }
                let payload: Value = serde_json::from_str(&event.payload_json).ok()?;
                // The wrapper stores frontend/hook_event/metadata beside the
                // routed inner payload; only the inner payload carries the
                // prompt, so never scan the wrapper (it would match `frontend`).
                payload.get("payload").and_then(extract_prompt_text)
            })
        })
        .or_else(|| topic.clone());
    let reasoning: Vec<Value> = snapshot_view
        .as_ref()
        .and_then(|view| view.get("recent_reasoning"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .take(max_items)
                .map(|entry| {
                    json!({
                        "summary": entry.get("summary").cloned().unwrap_or(Value::Null),
                        "source_id": entry.get("source_id").cloned().unwrap_or(Value::Null),
                        "at": entry.get("at").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let current_focus = json!({
        "intent": last_intent,
        "reasoning": reasoning,
    });

    // ── recent decisions: snapshot entries + topic-matched decision memory ───
    let snapshot_decisions: Vec<Value> = snapshot_view
        .as_ref()
        .and_then(|view| view.get("recent_decisions"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    normalize_decision(
                        entry.get("summary"),
                        entry.get("rationale"),
                        entry.get("at"),
                        None,
                        &[],
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let topic_hits: Vec<Value> = if let Some(topic) = topic.as_deref() {
        if let Some(store) = store.as_ref() {
            store
                .search_decisions(&repo, topic, None, max_items)
                .map(|hits| normalize_decision_hits(&hits))
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let recent_decisions = merge_decisions(&snapshot_decisions, topic_hits, max_items);

    // ── critical memories + recent feedback from the content store ───────────
    let content_db = atlas_adapters::derive_content_db_path(db_path);
    let content_store = ContentStore::open(&content_db).ok();
    let repo_filters = |source_type: Option<String>| SearchFilters {
        session_id: None,
        agent_id: None,
        source_type,
        repo_root: None,
        repo_roots: vec![repo.clone()],
    };

    let memory_entry = |source_id: &str| -> Option<Value> {
        let cs = content_store.as_ref()?;
        let source = cs.get_source(source_id).ok().flatten()?;
        if source.source_type == "hook_event" {
            return None;
        }
        let chunk_count = cs
            .get_chunks(source_id)
            .map(|chunks| chunks.len())
            .unwrap_or(0);
        Some(json!({
            "source_id": source.id,
            "label": source.label,
            "source_type": source.source_type,
            "agent_id": source.agent_id,
            "created_at": source.created_at,
            "chunk_count": chunk_count,
        }))
    };

    let mut critical_memories: Vec<Value> = Vec::new();
    let mut recent_feedback: Vec<Value> = Vec::new();
    if let Some(cs) = content_store.as_ref() {
        if let Ok(ids) = cs.recent_source_ids_by_prefix("", &repo_filters(None), max_items) {
            for id in ids {
                if let Some(entry) = memory_entry(&id) {
                    critical_memories.push(entry);
                }
                if critical_memories.len() >= max_items {
                    break;
                }
            }
        }
        if let Ok(ids) = cs.recent_source_ids_by_prefix(
            "",
            &repo_filters(Some("preference".to_owned())),
            MAX_FEEDBACK_ITEMS,
        ) {
            for id in ids {
                if let Some(entry) = memory_entry(&id) {
                    recent_feedback.push(entry);
                }
                if recent_feedback.len() >= MAX_FEEDBACK_ITEMS {
                    break;
                }
            }
        }
    }

    // ── active concepts, changed files, retrieval hints ─────────────────────
    let active_memoir_concepts =
        collect_concepts(snapshot_view.as_ref(), store.as_ref(), &repo, max_items);
    let changed_files: Vec<String> = snapshot_view
        .as_ref()
        .and_then(|view| view.get("changed_files"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .take(max_items)
                .collect()
        })
        .unwrap_or_default();
    let retrieval_hints: Vec<Value> = snapshot_view
        .as_ref()
        .and_then(|view| view.get("retrieval_hints"))
        .and_then(|v| v.as_array())
        .map(|entries| entries.iter().take(max_items).cloned().collect())
        .unwrap_or_default();

    // ── record wake-up through the shared event service ─────────────────────
    let wake_status = if session_status == "unavailable" {
        "degraded"
    } else {
        "ok"
    };
    let event_recorded = match record_agent_event(AgentEventRequest {
        repo_root: repo.clone(),
        graph_db_path: db_path.to_owned(),
        frontend: frontend.clone(),
        event: "session-start".to_owned(),
        session_id: Some(session_id.as_str().to_owned()),
        agent_id: agent_id.clone(),
        payload: json!({
            "tool": "wake_up",
            "topic": topic,
            "wake_up": { "status": wake_status, "max_items": max_items },
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
        "session_id": session_id.as_str(),
        "frontend": frontend,
        "agent_id": agent_id,
        "current_focus": current_focus,
        "recent_decisions": recent_decisions,
        "critical_memories": critical_memories,
        "recent_feedback": recent_feedback,
        "active_memoir_concepts": active_memoir_concepts,
        "changed_files": changed_files,
        "graph_readiness": graph_readiness_value(&readiness),
        "retrieval_hints": retrieval_hints,
        "generated_at": format_now_rfc3339(),
        "event_recorded": event_recorded,
        "summary": {
            "status": session_status,
            "pending_resume": pending_resume,
            "event_count": event_count,
            "decision_count": recent_decisions.len(),
            "critical_memory_count": critical_memories.len(),
            "feedback_count": recent_feedback.len(),
            "concept_count": active_memoir_concepts.len(),
            "changed_file_count": changed_files.len(),
            "retrieval_hint_count": retrieval_hints.len(),
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
                "output_format": "json",
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

    #[test]
    fn wake_up_empty_repo_memory_returns_normalized_shape() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let result = tool_wake_up(Some(&json!({})), &repo, &db_path, OutputFormat::Toon).unwrap();
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
        assert!(!pref_id.is_empty() && !design_id.is_empty());
        // Stop builds a resume snapshot; wake-up then loads and consumes it.
        tool_record_session_event(
            Some(&json!({ "event": "stop" })),
            &repo,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();

        let result = tool_wake_up(Some(&json!({})), &repo, &db_path, OutputFormat::Toon).unwrap();
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
        let feedback = body["recent_feedback"].as_array().unwrap();
        assert!(feedback.iter().any(|m| m["source_id"] == pref_id));
        assert_eq!(body["event_recorded"]["resume_loaded"], true);
        assert_eq!(body["event_recorded"]["lifecycle_status"], "loaded");
        assert_eq!(body["summary"]["recorded"], "ok");
        // Pending snapshot existed, so the wake-up event is a SessionResume.
        assert_eq!(
            last_event_type(&db_path, &repo),
            SessionEventType::SessionResume
        );

        // Second wake-up: snapshot was consumed, no pending resume.
        let result = tool_wake_up(Some(&json!({})), &repo, &db_path, OutputFormat::Toon).unwrap();
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
            tool_wake_up(Some(&json!({})), &repo_str, &db_path, OutputFormat::Toon).unwrap();
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
            OutputFormat::Toon,
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
            OutputFormat::Toon,
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
            OutputFormat::Toon,
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
            OutputFormat::Toon,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["critical_memories"].as_array().unwrap().len(), 3);
        assert_eq!(body["summary"]["critical_memory_count"], 3);
    }
}
