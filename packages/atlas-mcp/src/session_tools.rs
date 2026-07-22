//! CM7 — MCP session-continuity and saved-context tools.
//!
//! Implements the six new MCP tools that expose session identity, resume
//! snapshots, and content-store search/save/purge to agents.  Session event
//! emission for the four existing continuity tools is also handled here via
//! `emit_session_event_best_effort`.
//!
//! Design constraints (from Core Design Rules):
//! - Never store saved context in the graph database.
//! - Never block the primary tool result on session persistence failure.
//! - Return previews / pointers instead of raw large blobs.
//! - Restore context through retrieval, not transcript replay.

use anyhow::Result;
use atlas_adapters::bridge::{bridge_file_count, purge_all_bridge_files};
use atlas_adapters::{
    ArtifactIdentity, RedactionRules, derive_bridge_dir, derive_content_db_path,
    derive_session_db_path, extract_decision_event_with_details, generate_source_id,
    load_redaction_rules_file, redact_text_with_rules,
};
use atlas_contentstore::{ContentStore, OutputRouting, SearchFilters, SourceMeta};
use atlas_core::{BudgetManager, BudgetPolicy, BudgetReport};
use atlas_session::{
    AgentMemorySummary, CurationResult, DecisionSearchHit, GlobalAccessEntry,
    GlobalWorkflowPattern, NewSessionEvent, ResumeSnapshot, SessionEventType, SessionId,
    SessionMeta, SessionStore,
};
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

use crate::output::OutputFormat;
use crate::tool_result::tool_result_value as build_tool_result_value;

/// Derive the MCP session id for a given repo root.
///
/// Uses `worktree_id = ""` and `frontend = "mcp"` as stable anchors.
fn mcp_session_id(repo_root: &str) -> SessionId {
    SessionId::derive(repo_root, "", "mcp")
}

fn open_session_store_best_effort(db_path: &str) -> Option<SessionStore> {
    let session_db = derive_session_db_path(db_path);
    SessionStore::open(&session_db).ok()
}

pub(crate) fn search_decisions_best_effort(
    repo_root: &str,
    db_path: &str,
    session_id: Option<&str>,
    query: &str,
    limit: usize,
) -> Vec<DecisionSearchHit> {
    let Some(store) = open_session_store_best_effort(db_path) else {
        return Vec::new();
    };

    if let Some(session_id) = session_id {
        let current = store
            .search_decisions(repo_root, query, Some(session_id), limit)
            .unwrap_or_default();
        if !current.is_empty() {
            return current;
        }
    }

    store
        .search_decisions(repo_root, query, None, limit)
        .unwrap_or_default()
}

pub(crate) fn search_decisions_strict_best_effort(
    repo_root: &str,
    db_path: &str,
    session_id: Option<&str>,
    query: &str,
    limit: usize,
) -> Vec<DecisionSearchHit> {
    let Some(store) = open_session_store_best_effort(db_path) else {
        return Vec::new();
    };

    store
        .search_decisions(repo_root, query, session_id, limit)
        .unwrap_or_default()
}

pub(crate) fn decision_hits_json(hits: &[DecisionSearchHit]) -> Value {
    serde_json::to_value(hits).unwrap_or_else(|_| Value::Array(Vec::new()))
}

