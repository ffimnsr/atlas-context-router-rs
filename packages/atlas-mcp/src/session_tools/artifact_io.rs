//! Saved-context read/purge tools: `read_saved_context`,
//! `purge_saved_context`, and `cross_session_search`.

use anyhow::Result;
use atlas_adapters::bridge::purge_all_bridge_files;
use atlas_adapters::{derive_bridge_dir, derive_content_db_path};
use atlas_contentstore::{ContentStore, SearchFilters};
use atlas_core::{BudgetManager, BudgetReport};
use serde::Serialize;
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::{
    normalized_tool_result_value as build_normalized_tool_result_value, tool_execution_error_value,
};
use crate::tools::shared::inject_deprecated_input_fields;

use super::{
    inject_budget_metadata, load_budget_policy, normalize_repo_roots, resolve_requested_repo_roots,
    tool_result_value,
};

fn repo_scopes_overlap(left: &[String], right: &[String]) -> bool {
    left.iter()
        .any(|candidate| right.iter().any(|other| other == candidate))
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
    let (requested_repo_roots, deprecated_input_fields) =
        match resolve_requested_repo_roots("read_saved_context", args, repo_root) {
            Ok(resolved) => resolved,
            Err(payload) => return tool_execution_error_value(output_format, &payload),
        };

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
            inject_deprecated_input_fields(&mut response, &deprecated_input_fields);
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
        inject_deprecated_input_fields(&mut response, &deprecated_input_fields);
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
        inject_deprecated_input_fields(&mut response, &deprecated_input_fields);
        return Ok(response);
    }

    let artifact_repo_roots = normalize_repo_roots(source.repo_roots.clone());
    if !repo_scopes_overlap(&requested_repo_roots, &artifact_repo_roots) {
        let mut response = tool_result_value(
            &build_error_result(
                "repo_scope_mismatch",
                "artifact repo scope does not overlap current request scope",
            ),
            output_format,
        )?;
        inject_budget_metadata(&mut response, &summary_budget(max_bytes));
        inject_deprecated_input_fields(&mut response, &deprecated_input_fields);
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
        "repo_scope": {
            "repo_roots": artifact_repo_roots,
            "repo_count": artifact_repo_roots.len(),
            "requested_repo_roots": requested_repo_roots,
        },
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
    inject_deprecated_input_fields(&mut response, &deprecated_input_fields);
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
        if crate::runtime_context::current().is_ok() {
            match crate::elicitation::confirm_age_based_purge()? {
                crate::elicitation::ConfirmationProgress::Confirmed => {}
                crate::elicitation::ConfirmationProgress::Cancelled => {
                    return Err(anyhow::anyhow!("purge_saved_context cancelled by client"));
                }
                crate::elicitation::ConfirmationProgress::InputRequired(input_required) => {
                    let mut response = crate::mrtr::build_input_required_tool_result(
                        &input_required,
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
                    return Ok(response);
                }
            }
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
    let (repo_scope_roots, deprecated_input_fields) =
        match resolve_requested_repo_roots("cross_session_search", args, repo_root) {
            Ok(resolved) => resolved,
            Err(payload) => return tool_execution_error_value(output_format, &payload),
        };

    // Explicitly filter by repo scope, no session_id restriction.
    let filters = SearchFilters {
        session_id: None,
        agent_id: if merge_agent_partitions {
            None
        } else {
            agent_id_filter.clone()
        },
        source_type: source_type_filter,
        repo_root: None,
        repo_roots: repo_scope_roots.clone(),
    };

    let chunks = cs.search_with_fallback(query, &filters)?;

    #[derive(Serialize)]
    struct CrossSessionResult {
        source_id: String,
        chunk_id: String,
        chunk_index: usize,
        repo_roots: Vec<String>,
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
                repo_roots: source
                    .as_ref()
                    .map(|s| s.repo_roots.clone())
                    .unwrap_or_default(),
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
    let truncated = total_matches > limit;
    let sessions = results
        .iter()
        .filter_map(|item| {
            item.session_id
                .as_ref()
                .map(|session_id| (session_id.clone(), item.agent_id.clone()))
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|(session_id, agent_id)| {
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
            })
        })
        .collect::<Vec<_>>();
    let mut response = build_normalized_tool_result_value(
        &serde_json::json!({
            "tool": "cross_session_search",
            "query": {
                "text": query,
                "repo_root": repo_root,
                "cross_session": true,
                "agent_id": agent_id_filter,
                "merge_agent_partitions": merge_agent_partitions,
                "source_type": filters.source_type,
                "requested_limit": requested_limit,
                "applied_limit": limit,
                "repo_scope": {
                    "repo_roots": repo_scope_roots,
                    "repo_count": repo_scope_roots.len(),
                }
            },
            "sessions": sessions,
            "matches": results,
            "summary": {
                "match_count": total,
                "total_matches": total_matches,
                "session_count": sessions.len(),
            },
            "truncated": truncated,
            "warnings": [],
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
    inject_deprecated_input_fields(&mut response, &deprecated_input_fields);
    Ok(response)
}
