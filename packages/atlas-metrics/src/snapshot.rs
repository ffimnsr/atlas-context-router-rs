//! Serializable point-in-time metric snapshot.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::histogram::HistogramSnapshot;

/// Per-tool MCP call outcome counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolCallCounts {
    pub ok: u64,
    pub error: u64,
}

/// Serializable view of all Atlas runtime metrics recorded in this process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Build/update attempts by kind (`build` or `update`).
    pub build_runs: BTreeMap<String, u64>,
    /// Failed build/update attempts by kind.
    pub build_failures: BTreeMap<String, u64>,
    /// End-to-end build/update duration in milliseconds by kind.
    pub build_duration_ms: BTreeMap<String, HistogramSnapshot>,
    /// Files parsed per successful build/update run by kind.
    pub build_parsed_files: BTreeMap<String, HistogramSnapshot>,
    /// Total `ParserRegistry::parse` calls.
    pub parser_parses_total: u64,
    /// Parse calls that supplied a cached old tree to tree-sitter.
    pub parser_tree_reuses_total: u64,
    /// `parser_tree_reuses_total / parser_parses_total` (0.0 when no parses).
    pub parser_cache_reuse_ratio: f64,
    /// Query executions by execution mode.
    pub query_calls: BTreeMap<String, u64>,
    /// Query latency in milliseconds by execution mode.
    pub query_duration_ms: BTreeMap<String, HistogramSnapshot>,
    /// MCP tool calls by tool name and outcome.
    pub mcp_tool_calls: BTreeMap<String, McpToolCallCounts>,
    /// MCP tool call duration in milliseconds by tool name.
    pub mcp_tool_duration_ms: BTreeMap<String, HistogramSnapshot>,
}
