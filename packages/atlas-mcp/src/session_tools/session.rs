//! Session identity and continuity tools: `get_session_status`,
//! `compact_session`, `resume_session`, and best-effort session event
//! emission for the existing continuity tools.

use anyhow::Result;
use atlas_adapters::derive_session_db_path;
use atlas_core::BudgetReport;
use atlas_session::{
    AgentMemorySummary, CurationResult, NewSessionEvent, ResumeSnapshot, SessionEventType,
    SessionMeta, SessionStore,
};
use serde_json::Value;
use tracing::warn;

use crate::output::OutputFormat;

use super::{
    inject_budget_metadata, load_budget_policy, mcp_session_id, resolve_session_id,
    tool_result_value,
};

// ---------------------------------------------------------------------------
// CM7: best-effort session event emission for existing continuity tools
// ---------------------------------------------------------------------------

/// Emit a session event after a successful tool call. Called from the tool
/// dispatcher for the four continuity tools. Failures are logged and swallowed
/// — they must never block the primary tool result.
pub fn emit_session_event_best_effort(
    tool_name: &str,
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
) {
    let Some((event_type, payload)) = continuity_event_spec(tool_name, args) else {
        return;
    };

    let session_id = mcp_session_id(repo_root);
    let session_db = derive_session_db_path(db_path);

    let outcome: std::result::Result<(), Box<dyn std::error::Error>> = (|| {
        let mut store = SessionStore::open(&session_db)?;
        store.upsert_session_meta(session_id.clone(), repo_root, "mcp", None)?;
        store.append_event(NewSessionEvent {
            session_id,
            event_type,
            priority: 0,
            payload,
            created_at: None,
        })?;
        Ok(())
    })();

    if let Err(e) = outcome {
        warn!(tool = tool_name, err = %e, "session event emit failed (best-effort, ignored)");
    }
}

