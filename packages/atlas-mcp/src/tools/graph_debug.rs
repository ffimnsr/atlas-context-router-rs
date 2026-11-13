//! MCP tool: `debug_graph` — graph internals for structural anomaly triage.
//!
//! Extracted from `tools::health` so the health module stays below the
//! repository file-size threshold. Payload keys match `atlas debug-graph
//! --json` and the registry output schema (`nodes_by_kind` / `edges_by_kind`).

use anyhow::{Context, Result};
use atlas_core::BudgetManager;
use serde_json::json;

use crate::tool_result::{ToolSuccessEnvelope, normalized_tool_result_value};

use super::shared::{inject_budget_metadata, load_budget_policy, open_store, u64_arg};

pub(super) fn tool_debug_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    const DEFAULT_LIMIT: usize = 20;
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let requested_limit = u64_arg(args, "limit").unwrap_or(DEFAULT_LIMIT as u64) as usize;
    let limit = budgets.resolve_limit(
        policy.mcp_cli_payload_serialization.nodes,
        "mcp_cli_payload_serialization.max_nodes",
        Some(requested_limit),
    );

    let store = open_store(db_path)?;
    let stats = store.graph_stats().context("cannot read graph stats")?;
    let edge_kinds = store.edge_kind_stats().context("edge kind stats failed")?;
    let top_files = store
        .top_files_by_node_count(limit)
        .context("top files query failed")?;
    let orphans = store
        .orphan_nodes(limit)
        .context("orphan node query failed")?;
    let dangling = store
        .structural_dangling_edges(limit)
        .context("dangling edge query failed")?;

    let orphan_nodes = orphans
        .iter()
        .map(|n| {
            json!({
                "kind": n.kind.as_str(),
                "qualified_name": &n.qualified_name,
                "file_path": &n.file_path,
                "line_start": n.line_start,
            })
        })
        .collect::<Vec<_>>();

    let dangling_edges = dangling
        .iter()
        .map(|(id, src, tgt, kind, side)| {
            json!({
                "id": id,
                "kind": kind,
                "source_qn": src,
                "target_qn": tgt,
                "missing_side": side,
            })
        })
        .collect::<Vec<_>>();

    let top_file_rows = top_files
        .iter()
        .map(|(path, count)| {
            json!({
                "file_path": path,
                "path": path,
                "node_count": count,
            })
        })
        .collect::<Vec<_>>();

    let payload = json!({
        "nodes_by_kind": stats.nodes_by_kind,
        "edges_by_kind": edge_kinds,
        "top_files": top_file_rows,
        "orphan_nodes": orphan_nodes,
        "dangling_edges": dangling_edges,
        "summary": {
            "node_count": stats.node_count,
            "edge_count": stats.edge_count,
            "file_count": stats.file_count,
            "top_file_count": top_files.len(),
            "orphan_node_count": orphans.len(),
            "dangling_edge_count": dangling.len(),
        },
        "warnings": Vec::<String>::new(),
    });

    let observed = top_files.len().max(orphans.len()).max(dangling.len());
    if orphans.len() >= limit || dangling.len() >= limit {
        budgets.record_usage(
            policy.mcp_cli_payload_serialization.nodes,
            "mcp_cli_payload_serialization.max_nodes",
            limit,
            observed,
            true,
        );
    }

    let envelope = ToolSuccessEnvelope::new("debug_graph", payload);
    let mut response = normalized_tool_result_value(&envelope, output_format)?;
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
