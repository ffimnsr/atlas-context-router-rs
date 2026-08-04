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
    let text = args
        .and_then(|a| a.get("text"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: text"))?;
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
        metadata: serde_json::json!({}),
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
        Some(store) => store.recall_memories(repo_root, query, &filter, shared, &viewer, limit)?,
        None => Vec::new(),
    };

    let results = hits
        .iter()
        .map(|hit| {
            serde_json::json!({
                "memory": hit.memory,
                "relevance_score": hit.relevance_score,
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
