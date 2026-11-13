//! `list_graph_stats` payload building.
//!
//! Kept out of `graph.rs` so the transport surface stays readable. Every count
//! comes from `Store::stats()` — the same source used by `status`, `doctor`,
//! `debug_graph`, and the CLI — so totals never drift between tools.

use anyhow::{Context, Result};
use serde_json::json;

use super::shared::{open_store, tool_result_value};

pub(super) fn tool_list_graph_stats(
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let store = open_store(db_path)?;
    let stats = store.graph_stats().context("stats query failed")?;
    let language_counts: std::collections::BTreeMap<String, i64> = store
        .language_node_counts()
        .context("language node counts failed")?
        .into_iter()
        .collect();
    let edge_kinds: std::collections::BTreeMap<String, i64> = store
        .edge_kind_stats()
        .context("edge kind stats failed")?
        .into_iter()
        .collect();

    let payload = json!({
        "file_count": stats.file_count,
        "node_count": stats.node_count,
        "edge_count": stats.edge_count,
        "nodes_by_kind": stats.nodes_by_kind,
        // `languages` maps language name to its node count so consumers can
        // assert `languages.rust.nodes`; `language_node_counts` is the flat
        // additive form for simple lookups.
        "languages": language_counts
            .iter()
            .map(|(language, count)| (language.clone(), json!({ "nodes": count })))
            .collect::<serde_json::Map<_, _>>(),
        "language_node_counts": language_counts,
        "edge_kinds": edge_kinds,
        "last_indexed_at": stats.last_indexed_at,
    });
    tool_result_value(&payload, output_format)
}