pub(crate) fn record_mcp_decision_best_effort(
    repo_root: &str,
    db_path: &str,
    summary: &str,
    rationale: Option<&str>,
    details: Value,
) {
    let session_id = mcp_session_id(repo_root);
    let session_db = derive_session_db_path(db_path);
    let outcome: std::result::Result<(), Box<dyn std::error::Error>> = (|| {
        let mut store = SessionStore::open(&session_db)?;
        store.upsert_session_meta(session_id.clone(), repo_root, "mcp", None)?;
        store.append_event(
            extract_decision_event_with_details(summary, rationale, details).bind(session_id),
        )?;
        Ok(())
    })();

    if let Err(error) = outcome {
        warn!(err = %error, "MCP decision event emit failed (best-effort, ignored)");
    }
}

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
fn continuity_event_spec(
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
                .and_then(|a| a.get("files"))
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            Some((
                SessionEventType::ImpactAnalysis,
                serde_json::json!({"tool": "get_impact_radius", "files": files}),
            ))
        }
        "get_review_context" => {
            let files = args
                .and_then(|a| a.get("files"))
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            Some((
                SessionEventType::ReviewContext,
                serde_json::json!({"tool": "get_review_context", "files": files}),
            ))
        }
        "detect_changes" => {
            let base = args
                .and_then(|a| a.get("base"))
                .cloned()
                .unwrap_or(Value::Null);
            let staged = args
                .and_then(|a| a.get("staged"))
                .cloned()
                .unwrap_or(Value::Bool(false));
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

// ---------------------------------------------------------------------------
// search_saved_context
// ---------------------------------------------------------------------------

/// Search saved artifacts in the content store using BM25 + trigram fallback.
///
/// Returns previews (first 256 chars) instead of full blobs.  Use the
/// returned `source_id` with subsequent searches to narrow to one source.
pub fn tool_search_saved_context(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let query = args
        .and_then(|a| a.get("query"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: query"))?;
    let cross_session = args
        .and_then(|a| a.get("cross_session"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // When cross_session=true, session_id filter is dropped so all sessions
    // in the repo are searched. The repo_root filter is still applied.
    let session_id_filter = if cross_session {
        None
    } else {
        args.and_then(|a| a.get("session_id"))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let agent_id_filter = if cross_session {
        None
    } else {
        args.and_then(|a| a.get("agent_id"))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let merge_agent_partitions = args
        .and_then(|a| a.get("merge_agent_partitions"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let repo_root_filter = if cross_session {
        Some(repo_root.to_string())
    } else {
        None
    };
    let source_type_filter = args
        .and_then(|a| a.get("source_type"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let requested_limit = args
        .and_then(|a| a.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;
    let limit = budgets.resolve_limit(
        policy.content_saved_context_lookup.sources,
        "content_saved_context_lookup.max_sources",
        Some(requested_limit),
    );

    let content_db = derive_content_db_path(db_path);
    let mut cs = ContentStore::open(&content_db)?;
    let _ = cs.migrate();

    let filters = SearchFilters {
        session_id: session_id_filter.clone(),
        agent_id: if merge_agent_partitions {
            None
        } else {
            agent_id_filter.clone()
        },
        source_type: source_type_filter,
        repo_root: repo_root_filter,
    };

    let chunks = cs.search_with_fallback(query, &filters)?;

    #[derive(Serialize)]
    struct ChunkPreview {
        source_id: String,
        chunk_id: String,
        chunk_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identity_kind: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identity_value: Option<String>,
        /// First 256 chars only — full content available via source_id.
        preview: String,
        content_type: String,
    }

    let total_matches = chunks.len();
    let linked_decisions = search_decisions_best_effort(
        repo_root,
        db_path,
        session_id_filter.as_deref(),
        query,
        limit.min(5),
    );
    let results: Vec<ChunkPreview> = chunks
        .into_iter()
        .take(limit)
        .map(|c| {
            let source = cs.get_source(&c.source_id).ok().flatten();
            ChunkPreview {
                source_id: c.source_id,
                chunk_id: c.chunk_id,
                chunk_index: c.chunk_index,
                title: c.title,
                label: source.as_ref().map(|row| row.label.clone()),
                agent_id: source.as_ref().and_then(|row| row.agent_id.clone()),
                source_type: source.as_ref().map(|row| row.source_type.clone()),
                identity_kind: source.as_ref().map(|row| row.identity_kind.clone()),
                identity_value: source.as_ref().map(|row| row.identity_value.clone()),
                preview: c.content.chars().take(256).collect(),
                content_type: c.content_type,
            }
        })
        .collect();

    if total_matches > limit {
        budgets.record_usage(
            policy.content_saved_context_lookup.sources,
            "content_saved_context_lookup.max_sources",
            limit,
            total_matches,
            true,
        );
    }

    let total = results.len();
    let mut response = tool_result_value(
        &serde_json::json!({
            "query": query,
            "agent_id": agent_id_filter,
            "merged_agent_view": merge_agent_partitions || cross_session,
            "results": results,
            "total": total,
            "linked_decisions": linked_decisions,
        }),
        output_format,
    )?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "content_saved_context_lookup.max_sources",
            limit,
            requested_limit.max(total_matches),
        ),
    );
    if !linked_decisions.is_empty() {
        let source_ids = linked_decisions
            .iter()
            .flat_map(|hit| hit.decision.source_ids.iter().cloned())
            .take(5)
            .collect::<Vec<_>>();
        record_mcp_decision_best_effort(
            repo_root,
            db_path,
            &format!("reuse prior decision during saved-context lookup: {query}"),
            Some("saved-context query matched stored decision memory"),
            serde_json::json!({
                "query": query,
                "conclusion": "prior decision reused during saved-context lookup",
                "source_ids": source_ids,
                "evidence": linked_decisions.iter().take(3).map(|hit| serde_json::json!({
                    "decision_id": hit.decision.decision_id,
                    "summary": hit.decision.summary,
                    "relevance_score": hit.relevance_score,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    Ok(response)
}

pub fn tool_search_decisions(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let query = args
        .and_then(|a| a.get("query"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: query"))?;
    let session_id = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let agent_id = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let limit = args
        .and_then(|a| a.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;

    let hits = search_decisions_strict_best_effort(
        repo_root,
        db_path,
        session_id.as_deref(),
        query,
        limit,
    );
    tool_result_value(
        &serde_json::json!({
            "query": query,
            "session_id": session_id,
            "agent_id": agent_id,
            "results": hits,
            "total": hits.len(),
        }),
        output_format,
    )
}

// ---------------------------------------------------------------------------
// save_context_artifact
// ---------------------------------------------------------------------------

/// Index and store a large output in the content store.
///
/// Routing:
/// - Small (≤ 512 B) → returned raw, not indexed.
/// - Medium (≤ 4 KB) → indexed, preview returned.
/// - Large           → indexed, pointer (`source_id`) returned only.
///
/// The `source_id` is derived from a structured identity seed plus content
/// hash so identical logical artifacts are deduplicated automatically.
pub fn tool_save_context_artifact(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let content = args
        .and_then(|a| a.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: content"))?;
    let label = args
        .and_then(|a| a.get("label"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: label"))?;
    let source_type = args
        .and_then(|a| a.get("source_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("mcp_artifact");
    let content_type = args
        .and_then(|a| a.get("content_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("text/plain");
    let session_id_str = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| mcp_session_id(repo_root).as_str().to_string());
    let agent_id = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let redaction_rules = load_redaction_rules(repo_root)?;
    let sanitized_content = redact_text_with_rules(content, &redaction_rules);

    let identity = ArtifactIdentity::artifact_label(label);
    let source_id = generate_source_id(&identity, &sanitized_content);

    let content_db = derive_content_db_path(db_path);
    let mut cs = ContentStore::open(&content_db)?;
    let _ = cs.migrate();

    let meta = SourceMeta {
        id: source_id,
        session_id: Some(session_id_str.clone()),
        agent_id: agent_id.clone(),
        source_type: source_type.to_string(),
        label: label.to_string(),
        repo_root: Some(repo_root.to_string()),
        identity_kind: identity.kind_str().to_owned(),
        identity_value: identity.value().to_owned(),
    };

    let routing = cs.route_output(meta, &sanitized_content, content_type)?;
    let content_size_bytes = sanitized_content.len();

    let (storage_mode, source_id_value, preview, inline_content, retrieval_hint) = match routing {
        OutputRouting::Raw(raw) => (
            "raw_inline",
            Value::Null,
            Value::String(raw.chars().take(256).collect()),
            Value::String(raw),
            Value::Null,
        ),
        OutputRouting::Preview {
            source_id: sid,
            preview,
        } => (
            "indexed_preview",
            Value::String(sid.clone()),
            Value::String(preview),
            Value::Null,
            Value::String(format!(
                "use read_saved_context with source_id={sid} to retrieve full content"
            )),
        ),
        OutputRouting::Pointer { source_id: sid } => (
            "indexed_pointer",
            Value::String(sid.clone()),
            Value::Null,
            Value::Null,
            Value::String(format!(
                "use read_saved_context with source_id={sid} to retrieve content"
            )),
        ),
    };

    let chunk_count = source_id_value
        .as_str()
        .map(|sid| cs.get_chunks(sid).map(|chunks| chunks.len()).unwrap_or(0))
        .unwrap_or(0);
    let resource_link = source_id_value.as_str().map(|sid| {
        serde_json::json!({
            "type": "resource_link",
            "uri": format!("atlas://saved-context/{sid}"),
            "name": "saved_context",
            "title": label,
            "mime_type": content_type,
        })
    });

    let result = serde_json::json!({
        "tool": "save_context_artifact",
        "storage_mode": storage_mode,
        "source_id": source_id_value,
        "label": label,
        "source_type": source_type,
        "agent_id": agent_id,
        "preview": preview,
        "inline_content": inline_content,
        "content_size_bytes": content_size_bytes,
        "chunk_count": chunk_count,
        "resource_link": resource_link,
        "retrieval_hint": retrieval_hint,
        "summary": {
            "session_id": session_id_str,
            "stored": storage_mode != "raw_inline",
            "inline": storage_mode == "raw_inline",
            "content_type": content_type,
        }
    });

    tool_result_value(&result, output_format)
}

// ---------------------------------------------------------------------------
// get_context_stats
// ---------------------------------------------------------------------------

/// Return storage statistics for the current (or specified) session.
pub fn tool_get_context_stats(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let policy = load_budget_policy(repo_root)?;
    let session_id = resolve_session_id(args, repo_root);
    let agent_id = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let session_db = derive_session_db_path(db_path);
    let content_db = derive_content_db_path(db_path);
    let bridge_dir = derive_bridge_dir(db_path);

    // Session stats (best-effort — store may not exist for brand-new repos).
    let event_count = SessionStore::open(&session_db)
        .ok()
        .and_then(|s| s.list_events(&session_id).ok())
        .map(|e| e.len())
        .unwrap_or(0);

    // Content stats + retrieval index state (best-effort).
    let (source_count, chunk_count) = ContentStore::open(&content_db)
        .ok()
        .and_then(|mut cs| {
            let _ = cs.migrate();
            cs.stats(Some(session_id.as_str()), agent_id.as_deref())
                .ok()
        })
        .unwrap_or((0, 0));

    // Retrieval index status for this repo (best-effort).
    let retrieval_index = ContentStore::open(&content_db)
        .ok()
        .and_then(|mut cs| {
            let _ = cs.migrate();
            cs.get_index_status(repo_root).ok().flatten()
        })
        .map(|s| {
            serde_json::json!({
                "state": s.state,
                "files_discovered": s.files_discovered,
                "files_indexed": s.files_indexed,
                "chunks_written": s.chunks_written,
                "chunks_reused": s.chunks_reused,
                "last_indexed_at": s.last_indexed_at,
                "last_error": s.last_error,
                "updated_at": s.updated_at,
                "searchable": s.state == atlas_contentstore::IndexState::Indexed,
            })
        });

    // Bridge artifact count.
    let bridge_file_pending = bridge_file_count(&bridge_dir);

    let mut response = tool_result_value(
        &serde_json::json!({
            "session_id": session_id.as_str(),
            "agent_id": agent_id,
            "event_count": event_count,
            "source_count": source_count,
            "chunk_count": chunk_count,
            "bridge_file_count": bridge_file_pending,
            "content_db_path": content_db,
            "session_db_path": session_db,
            "bridge_dir_path": bridge_dir.to_string_lossy(),
            "retrieval_index": retrieval_index,
        }),
        output_format,
    )?;
    let emitted_bytes = serde_json::to_vec(&response)?.len();
    inject_budget_metadata(
        &mut response,
        &BudgetReport::within_budget(
            "mcp_cli_payload_serialization.max_mcp_response_bytes",
            policy
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .default_limit,
            emitted_bytes,
        ),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// read_saved_context (MCP13)
// ---------------------------------------------------------------------------

/// Maximum bytes returned in a single `read_saved_context` call when the
/// caller does not supply an explicit `max_bytes` cap.
const DEFAULT_READ_MAX_BYTES: usize = 65_536; // 64 KiB

/// Retrieve the full content of a saved artifact by `source_id`.
///
/// Scoping rules:
/// - If `session_id` is supplied, it must match the artifact's stored session.
/// - If `repo_root` is supplied (always passed from the caller), it must match
///   the artifact's stored repo_root when one was recorded.
///
/// Paging:
/// - `chunk_offset` (default 0): first chunk index to include in this response.
/// - `max_bytes` (default 64 KiB): byte cap on returned content.
///   When the remaining content exceeds the cap the response includes
///   `truncated: true`, `next_chunk_offset`, and a `continuation_hint`.
pub fn tool_read_saved_context(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let source_id = args
        .and_then(|a| a.get("source_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: source_id"))?;

    let caller_session_id = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let caller_agent_id = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let merge_agent_partitions = args
        .and_then(|a| a.get("merge_agent_partitions"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let chunk_offset = args
        .and_then(|a| a.get("chunk_offset"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let requested_max_bytes = args
        .and_then(|a| a.get("max_bytes"))
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_READ_MAX_BYTES);
    let max_bytes = budgets.resolve_limit(
        policy.mcp_cli_payload_serialization.saved_context_bytes,
        "mcp_cli_payload_serialization.max_saved_context_bytes",
        Some(requested_max_bytes),
    );

    let content_db = derive_content_db_path(db_path);
    let mut cs = ContentStore::open(&content_db)?;
    let _ = cs.migrate();

    let summary_budget = |observed| {
        budgets.summary(
            "mcp_cli_payload_serialization.max_saved_context_bytes",
            max_bytes,
            requested_max_bytes.max(observed),
        )
    };

    let build_error_result = |access_status: &str, warning: &str| {
        serde_json::json!({
            "tool": "read_saved_context",
            "found": false,
            "access_status": access_status,
            "source_id": source_id,
            "content": Value::Null,
            "content_format": Value::Null,
            "chunk_offset": chunk_offset,
            "next_chunk_offset": Value::Null,
            "truncated": false,
            "summary": {
                "status": access_status,
                "byte_count": 0,
                "chunk_count": 0,
                "returned_chunk_count": 0,
            },
            "warnings": [warning],
        })
    };

    let source = match cs.get_source(source_id)? {
        Some(s) => s,
        None => {
            let mut response = tool_result_value(
                &build_error_result("not_found", "artifact not found"),
                output_format,
            )?;
            inject_budget_metadata(&mut response, &summary_budget(max_bytes));
            return Ok(response);
        }
    };

    if let Some(ref caller_sid) = caller_session_id
        && source.session_id.as_deref() != Some(caller_sid.as_str())
    {
        let mut response = tool_result_value(
            &build_error_result(
                "session_mismatch",
                "artifact not accessible from this session",
            ),
            output_format,
        )?;
        inject_budget_metadata(&mut response, &summary_budget(max_bytes));
        return Ok(response);
    }

    if !merge_agent_partitions
        && let Some(ref caller_agent_id) = caller_agent_id
        && source.agent_id.as_deref() != Some(caller_agent_id.as_str())
    {
        let mut response = tool_result_value(
            &build_error_result(
                "agent_mismatch",
                "artifact not accessible from this agent partition",
            ),
            output_format,
        )?;
        inject_budget_metadata(&mut response, &summary_budget(max_bytes));
        return Ok(response);
    }

    if let Some(ref artifact_repo) = source.repo_root
        && artifact_repo != repo_root
    {
        let mut response = tool_result_value(
            &build_error_result(
                "repo_mismatch",
                "artifact not accessible from this repository",
            ),
            output_format,
        )?;
        inject_budget_metadata(&mut response, &summary_budget(max_bytes));
        return Ok(response);
    }

    let all_chunks = cs.get_chunks(source_id)?;
    let total_chunks = all_chunks.len();
    let content_format = all_chunks
        .first()
        .map(|chunk| chunk.content_type.clone())
        .unwrap_or_else(|| "text/plain".to_owned());
    let remaining_chunks: Vec<_> = all_chunks
        .into_iter()
        .filter(|c| c.chunk_index >= chunk_offset)
        .collect();

    let mut content_parts: Vec<String> = Vec::new();
    let mut returned_chunk_ids: Vec<String> = Vec::new();
    let mut bytes_used: usize = 0;
    let mut last_included_index: Option<usize> = None;
    let mut last_included_chunk_id: Option<String> = None;
    let mut truncated = false;
    let mut next_chunk_offset: Option<usize> = None;
    let mut next_chunk_id: Option<String> = None;

    for chunk in &remaining_chunks {
        let chunk_bytes = chunk.content.len();
        if bytes_used + chunk_bytes > max_bytes {
            truncated = true;
            next_chunk_offset = Some(chunk.chunk_index);
            next_chunk_id = Some(chunk.chunk_id.clone());
            break;
        }
        bytes_used += chunk_bytes;
        last_included_index = Some(chunk.chunk_index);
        last_included_chunk_id = Some(chunk.chunk_id.clone());
        returned_chunk_ids.push(chunk.chunk_id.clone());
        content_parts.push(chunk.content.clone());
    }

    let content = content_parts.join("\n");
    let total_byte_count: usize = remaining_chunks.iter().map(|c| c.content.len()).sum();
    if truncated {
        budgets.record_usage(
            policy.mcp_cli_payload_serialization.saved_context_bytes,
            "mcp_cli_payload_serialization.max_saved_context_bytes",
            max_bytes,
            total_byte_count,
            true,
        );
    }

    let result = serde_json::json!({
        "tool": "read_saved_context",
        "found": true,
        "access_status": "ok",
        "source_id": source.id,
        "artifact_kind": source.source_type,
        "identity_kind": source.identity_kind,
        "identity_value": source.identity_value,
        "created_at": source.created_at,
        "session_id": source.session_id,
        "agent_id": source.agent_id,
        "merged_agent_view": merge_agent_partitions,
        "label": source.label,
        "content": content,
        "content_format": content_format,
        "byte_count": total_byte_count,
        "chunk_count": total_chunks,
        "chunk_offset": chunk_offset,
        "last_included_chunk": last_included_index,
        "last_included_chunk_id": last_included_chunk_id,
        "returned_chunk_ids": returned_chunk_ids,
        "next_chunk_offset": next_chunk_offset,
        "next_chunk_id": next_chunk_id,
        "continuation_hint": next_chunk_offset.map(|next| format!(
            "call read_saved_context with source_id={source_id} chunk_offset={next} to read more"
        )),
        "truncated": truncated,
        "summary": {
            "status": "ok",
            "byte_count": total_byte_count,
            "chunk_count": total_chunks,
            "returned_chunk_count": content_parts.len(),
        },
        "warnings": [],
    });

    let mut response = tool_result_value(&result, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "mcp_cli_payload_serialization.max_saved_context_bytes",
            max_bytes,
            requested_max_bytes.max(total_byte_count),
        ),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// purge_saved_context
// ---------------------------------------------------------------------------

/// Delete saved artifacts from the content store.
///
/// Supports two modes:
/// - `session_id` provided → delete all sources for that session.
/// - `session_id` omitted  → age-based cleanup: delete sources older than
///   `keep_days` days (default 30).
///
/// Pass `purge_bridge_files: true` to also delete pending bridge artifact
/// files from `.atlas/bridge/`.
pub fn tool_purge_saved_context(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let policy = load_budget_policy(repo_root)?;
    let session_id_filter = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let agent_id_filter = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let keep_days = args
        .and_then(|a| a.get("keep_days"))
        .and_then(|v| v.as_u64())
        .unwrap_or(30) as u32;
    let purge_bridge = args
        .and_then(|a| a.get("purge_bridge_files"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let content_db = derive_content_db_path(db_path);
    let bridge_dir = derive_bridge_dir(db_path);

    let mut cs = ContentStore::open(&content_db)?;
    let _ = cs.migrate();

    let mode = if session_id_filter.is_some() {
        "session"
    } else {
        "age_based"
    };
    let (before_sources, before_chunks) =
        cs.stats(session_id_filter.as_deref(), agent_id_filter.as_deref())?;
    let deleted_sources = if let Some(ref sid) = session_id_filter {
        cs.delete_session_sources(sid, agent_id_filter.as_deref())?
    } else {
        if crate::runtime_context::current().is_ok()
            && !crate::elicitation::confirm_age_based_purge()?
        {
            return Err(anyhow::anyhow!(
                "purge_saved_context without session_id requires explicit elicited confirmation"
            ));
        }
        cs.cleanup(keep_days)?
    };
    let (after_sources, after_chunks) =
        cs.stats(session_id_filter.as_deref(), agent_id_filter.as_deref())?;
    let deleted_chunks = before_chunks.saturating_sub(after_chunks);
    let deleted_bridge = if purge_bridge {
        purge_all_bridge_files(&bridge_dir)
    } else {
        0
    };

    let result = serde_json::json!({
        "tool": "purge_saved_context",
        "mode": mode,
        "session_id": session_id_filter,
        "agent_id": agent_id_filter,
        "cutoff_days": keep_days,
        "deleted_sources": deleted_sources,
        "deleted_chunks": deleted_chunks,
        "deleted_bridge_files": deleted_bridge,
        "summary": {
            "status": "ok",
            "sources_before": before_sources,
            "sources_after": after_sources,
            "chunks_before": before_chunks,
            "chunks_after": after_chunks,
        },
        "warnings": [],
    });

    let mut response = tool_result_value(&result, output_format)?;
    let emitted_bytes = serde_json::to_vec(&response)?.len();
    inject_budget_metadata(
        &mut response,
        &BudgetReport::within_budget(
            "mcp_cli_payload_serialization.max_mcp_response_bytes",
            policy
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .default_limit,
            emitted_bytes,
        ),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// cross_session_search  (CM11)
// ---------------------------------------------------------------------------

/// Search saved context artifacts across **all** sessions for this repo.
///
/// Unlike `search_saved_context` (which defaults to the current session),
/// this tool always scans every session stored under `repo_root`, making it
/// suitable for cross-session recall workflows.
pub fn tool_cross_session_search(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let query = args
        .and_then(|a| a.get("query"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: query"))?;
    let source_type_filter = args
        .and_then(|a| a.get("source_type"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let agent_id_filter = args
        .and_then(|a| a.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let merge_agent_partitions = args
        .and_then(|a| a.get("merge_agent_partitions"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let requested_limit = args
        .and_then(|a| a.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;
    let limit = budgets.resolve_limit(
        policy.content_saved_context_lookup.sources,
        "content_saved_context_lookup.max_sources",
        Some(requested_limit),
    );

    let content_db = derive_content_db_path(db_path);
    let mut cs = ContentStore::open(&content_db)?;
    let _ = cs.migrate();

    // Explicitly filter by repo_root, no session_id restriction.
    let filters = SearchFilters {
        session_id: None,
        agent_id: if merge_agent_partitions {
            None
        } else {
            agent_id_filter.clone()
        },
        source_type: source_type_filter,
        repo_root: Some(repo_root.to_string()),
    };

    let chunks = cs.search_with_fallback(query, &filters)?;

    #[derive(Serialize)]
    struct CrossSessionResult {
        source_id: String,
        chunk_id: String,
        chunk_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_type: Option<String>,
        /// First 256 chars — full content retrievable via `read_saved_context`.
        preview: String,
    }

    let total_matches = chunks.len();
    let results: Vec<CrossSessionResult> = chunks
        .into_iter()
        .take(limit)
        .map(|c| {
            let source = cs.get_source(&c.source_id).ok().flatten();
            CrossSessionResult {
                source_id: c.source_id,
                chunk_id: c.chunk_id,
                chunk_index: c.chunk_index,
                session_id: source.as_ref().and_then(|s| s.session_id.clone()),
                agent_id: source.as_ref().and_then(|s| s.agent_id.clone()),
                title: c.title,
                label: source.as_ref().map(|s| s.label.clone()),
                source_type: source.as_ref().map(|s| s.source_type.clone()),
                preview: c.content.chars().take(256).collect(),
            }
        })
        .collect();

    if total_matches > limit {
        budgets.record_usage(
            policy.content_saved_context_lookup.sources,
            "content_saved_context_lookup.max_sources",
            limit,
            total_matches,
            true,
        );
    }

    let total = results.len();
    let mut response = tool_result_value(
        &serde_json::json!({
            "query": query,
            "repo_root": repo_root,
            "cross_session": true,
            "agent_id": agent_id_filter,
            "merged_agent_view": merge_agent_partitions,
            "results": results,
            "total": total,
        }),
        output_format,
    )?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "content_saved_context_lookup.max_sources",
            limit,
            requested_limit.max(total_matches),
        ),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// get_global_memory  (CM11)
// ---------------------------------------------------------------------------

/// Return the cross-session global memory summary for this repo:
/// frequently-accessed symbols and files, and recurring workflow patterns.
///
/// Optionally supply `focus_symbols` and `focus_files` to also receive a list
/// of past sessions most relevant to the current work context.
pub fn tool_get_global_memory(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let requested_limit = args
        .and_then(|a| a.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;
    let limit = budgets.resolve_limit(
        policy.content_saved_context_lookup.sources,
        "content_saved_context_lookup.max_sources",
        Some(requested_limit),
    ) as u32;
    let focus_symbols: Vec<String> = args
        .and_then(|a| a.get("focus_symbols"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let focus_files: Vec<String> = args
        .and_then(|a| a.get("focus_files"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    let session_db = derive_session_db_path(db_path);
    let store = SessionStore::open(&session_db)?;

    let symbols = store.get_frequent_symbols(repo_root, limit)?;
    let files = store.get_frequent_files(repo_root, limit)?;
    let workflows = store.get_recurring_workflows(repo_root, limit)?;

    #[derive(Serialize)]
    struct AccessPreview {
        value: String,
        access_count: u64,
        last_accessed: String,
    }
    #[derive(Serialize)]
    struct WorkflowPreview {
        pattern: Vec<String>,
        occurrence_count: u64,
        last_seen: String,
    }
    #[derive(Serialize)]
    struct RelatedSession {
        session_id: String,
        repo_root: String,
        frontend: String,
        updated_at: String,
    }

    let frequent_symbols: Vec<AccessPreview> = symbols
        .into_iter()
        .map(|e: GlobalAccessEntry| AccessPreview {
            value: e.value,
            access_count: e.access_count,
            last_accessed: e.last_accessed,
        })
        .collect();
    let frequent_files: Vec<AccessPreview> = files
        .into_iter()
        .map(|e: GlobalAccessEntry| AccessPreview {
            value: e.value,
            access_count: e.access_count,
            last_accessed: e.last_accessed,
        })
        .collect();
    let workflow_patterns: Vec<WorkflowPreview> = workflows
        .into_iter()
        .map(|w: GlobalWorkflowPattern| WorkflowPreview {
            pattern: w.pattern,
            occurrence_count: w.occurrence_count,
            last_seen: w.last_seen,
        })
        .collect();

    let relevant_sessions: Vec<RelatedSession> =
        if !focus_symbols.is_empty() || !focus_files.is_empty() {
            store
                .find_relevant_sessions(repo_root, &focus_symbols, &focus_files, limit)?
                .into_iter()
                .map(|m: SessionMeta| RelatedSession {
                    session_id: m.session_id.as_str().to_string(),
                    repo_root: m.repo_root,
                    frontend: m.frontend,
                    updated_at: m.updated_at,
                })
                .collect()
        } else {
            Vec::new()
        };

    let observed = frequent_symbols
        .len()
        .max(frequent_files.len())
        .max(workflow_patterns.len())
        .max(relevant_sessions.len());
    if observed > limit as usize {
        budgets.record_usage(
            policy.content_saved_context_lookup.sources,
            "content_saved_context_lookup.max_sources",
            limit as usize,
            observed,
            true,
        );
    }

    let focus = (!focus_symbols.is_empty() || !focus_files.is_empty()).then(|| {
        serde_json::json!({
            "symbols": focus_symbols,
            "files": focus_files,
        })
    });

    let mut response = tool_result_value(
        &serde_json::json!({
            "tool": "get_global_memory",
            "repo_root": repo_root,
            "focus": focus,
            "frequent_symbols": frequent_symbols,
            "frequent_files": frequent_files,
            "workflow_patterns": workflow_patterns,
            "relevant_sessions": relevant_sessions,
            "summary": {
                "frequent_symbol_count": frequent_symbols.len(),
                "frequent_file_count": frequent_files.len(),
                "workflow_pattern_count": workflow_patterns.len(),
                "relevant_session_count": relevant_sessions.len(),
            },
            "warnings": [],
        }),
        output_format,
    )?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "content_saved_context_lookup.max_sources",
            limit as usize,
            requested_limit.max(observed),
        ),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
fn resolve_session_id(args: Option<&Value>, repo_root: &str) -> SessionId {
    if let Some(sid) = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
    {
        SessionId(sid.to_string())
    } else {
        mcp_session_id(repo_root)
    }
}

fn load_budget_policy(repo_root: &str) -> Result<BudgetPolicy> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    config.budget_policy()
}

fn load_redaction_rules(repo_root: &str) -> Result<RedactionRules> {
    let atlas_dir = atlas_engine::paths::atlas_dir(repo_root);
    let config = atlas_engine::Config::load(&atlas_dir).unwrap_or_default();
    let Some(path) = config.resolve_redaction_rules_file(&atlas_dir)? else {
        return Ok(RedactionRules::default());
    };
    load_redaction_rules_file(&path)
}

fn inject_budget_metadata(response: &mut Value, budget: &BudgetReport) {
    response["budget_status"] = serde_json::json!(budget.budget_status);
    response["budget_hit"] = serde_json::json!(budget.budget_hit);
    response["budget_name"] = serde_json::json!(&budget.budget_name);
    response["budget_limit"] = serde_json::json!(budget.budget_limit);
    response["budget_observed"] = serde_json::json!(budget.budget_observed);
    response["partial"] = serde_json::json!(budget.partial);
    response["safe_to_answer"] = serde_json::json!(budget.safe_to_answer);
}

/// Wrap structured output in MCP tool-result envelope.
pub(crate) fn tool_result_value<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<Value> {
    build_tool_result_value(value, output_format)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_db_path(dir: &TempDir) -> String {
        dir.path()
            .join(".atlas")
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned()
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

    fn open_session_store(db_path: &str) -> SessionStore {
        SessionStore::open(&derive_session_db_path(db_path)).unwrap()
    }

    fn seed_session_meta(store: &mut SessionStore, repo_root: &str) -> SessionId {
        let session_id = SessionId::derive(repo_root, "", "mcp");
        store
            .upsert_session_meta(session_id.clone(), repo_root, "mcp", None)
            .unwrap();
        session_id
    }

    fn append_session_event(
        store: &mut SessionStore,
        session_id: &SessionId,
        event_type: SessionEventType,
        payload: Value,
    ) {
        store
            .append_event(NewSessionEvent {
                session_id: session_id.clone(),
                event_type,
                priority: 1,
                payload,
                created_at: None,
            })
            .unwrap();
    }

    #[test]
    fn test_get_session_status_no_session() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path().to_str().unwrap();
        let db_path = setup_db_path(&dir);
        let result =
            tool_get_session_status(None, repo_root, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(body["status"], "no_session");
        assert_eq!(body["summary"]["status"], "no_session");
        assert_eq!(body["repo_root"], repo_root);
        assert_eq!(body["resume_snapshot_exists"], false);
        assert!(body["warnings"].as_array().is_some());
    }

    #[test]
    fn get_session_status_active_and_resumable_share_shape() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let repo_root = dir.path().to_str().unwrap();
        let db_path = setup_db_path(&dir);
        let mut store = open_session_store(&db_path);
        let session_id = seed_session_meta(&mut store, repo_root);
        append_session_event(
            &mut store,
            &session_id,
            SessionEventType::ContextRequest,
            serde_json::json!({"query": "compute"}),
        );
        store.build_resume(&session_id).unwrap();

        let result =
            tool_get_session_status(None, repo_root, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(body["session_id"], session_id.as_str());
        assert_eq!(body["status"], "active");
        assert_eq!(body["resume_snapshot_exists"], true);
        assert_eq!(body["summary"]["has_session"], true);
        assert!(body["event_count"].as_i64().unwrap() >= 1);
    }

    #[test]
    fn compact_session_no_op_returns_normalized_shape() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let repo_root = dir.path().to_str().unwrap();
        let db_path = setup_db_path(&dir);

        let result = tool_compact_session(None, repo_root, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(
            body["session_id"],
            SessionId::derive(repo_root, "", "mcp").as_str()
        );
        assert_eq!(body["summary"]["no_op"], true);
        assert_eq!(body["before_counts"]["events"], 0);
        assert_eq!(body["after_counts"]["events"], 0);
    }

    #[test]
    fn compact_session_effective_returns_normalized_shape() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let repo_root = dir.path().to_str().unwrap();
        let db_path = setup_db_path(&dir);
        let mut store = open_session_store(&db_path);
        let session_id = seed_session_meta(&mut store, repo_root);
        for run in 0..5 {
            append_session_event(
                &mut store,
                &session_id,
                SessionEventType::CommandRun,
                serde_json::json!({"command": "cargo build", "run": run}),
            );
        }
        drop(store);

        let result = tool_compact_session(None, repo_root, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(body["session_id"], session_id.as_str());
        assert!(body["merged_groups"].as_i64().unwrap() >= 1);
        assert!(
            body["removed_events"].as_i64().unwrap() >= 1
                || body["after_counts"]["events"].as_i64().unwrap()
                    < body["before_counts"]["events"].as_i64().unwrap()
        );
        assert_eq!(body["summary"]["status"], "ok");
    }

    #[test]
    fn resume_session_builds_snapshot_when_missing() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let repo_root = dir.path().to_str().unwrap();
        let db_path = setup_db_path(&dir);
        let mut store = open_session_store(&db_path);
        let session_id = seed_session_meta(&mut store, repo_root);
        append_session_event(
            &mut store,
            &session_id,
            SessionEventType::UserIntent,
            serde_json::json!({"intent": "review"}),
        );
        drop(store);

        let result = tool_resume_session(
            Some(&serde_json::json!({"mark_consumed": false})),
            repo_root,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let body = tool_body(&result);
        assert_eq!(body["session_id"], session_id.as_str());
        assert_eq!(body["snapshot_status"], "built_snapshot");
        assert_eq!(body["consumed"], false);
        assert!(body["snapshot"].is_object());
        assert!(body["event_count"].as_i64().unwrap() >= 1);
    }

    #[test]
    fn resume_session_reuses_existing_snapshot_and_marks_consumed() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let repo_root = dir.path().to_str().unwrap();
        let db_path = setup_db_path(&dir);
        let mut store = open_session_store(&db_path);
        let session_id = seed_session_meta(&mut store, repo_root);
        append_session_event(
            &mut store,
            &session_id,
            SessionEventType::Decision,
            serde_json::json!({"summary": "reuse context"}),
        );
        store.build_resume(&session_id).unwrap();
        drop(store);

        let result = tool_resume_session(None, repo_root, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert_eq!(body["snapshot_status"], "existing_snapshot");
        assert_eq!(body["consumed"], true);
        assert_eq!(body["summary"]["snapshot_consumed"], true);

        let store = open_session_store(&db_path);
        let snapshot = store.get_resume_snapshot(&session_id).unwrap().unwrap();
        assert!(snapshot.consumed);
    }

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
            tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
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
            tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
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
            tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
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
            tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
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
            tool_read_saved_context(Some(&read_args), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
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
            tool_search_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        let hit = &body["results"].as_array().unwrap()[0];
        assert!(hit["chunk_id"].as_str().is_some());
        assert_eq!(hit["identity_kind"].as_str().unwrap(), "artifact_label");
        assert_eq!(hit["identity_value"].as_str().unwrap(), "my label");
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
                "session_id": session_id.as_str(),
                "output_format": "json"
            })),
            repo_root,
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["query"], "src/lib.rs");
        assert_eq!(body["session_id"], session_id.as_str());
        assert_eq!(body["total"], 1);
        let hit = &body["results"][0];
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
            tool_search_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(result["budget_status"], "partial_result");
        assert_eq!(result["budget_hit"], true);
        assert_eq!(
            result["budget_name"],
            "content_saved_context_lookup.max_sources"
        );
        assert_eq!(result["budget_limit"], 25);
        assert_eq!(result["budget_observed"], 30);
        assert_eq!(body["total"], 25);
        assert_eq!(body["results"].as_array().unwrap().len(), 25);
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
        let client = crate::runtime_context::ReverseRequestClient::new(
            std::sync::Arc::new(|_, _, _| Ok(serde_json::json!({"action": "decline"}))),
            std::sync::Arc::new(|_| Ok(())),
            crate::runtime_context::ClientInteractionCapabilities {
                supports_elicitation_form: true,
                supports_elicitation_url: false,
            },
            "stdio",
            None,
            "1",
        );
        crate::runtime_context::install(client);
        let args = serde_json::json!({"keep_days": 30});
        let error = tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json)
            .unwrap_err();
        crate::runtime_context::uninstall();
        assert!(
            error
                .to_string()
                .contains("requires explicit elicited confirmation")
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

    #[test]
    fn test_continuity_event_spec_known_tools() {
        let args = serde_json::json!({"text": "find foo"});
        let spec = continuity_event_spec("query_graph", Some(&args));
        assert!(spec.is_some());
        let (et, payload) = spec.unwrap();
        assert_eq!(et, SessionEventType::ContextRequest);
        assert_eq!(payload["query"].as_str().unwrap(), "find foo");
    }

    #[test]
    fn test_continuity_event_spec_unknown_tool() {
        let spec = continuity_event_spec("list_graph_stats", None);
        assert!(spec.is_none());
    }

    // -----------------------------------------------------------------------
    // MCP13: read_saved_context tests
    // -----------------------------------------------------------------------

    /// Index a medium-sized artifact (above DEFAULT_SMALL_OUTPUT_BYTES) so it
    /// is actually stored and chunks are written, then read it back.
    fn save_indexed_artifact(
        repo_root: &str,
        db_path: &str,
        label: &str,
        content: &str,
        session_id: Option<&str>,
    ) -> String {
        let mut args = serde_json::json!({
            "content": content,
            "label": label,
        });
        if let Some(sid) = session_id {
            args["session_id"] = serde_json::json!(sid);
        }
        let result =
            tool_save_context_artifact(Some(&args), repo_root, db_path, OutputFormat::Json)
                .unwrap();
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        // Return the source_id regardless of routing (preview or pointer).
        body["source_id"].as_str().unwrap_or("").to_string()
    }

    /// Build a string longer than DEFAULT_SMALL_OUTPUT_BYTES (512 B) so
    /// `route_output` actually indexes it.
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

    fn oversized_content(paragraphs: usize) -> String {
        (0..paragraphs)
            .map(|i| {
                format!(
                    "paragraph {i} carries unique oversized artifact text with several safe words here\n\n"
                )
            })
            .collect()
    }

    fn medium_secret_content(secret_pair: &str, token: &str) -> String {
        let payload = std::iter::repeat_n("visible safe payload text", 50)
            .collect::<Vec<_>>()
            .join(" ");
        format!("{secret_pair} token={token} {payload}")
    }

    #[test]
    fn read_saved_context_missing_source_id_returns_error() {
        let dir = TempDir::new().unwrap();
        let db_path = setup_db_path(&dir);
        let err = tool_read_saved_context(
            Some(&serde_json::json!({})),
            "",
            &db_path,
            OutputFormat::Json,
        );
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("source_id"));
    }

    #[test]
    fn read_saved_context_unknown_source_id_returns_not_found() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        let args = serde_json::json!({"source_id": "does_not_exist"});
        let result =
            tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert!(!body["found"].as_bool().unwrap());
        assert_eq!(body["access_status"], "not_found");
        assert!(body["warnings"][0].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn read_saved_context_found_artifact_returns_metadata_and_content() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        let content = medium_content("artifact");
        let source_id = save_indexed_artifact(repo_root, &db_path, "my label", &content, None);
        assert!(!source_id.is_empty(), "artifact must be indexed (not raw)");

        let args = serde_json::json!({"source_id": source_id});
        let result =
            tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

        assert!(body["found"].as_bool().unwrap());
        assert_eq!(body["source_id"].as_str().unwrap(), source_id);
        assert_eq!(body["label"].as_str().unwrap(), "my label");
        assert_eq!(body["identity_kind"].as_str().unwrap(), "artifact_label");
        assert_eq!(body["identity_value"].as_str().unwrap(), "my label");
        assert!(body["created_at"].is_string());
        assert!(body["artifact_kind"].is_string());
        assert!(body["chunk_count"].as_u64().unwrap() > 0);
        assert!(body["byte_count"].as_u64().unwrap() > 0);
        assert!(body["returned_chunk_ids"].as_array().is_some());
        assert!(!body["content"].as_str().unwrap().is_empty());
        assert!(!body["truncated"].as_bool().unwrap());
    }

    #[test]
    fn read_saved_context_oversized_artifact_sets_truncated_and_continuation_hint() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        // Build content large enough to span multiple chunks and exceed a tiny cap.
        // Use unique paragraphs to avoid duplicate chunk_ids.
        let content: String = (0..200)
            .map(|i| format!("paragraph number {i} with some unique text here\n\n"))
            .collect();
        let source_id = save_indexed_artifact(repo_root, &db_path, "big artifact", &content, None);
        assert!(!source_id.is_empty(), "artifact must be indexed");

        // Request with a very tight byte cap so the first chunk alone exceeds it.
        let args = serde_json::json!({"source_id": source_id, "max_bytes": 1});
        let result =
            tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

        assert!(body["found"].as_bool().unwrap());
        assert!(body["truncated"].as_bool().unwrap());
        assert!(body["next_chunk_offset"].is_number());
        assert!(body["next_chunk_id"].as_str().is_some());
        assert!(
            body["continuation_hint"]
                .as_str()
                .unwrap()
                .contains("chunk_offset")
        );
    }

    #[test]
    fn read_saved_context_max_bytes_is_clamped_by_central_budget_policy() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        let content: String = (0..200)
            .map(|i| {
                format!(
                    "chunk paragraph {i} with unique payload data {}\n\n",
                    "x".repeat(256)
                )
            })
            .collect();
        let source_id = save_indexed_artifact(repo_root, &db_path, "very large", &content, None);
        assert!(!source_id.is_empty(), "artifact must be indexed");

        let args = serde_json::json!({"source_id": source_id, "max_bytes": 999999});
        let result =
            tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
        let body: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(result["budget_status"], "partial_result");
        assert_eq!(result["budget_hit"], true);
        assert_eq!(
            result["budget_name"],
            "mcp_cli_payload_serialization.max_saved_context_bytes"
        );
        assert_eq!(result["budget_limit"], 32768);
        assert!(result["budget_observed"].as_u64().unwrap() > 32_768);
        assert_eq!(body["found"], true);
        assert!(
            body["truncated"].as_bool().unwrap()
                || body["content"].as_str().unwrap().len() <= 32_768
        );
    }

    #[test]
    fn read_saved_context_paging_chunk_offset_skips_earlier_chunks() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        // Use unique paragraphs to avoid duplicate chunk_ids.
        let content: String = (0..100)
            .map(|i| format!("unique paragraph {i} here\n\n"))
            .collect();
        let source_id = save_indexed_artifact(repo_root, &db_path, "paged", &content, None);
        assert!(!source_id.is_empty());

        // First call with cap that forces truncation.
        let args = serde_json::json!({"source_id": source_id, "max_bytes": 100});
        let r1 =
            tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
        let b1: Value = serde_json::from_str(r1["content"][0]["text"].as_str().unwrap()).unwrap();

        let next = b1.get("next_chunk_offset").and_then(|v| v.as_u64());
        if let Some(offset) = next {
            let args2 = serde_json::json!({"source_id": source_id, "chunk_offset": offset});
            let r2 = tool_read_saved_context(Some(&args2), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
            let b2: Value =
                serde_json::from_str(r2["content"][0]["text"].as_str().unwrap()).unwrap();
            assert!(b2["found"].as_bool().unwrap());
            assert_eq!(b2["chunk_offset"].as_u64().unwrap(), offset);
            assert!(b2["returned_chunk_ids"].as_array().is_some());
        }
        // If not truncated the content was small enough in one page — test still passes.
    }

    #[test]
    fn read_saved_context_cross_session_isolation_blocks_read() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        let content = medium_content("secret");
        let source_id =
            save_indexed_artifact(repo_root, &db_path, "private", &content, Some("session-A"));
        assert!(!source_id.is_empty());

        // Attempt to read with a different session_id.
        let args = serde_json::json!({"source_id": source_id, "session_id": "session-B"});
        let result =
            tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
        let body = tool_body(&result);
        assert!(!body["found"].as_bool().unwrap());
        assert_eq!(body["access_status"], "session_mismatch");
        assert!(
            body["warnings"][0]
                .as_str()
                .unwrap()
                .contains("not accessible from this session")
        );
    }

    #[test]
    fn read_saved_context_cross_repo_isolation_blocks_read() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        let content = medium_content("cross-repo");
        let source_id = save_indexed_artifact(repo_root, &db_path, "repo-bound", &content, None);
        assert!(!source_id.is_empty());

        let args = serde_json::json!({"source_id": source_id});
        let result =
            tool_read_saved_context(Some(&args), "/different/repo", &db_path, OutputFormat::Json)
                .unwrap();
        let body = tool_body(&result);
        assert!(!body["found"].as_bool().unwrap());
        assert_eq!(body["access_status"], "repo_mismatch");
        assert!(
            body["warnings"][0]
                .as_str()
                .unwrap()
                .contains("not accessible from this repository")
        );
    }

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
}