/// Map a tool name to the event type and payload it should emit, or `None` if
/// the tool is not a continuity tool.
pub(crate) fn continuity_event_spec(
    tool_name: &str,
    args: Option<&Value>,
) -> Option<(SessionEventType, Value)> {
    match tool_name {
        "query_graph" => {
            let text = args
                .and_then(|a| a.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some((
                SessionEventType::ContextRequest,
                serde_json::json!({"tool": "query_graph", "query": text}),
            ))
        }
        "get_impact_radius" => {
            let files = args
                .and_then(|a| a.get("change_source"))
                .and_then(|value| value.as_object())
                .filter(|change_source| {
                    change_source.get("kind").and_then(|value| value.as_str()) == Some("files")
                })
                .and_then(|change_source| change_source.get("files"))
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            Some((
                SessionEventType::ImpactAnalysis,
                serde_json::json!({"tool": "get_impact_radius", "files": files}),
            ))
        }
        "get_review_context" => {
            let files = args
                .and_then(|a| a.get("change_source"))
                .and_then(|value| value.as_object())
                .filter(|change_source| {
                    change_source.get("kind").and_then(|value| value.as_str()) == Some("files")
                })
                .and_then(|change_source| change_source.get("files"))
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            Some((
                SessionEventType::ReviewContext,
                serde_json::json!({"tool": "get_review_context", "files": files}),
            ))
        }
        "detect_changes" => {
            let change_source = args
                .and_then(|a| a.get("change_source"))
                .and_then(|value| value.as_object());
            let base = change_source
                .filter(|change_source| {
                    change_source.get("kind").and_then(|value| value.as_str()) == Some("base")
                })
                .and_then(|change_source| change_source.get("base"))
                .cloned()
                .unwrap_or(Value::Null);
            let staged = Value::Bool(
                change_source
                    .and_then(|change_source| change_source.get("kind"))
                    .and_then(|value| value.as_str())
                    == Some("staged"),
            );
            Some((
                SessionEventType::CommandRun,
                serde_json::json!({"tool": "detect_changes", "base": base, "staged": staged}),
            ))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// get_session_status
// ---------------------------------------------------------------------------

/// Return the status of the current (or specified) MCP session.
///
/// If no session exists yet for this repo, returns `"status": "no_session"`.
pub fn tool_get_session_status(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let session_id = resolve_session_id(args, repo_root);
    let agent_id = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let merge_agent_partitions = args
        .and_then(|a| a.get("merge_agent_partitions"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let session_db = derive_session_db_path(db_path);

    let (meta, event_count, snapshot, agent_summary, warnings): (
        Option<SessionMeta>,
        i64,
        Option<ResumeSnapshot>,
        AgentMemorySummary,
        Vec<String>,
    ) = match SessionStore::open(&session_db) {
        Ok(store) => {
            let meta = store.get_session_meta(&session_id)?;
            if meta.is_some() {
                let event_count = store
                    .build_resume_view(&session_id, agent_id.as_deref(), merge_agent_partitions)?
                    .get("event_count")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0);
                let snapshot = store.get_resume_snapshot(&session_id)?;
                let agent_summary = store.summarize_agent_memory(
                    &session_id,
                    agent_id.as_deref(),
                    merge_agent_partitions,
                )?;
                (meta, event_count, snapshot, agent_summary, Vec::new())
            } else {
                (
                    None,
                    0,
                    None,
                    AgentMemorySummary {
                        merged_view: merge_agent_partitions || agent_id.is_none(),
                        requested_agent_id: agent_id.clone(),
                        ..AgentMemorySummary::default()
                    },
                    Vec::new(),
                )
            }
        }
        Err(e) => (
            None,
            0,
            None,
            AgentMemorySummary {
                merged_view: merge_agent_partitions || agent_id.is_none(),
                requested_agent_id: agent_id.clone(),
                ..AgentMemorySummary::default()
            },
            vec![format!("session store unavailable: {e}")],
        ),
    };

    let (
        status,
        repo_root_value,
        frontend,
        worktree_id,
        created_at,
        updated_at,
        last_resume_at,
        last_compaction_at,
    ) = if let Some(meta) = &meta {
        (
            "active",
            Some(meta.repo_root.clone()),
            Some(meta.frontend.clone()),
            meta.worktree_id.clone(),
            Some(meta.created_at.clone()),
            Some(meta.updated_at.clone()),
            meta.last_resume_at.clone(),
            meta.last_compaction_at.clone(),
        )
    } else {
        (
            "no_session",
            Some(repo_root.to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
        )
    };

    let result = serde_json::json!({
        "tool": "get_session_status",
        "session_id": session_id.as_str(),
        "agent_id": agent_id,
        "merged_agent_view": agent_summary.merged_view,
        "status": status,
        "repo_root": repo_root_value,
        "frontend": frontend,
        "worktree_id": worktree_id,
        "created_at": created_at,
        "updated_at": updated_at,
        "last_resume_at": last_resume_at,
        "last_compaction_at": last_compaction_at,
        "event_count": event_count,
        "resume_snapshot_exists": snapshot.is_some(),
        "snapshot_consumed": snapshot.as_ref().map(|s| s.consumed),
        "agent_partitions": agent_summary.partitions,
        "delegated_tasks": agent_summary.delegated_tasks,
        "agent_responsibilities": agent_summary.responsibilities,
        "summary": {
            "status": status,
            "has_session": meta.is_some(),
            "event_count": event_count,
            "partition_count": agent_summary.partitions.len(),
            "delegated_task_count": agent_summary.delegated_tasks.len(),
            "responsibility_count": agent_summary.responsibilities.len(),
            "resume_snapshot_exists": snapshot.is_some(),
        },
        "warnings": warnings,
    });

    tool_result_value(&result, output_format)
}

// ---------------------------------------------------------------------------
// compact_session
// ---------------------------------------------------------------------------

/// Compact and curate the session event ledger.
///
/// Removes stale low-value events, merges repeated actions, deduplicates
/// reasoning outputs, and promotes high-value events to a higher priority.
/// Returns curation stats. Safe to call repeatedly; no-ops when nothing needs
/// compaction.
pub fn tool_compact_session(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let policy = load_budget_policy(repo_root)?;
    let session_id = resolve_session_id(args, repo_root);
    let session_db = derive_session_db_path(db_path);

    let mut store = match SessionStore::open(&session_db) {
        Ok(s) => s,
        Err(e) => {
            let result = serde_json::json!({
                "tool": "compact_session",
                "session_id": session_id.as_str(),
                "before_counts": { "events": 0 },
                "after_counts": { "events": 0 },
                "promoted_events": 0,
                "removed_events": 0,
                "merged_groups": 0,
                "decayed_events": 0,
                "deduplicated_events": 0,
                "summary": {
                    "status": "no_session",
                    "no_op": true,
                    "events_before": 0,
                    "events_after": 0,
                    "events_removed": 0,
                },
                "warnings": [format!("session store unavailable: {e}")],
            });
            return tool_result_value(&result, output_format);
        }
    };

    store.upsert_session_meta(session_id.clone(), repo_root, "mcp", None)?;

    let CurationResult {
        events_before,
        events_after,
        merged_count,
        decayed_count,
        deduplicated_count,
        promoted_count,
    } = store.compact_session(&session_id)?;
    let removed_events = decayed_count + deduplicated_count;

    let result = serde_json::json!({
        "tool": "compact_session",
        "session_id": session_id.as_str(),
        "before_counts": { "events": events_before },
        "after_counts": { "events": events_after },
        "promoted_events": promoted_count,
        "removed_events": removed_events,
        "merged_groups": merged_count,
        "decayed_events": decayed_count,
        "deduplicated_events": deduplicated_count,
        "summary": {
            "status": "ok",
            "no_op": events_before == events_after && merged_count == 0 && removed_events == 0 && promoted_count == 0,
            "events_before": events_before,
            "events_after": events_after,
            "events_removed": removed_events,
        },
        "warnings": [],
    });

    let mut response = tool_result_value(&result, output_format)?;
    let emitted_bytes = serde_json::to_vec(&response)?.len();
    let budget = BudgetReport::within_budget(
        "mcp_cli_payload_serialization.max_mcp_response_bytes",
        policy
            .mcp_cli_payload_serialization
            .mcp_response_bytes
            .default_limit,
        emitted_bytes,
    );
    inject_budget_metadata(&mut response, &budget);
    Ok(response)
}

// ---------------------------------------------------------------------------
// resume_session
// ---------------------------------------------------------------------------

/// Return the resume snapshot for the current (or specified) session.
///
/// Builds a snapshot on demand if one does not exist yet.  Marks the snapshot
/// consumed by default so agents do not receive stale context on subsequent
/// calls.
pub fn tool_resume_session(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let session_id = resolve_session_id(args, repo_root);
    let agent_id = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let merge_agent_partitions = args
        .and_then(|a| a.get("merge_agent_partitions"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mark_consumed = args
        .and_then(|a| a.get("mark_consumed"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let session_db = derive_session_db_path(db_path);
    let mut store = SessionStore::open(&session_db)?;

    store.upsert_session_meta(session_id.clone(), repo_root, "mcp", None)?;

    let (snapshot, snapshot_status): (ResumeSnapshot, &str) =
        match store.get_resume_snapshot(&session_id)? {
            Some(s) => (s, "existing_snapshot"),
            None => (store.build_resume(&session_id)?, "built_snapshot"),
        };
    let snapshot_view =
        store.build_resume_view(&session_id, agent_id.as_deref(), merge_agent_partitions)?;

    if mark_consumed {
        let _ = store.mark_resume_consumed(&session_id, true);
    }

    let _ = store.append_event(NewSessionEvent {
        session_id: session_id.clone(),
        event_type: SessionEventType::SessionResume,
        priority: 1,
        payload: serde_json::json!({"tool": "resume_session"}),
        created_at: None,
    });

    let event_count = snapshot_view
        .get("event_count")
        .and_then(|value| value.as_i64())
        .unwrap_or(snapshot.event_count);
    let merged_agent_view =
        merge_agent_partitions || args.and_then(|a| a.get("agent_id")).is_none();
    let result = serde_json::json!({
        "tool": "resume_session",
        "session_id": snapshot.session_id.as_str(),
        "agent_id": agent_id,
        "merged_agent_view": merged_agent_view,
        "snapshot_status": snapshot_status,
        "snapshot": snapshot_view,
        "event_count": event_count,
        "consumed": mark_consumed,
        "created_at": snapshot.created_at,
        "summary": {
            "event_count": event_count,
            "merged_agent_view": merged_agent_view,
            "snapshot_consumed": mark_consumed,
        },
        "warnings": [],
    });

    tool_result_value(&result, output_format)
}
