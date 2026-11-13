//! Memory tools: `get_global_memory`, `memory_store`, and `memory_recall`.

use anyhow::Result;
use atlas_adapters::derive_session_db_path;
use atlas_core::BudgetManager;
use atlas_session::{
    GlobalAccessEntry, GlobalWorkflowPattern, MemoryImportance, MemoryListFilter, MemoryScope,
    MemoryViewer, NewMemory, SessionMeta, SessionStore,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

use crate::output::OutputFormat;
use crate::tool_result::normalized_tool_result_value as build_normalized_tool_result_value;

use super::{
    inject_budget_metadata, load_budget_policy, mcp_session_id, open_session_store_best_effort,
    tool_result_value,
};

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
// memory_store / memory_recall  (ICM-A shared memory surface)
// ---------------------------------------------------------------------------

/// Store a memory record through the shared memory service layer. Field names,
/// defaults, and validation are identical to `atlas memory store`.
pub fn tool_memory_store(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let text = args.and_then(|a| a.get("text")).and_then(|v| v.as_str());
    let patch_id = args
        .and_then(|a| a.get("patch_id"))
        .and_then(|v| v.as_str());
    let patch_text = args
        .and_then(|a| a.get("patch_text"))
        .and_then(|v| v.as_str());
    let patch_file = args
        .and_then(|a| a.get("patch_file"))
        .and_then(|v| v.as_str());

    if let Some(memory_id) = patch_id {
        if text.is_some() {
            return Err(anyhow::anyhow!(
                "memory_store accepts either text or patch_id, not both"
            ));
        }
        let patch = read_patch_text(patch_text, patch_file)?;
        let session_db = derive_session_db_path(db_path);
        let mut store = SessionStore::open(&session_db)?;
        let outcomes = store.apply_memory_patch(repo_root, memory_id, &patch, None)?;
        let record = store.get_memory(repo_root, memory_id)?;
        return build_normalized_tool_result_value(
            &serde_json::json!({
                "tool": "memory_store",
                "repo_root": repo_root,
                "memory": record,
                "patched": true,
                "patch_count": outcomes.len(),
                "patches": outcomes,
                "summary": {
                    "memory_id": memory_id,
                    "patched": true,
                    "patch_count": outcomes.len(),
                },
                "warnings": [],
            }),
            output_format,
        );
    }

    let text = text.ok_or_else(|| {
        anyhow::anyhow!("missing required argument: text or patch_id with patch_text/patch_file")
    })?;
    let topic = args.and_then(|a| a.get("topic")).and_then(|v| v.as_str());
    let title = args.and_then(|a| a.get("title")).and_then(|v| v.as_str());
    let importance = args
        .and_then(|a| a.get("importance"))
        .and_then(|v| v.as_str());
    let scope = args.and_then(|a| a.get("scope")).and_then(|v| v.as_str());
    let frontend = args
        .and_then(|a| a.get("frontend"))
        .and_then(|v| v.as_str());
    let source_id = args
        .and_then(|a| a.get("source_id"))
        .and_then(|v| v.as_str());

    // Same boundary validation as the CLI: strict enum parsing and
    // config-gated frontend normalization from the shared layer.
    let importance = match importance {
        Some(raw) => raw.parse().map_err(anyhow::Error::from)?,
        None => MemoryImportance::default(),
    };
    let scope = match scope {
        Some(raw) => raw.parse().map_err(anyhow::Error::from)?,
        None => MemoryScope::default(),
    };
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let frontend = frontend
        .map(|raw| atlas_session::normalize_frontend(raw, config.allow_custom_frontends()))
        .transpose()?;
    let session_id =
        (scope == MemoryScope::Session).then(|| mcp_session_id(repo_root).as_str().to_owned());
    // `patch_file` only participates in the EUA patch path. For a plain store it
    // is a related-file pointer, so persist it instead of silently dropping it.
    let metadata = match patch_file {
        Some(path) => serde_json::json!({ "related_file": path }),
        None => serde_json::json!({}),
    };
    let input = NewMemory {
        repo_root: repo_root.to_owned(),
        session_id,
        frontend,
        scope,
        topic: topic.unwrap_or_default().to_owned(),
        title: title.unwrap_or_default().to_owned(),
        body: text.to_owned(),
        importance,
        source_id: source_id.map(str::to_owned),
        metadata,
    };
    input.validate()?;

    let session_db = derive_session_db_path(db_path);
    let mut store = SessionStore::open(&session_db)?;
    let record = store.store_memory(&input)?;

    build_normalized_tool_result_value(
        &serde_json::json!({
            "tool": "memory_store",
            "repo_root": repo_root,
            "memory": record,
            "summary": {
                "memory_id": record.id,
                "scope": record.scope.as_str(),
                "importance": record.importance.as_str(),
            },
            "warnings": [],
        }),
        output_format,
    )
}

/// Recall memories through the shared memory service layer with the same
/// visibility rules and defaults as `atlas memory recall`. The MCP viewer is
/// the derived `mcp` frontend session; `shared` narrows to project + global.
pub fn tool_memory_recall(
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
    let topic = args.and_then(|a| a.get("topic")).and_then(|v| v.as_str());
    let importance = args
        .and_then(|a| a.get("importance"))
        .and_then(|v| v.as_str());
    let scope = args.and_then(|a| a.get("scope")).and_then(|v| v.as_str());
    let shared = args
        .and_then(|a| a.get("shared"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let hybrid = args
        .and_then(|a| a.get("hybrid"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let requested_limit = args
        .and_then(|a| a.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    let limit = budgets.resolve_limit(
        policy.mcp_cli_payload_serialization.nodes,
        "mcp_cli_payload_serialization.max_nodes",
        Some(requested_limit),
    );

    let filter = MemoryListFilter {
        topic: topic.map(str::to_owned),
        importance: importance
            .map(|raw| raw.parse().map_err(anyhow::Error::from))
            .transpose()?,
        scope: scope
            .map(|raw| raw.parse().map_err(anyhow::Error::from))
            .transpose()?,
        ..Default::default()
    };
    let viewer = MemoryViewer {
        frontend: "mcp".to_owned(),
        session_id: mcp_session_id(repo_root).as_str().to_owned(),
    };

    let hits = match open_session_store_best_effort(db_path) {
        Some(store) => {
            let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
                .unwrap_or_default();
            let recall_policy = atlas_session::MemoryRecallPolicy {
                freshness_window_days: config.memory.recall.freshness_window_days,
                weight_time: config.memory.recall.weight_time,
                weight_business: config.memory.recall.weight_business,
                primary_quota_ratio: config.memory.recall.primary_quota_ratio,
                aux_max_hops: config.memory.recall.aux_max_hops,
            };
            if hybrid {
                let query_vector = memory_query_embedding(&config, query)?;
                let embedding_policy = atlas_session::MemoryEmbeddingPolicy {
                    weight_fts: config.memory.search.hybrid_weight_fts,
                    weight_vector: config.memory.search.hybrid_weight_vector,
                    rrf_k: atlas_session::MEMORY_HYBRID_RRF_K,
                    min_similarity: config.memory.search.min_vector_similarity,
                };
                store.recall_memories_hybrid_with_policy(
                    repo_root,
                    query,
                    &query_vector,
                    &filter,
                    shared,
                    &viewer,
                    limit,
                    &recall_policy,
                    &embedding_policy,
                )?
            } else {
                store.recall_memories_with_policy(
                    repo_root,
                    query,
                    &filter,
                    shared,
                    &viewer,
                    limit,
                    &recall_policy,
                )?
            }
        }
        None => Vec::new(),
    };

    let results = hits
        .iter()
        .map(|hit| {
            serde_json::json!({
                "memory": hit.memory,
                "relevance_score": hit.relevance_score,
                "recall_score": hit.recall_score,
                "origin_score": hit.ranking.origin_score,
                "time_score": hit.ranking.time_score,
                "importance_score": hit.ranking.importance_score,
                "recall_path": hit.ranking.recall_path,
                "ranking": hit.ranking,
            })
        })
        .collect::<Vec<_>>();

    // Compact retrieval hints: distinct topics, scopes, and source ids so
    // follow-up recall can be targeted without re-reading bodies.
    let mut topics = BTreeSet::new();
    let mut scopes = BTreeSet::new();
    let mut source_ids = BTreeSet::new();
    for hit in &hits {
        if !hit.memory.topic.is_empty() {
            topics.insert(hit.memory.topic.clone());
        }
        scopes.insert(hit.memory.scope.as_str().to_owned());
        if let Some(source_id) = &hit.memory.source_id {
            source_ids.insert(source_id.clone());
        }
    }
    let mut retrieval_hints = Vec::new();
    for topic in topics {
        retrieval_hints.push(serde_json::json!({ "kind": "topic", "value": topic }));
    }
    for scope in scopes {
        retrieval_hints.push(serde_json::json!({ "kind": "scope", "value": scope }));
    }
    for source_id in source_ids {
        retrieval_hints.push(serde_json::json!({ "kind": "source_id", "value": source_id }));
    }

    let observed = hits.len();
    if observed >= limit {
        budgets.record_usage(
            policy.mcp_cli_payload_serialization.nodes,
            "mcp_cli_payload_serialization.max_nodes",
            limit,
            observed,
            true,
        );
    }

    let mut response = tool_result_value(
        &serde_json::json!({
            "tool": "memory_recall",
            "repo_root": repo_root,
            "query": {
                "text": query,
                "topic": topic,
                "importance": importance,
                "scope": scope,
                "shared": shared,
                "hybrid": hybrid,
                "requested_limit": requested_limit,
                "applied_limit": limit,
            },
            "results": results,
            "retrieval_hints": retrieval_hints,
            "summary": {
                "match_count": hits.len(),
                "total_matches": hits.len(),
                "retrieval_hint_count": retrieval_hints.len(),
            },
            "truncated": observed >= limit,
            "warnings": [],
        }),
        output_format,
    )?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "mcp_cli_payload_serialization.max_nodes",
            limit,
            requested_limit.max(observed),
        ),
    );
    Ok(response)
}

/// Resolve the opt-in memory embedding backend and embed one recall query.
///
/// Mirrors the CLI rule: `--hybrid` behavior fails clearly when embeddings are
/// disabled or when the configured dimension disagrees with the provider.
fn memory_query_embedding(config: &atlas_engine::Config, query: &str) -> Result<Vec<f32>> {
    let Some(backend) = config.memory_embedding_backend()? else {
        anyhow::bail!(
            "memory.embedding.enabled is false; enable [memory.embedding] with provider = \"search\" to use hybrid recall"
        );
    };
    let dimension = config.memory.embedding.dimension;
    let client = atlas_search::embed::EmbeddingConfig::new(
        backend.url.clone(),
        backend.model.clone(),
        backend.timeout_secs,
        backend.max_retries,
        backend.retry_backoff_ms,
    );
    let vector = atlas_search::embed::embed_text_blocking(&client, query)
        .map_err(|error| anyhow::anyhow!("cannot embed recall query: {error}"))?;
    if vector.len() != dimension {
        anyhow::bail!(
            "memory.embedding.dimension is {dimension} but the provider returned {} values",
            vector.len()
        );
    }
    Ok(vector)
}

/// Read an EUA patch body from inline text or a file (1 MiB ceiling).
fn read_patch_text(inline: Option<&str>, file: Option<&str>) -> Result<String> {
    if let Some(text) = inline {
        return Ok(text.to_owned());
    }
    let Some(path) = file else {
        return Err(anyhow::anyhow!(
            "patch_id requires patch_text or patch_file"
        ));
    };
    const MAX_PATCH_BYTES: u64 = 1024 * 1024;
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_PATCH_BYTES {
        return Err(anyhow::anyhow!("patch file exceeds 1 MiB: {path}"));
    }
    Ok(std::fs::read_to_string(path)?)
}

// ---------------------------------------------------------------------------
// memory_ops  (VM1 typed operator history)
// ---------------------------------------------------------------------------

/// Return the bounded typed-operator history for one memory row. This is a
/// read-only parity surface for `atlas memory ops`; an unknown id returns
/// `found = false` instead of an error so agents can probe safely.
pub fn tool_memory_ops(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let memory_id = args
        .and_then(|a| a.get("memory_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: memory_id"))?;

    let session_db = derive_session_db_path(db_path);
    let store = SessionStore::open(&session_db)?;
    let history = store.memory_op_history(repo_root, memory_id)?;

    let (found, numeric_value, entries, warnings) = match history {
        Some(history) => (true, history.numeric_value, history.entries, Vec::new()),
        None => (
            false,
            None,
            Vec::new(),
            vec![format!("no memory with id {memory_id} in {repo_root}")],
        ),
    };
    let entry_count = entries.len();

    build_normalized_tool_result_value(
        &serde_json::json!({
            "tool": "memory_ops",
            "repo_root": repo_root,
            "memory_id": memory_id,
            "found": found,
            "numeric_value": numeric_value,
            "op_history": entries,
            "summary": {
                "entry_count": entry_count,
            },
            "warnings": warnings,
        }),
        output_format,
    )
}
