//! Saved-context search/store tools and decision-memory helpers:
//! `search_saved_context`, `search_decisions`, `save_context_artifact`,
//! `get_context_stats`, plus the best-effort decision search/record helpers.

use anyhow::Result;
use atlas_adapters::bridge::bridge_file_count;
use atlas_adapters::{
    ArtifactIdentity, RedactionRules, derive_bridge_dir, derive_content_db_path,
    derive_session_db_path, extract_decision_event_with_details, generate_source_id,
    load_redaction_rules_file, redact_text_with_rules,
};
use atlas_contentstore::{ContentStore, OutputRouting, SearchFilters, SourceMeta};
use atlas_core::{BudgetManager, BudgetReport};
use atlas_session::{DecisionSearchHit, SessionStore};
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

use crate::output::OutputFormat;
use crate::tool_result::{
    normalized_tool_result_value as build_normalized_tool_result_value, tool_execution_error_value,
};
use crate::tools::shared::inject_deprecated_input_fields;

use super::{
    inject_budget_metadata, load_budget_policy, mcp_session_id, normalize_repo_roots,
    open_session_store_best_effort, resolve_requested_repo_roots, resolve_session_id,
    tool_result_value,
};

fn artifact_repo_roots(meta: &SourceMeta) -> Vec<String> {
    normalize_repo_roots(meta.repo_roots.clone())
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
    let (repo_scope_roots, deprecated_input_fields) = if cross_session {
        match resolve_requested_repo_roots("search_saved_context", args, repo_root) {
            Ok(resolved) => resolved,
            Err(payload) => return tool_execution_error_value(output_format, &payload),
        }
    } else {
        (Vec::new(), Vec::new())
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
        repo_root: None,
        repo_roots: repo_scope_roots.clone(),
    };

    let chunks = cs.search_with_fallback(query, &filters)?;

    #[derive(Serialize)]
    struct ChunkPreview {
        source_id: String,
        chunk_id: String,
        chunk_index: usize,
        repo_roots: Vec<String>,
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
                repo_roots: source
                    .as_ref()
                    .map(|row| row.repo_roots.clone())
                    .unwrap_or_default(),
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
    let truncated = total_matches > limit;
    let mut response = build_normalized_tool_result_value(
        &serde_json::json!({
            "tool": "search_saved_context",
            "query": {
                "text": query,
                "session_id": session_id_filter,
                "agent_id": agent_id_filter,
                "cross_session": cross_session,
                "merge_agent_partitions": merge_agent_partitions || cross_session,
                "source_type": filters.source_type,
                "requested_limit": requested_limit,
                "applied_limit": limit,
                "repo_scope": {
                    "repo_roots": repo_scope_roots,
                    "repo_count": repo_scope_roots.len(),
                }
            },
            "matches": results,
            "linked_decisions": linked_decisions,
            "summary": {
                "match_count": total,
                "total_matches": total_matches,
                "linked_decision_count": linked_decisions.len(),
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
    build_normalized_tool_result_value(
        &serde_json::json!({
            "tool": "search_decisions",
            "query": {
                "text": query,
                "session_id": session_id,
                "agent_id": agent_id,
                "requested_limit": limit,
            },
            "matches": hits,
            "summary": {
                "match_count": hits.len(),
                "total_matches": hits.len(),
            },
            "truncated": false,
            "warnings": [],
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
    let (repo_roots, deprecated_input_fields) =
        match resolve_requested_repo_roots("save_context_artifact", args, repo_root) {
            Ok(resolved) => resolved,
            Err(payload) => return tool_execution_error_value(output_format, &payload),
        };
    let primary_repo_root = (repo_roots.len() == 1).then(|| repo_roots[0].clone());

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
        repo_root: primary_repo_root,
        repo_roots: repo_roots.clone(),
        repo_id: None,
        repo_ids: vec![],
        identity_kind: identity.kind_str().to_owned(),
        identity_value: identity.value().to_owned(),
    };
    let artifact_repo_scope = artifact_repo_roots(&meta);

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
        "repo_scope": {
            "repo_roots": repo_roots,
            "repo_count": artifact_repo_scope.len(),
        },
        "summary": {
            "session_id": session_id_str,
            "stored": storage_mode != "raw_inline",
            "inline": storage_mode == "raw_inline",
            "content_type": content_type,
        }
    });

    let mut response = tool_result_value(&result, output_format)?;
    inject_deprecated_input_fields(&mut response, &deprecated_input_fields);
    Ok(response)
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

fn load_redaction_rules(repo_root: &str) -> Result<RedactionRules> {
    let atlas_dir = atlas_engine::paths::atlas_dir(repo_root);
    let config = atlas_engine::Config::load(&atlas_dir).unwrap_or_default();
    let Some(path) = config.resolve_redaction_rules_file(&atlas_dir)? else {
        return Ok(RedactionRules::default());
    };
    load_redaction_rules_file(&path)
}
