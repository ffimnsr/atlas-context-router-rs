//! Base `tools/list` registry entries for the `health` tool family.
//!
//! Assembled by `super::base_tool_list_json()`; entry order is
//! irrelevant because descriptors are sorted by name afterwards.

use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::{Value, json};

/// Base registry entry JSON for the health tools.
pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
                "name": "list_graph_stats",
                "description": "Return node/edge counts and language breakdown for the indexed graph.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "repo_registry",
                "description": "Inspect persisted multi-repo registry metadata under .atlas/, including registered repo identities, relationship kinds, trust states, and bootstrap warnings.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "broker_status",
                "description": "Return a lightweight health/ready check for the MCP broker process itself. Reports process uptime, PID, server version, and configured worker threads. Does NOT check graph readiness — use `status` or `doctor` for graph health. Useful for liveness probes and connectivity verification independent of graph state.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "status",
                "description": "Return a compact graph health summary: build state, node/edge counts, last-indexed timestamp, and a machine-readable failure category. Call this before query_graph or get_context to verify the graph is healthy and up-to-date. Succeeds even when the graph DB is missing.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "doctor",
                "description": "Run full repo health checks: git root detection, .atlas dir, config file, DB open/integrity, graph build state, tracked file count, and retrieval-index state. Returns an array of per-check results with pass/fail and detail. Call before trusting graph-backed context after a fresh clone or suspected corruption.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "db_check",
                "description": "Run SQLite integrity check, scan for orphan nodes (no edges) and dangling edges (missing endpoint), and validate the session-side memory schema. Returns ok=true when all checks pass. Use to diagnose corrupt or inconsistent graph rows or a missing memories table.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit":         { "type": "integer", "description": "Maximum orphan/dangling samples to return (default 100)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "debug_graph",
                "description": "Return internal graph diagnostics: node/edge counts by kind, top files by node count, orphan nodes, and dangling edges. Use to investigate structural anomalies or unexpected empty results.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit":         { "type": "integer", "description": "Maximum orphan/dangling samples to return (default 20)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "explain_query",
                "description": "Explain how a query_graph invocation would be executed: tokenisation, FTS phrase construction, regex validation, and expected search path. Use to diagnose why query_graph returns no results or to validate a regex pattern before running it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text":          { "type": "string",  "description": "Symbol name or identifier — same as query_graph 'text'." },
                        "kind":          { "type": "string",  "description": "Node kind filter (e.g. 'function', 'struct') — same as query_graph 'kind'." },
                        "language":      { "type": "string",  "description": "Language filter (e.g. 'rust') — same as query_graph 'language'." },
                        "limit":         { "type": "integer", "description": "Result limit (default 20)." },
                        "semantic":      { "type": "boolean", "description": "Whether semantic expansion would be applied (default false)." },
                        "regex":         { "type": "string",  "description": "Regex pattern — validated and explained. Regex-only (text empty): structural scan. text+regex: FTS5 first then UDF post-filter. Invalid pattern: error with details." },
                        "subpath":       { "type": "string",  "description": "File-path prefix filter — same as query_graph 'subpath'." },
                        "fuzzy":         { "type": "boolean", "description": "Whether fuzzy name-matching boost would be active (default false)." },
                        "hybrid":        { "type": "boolean", "description": "Whether hybrid FTS + vector retrieval would be used (default false). Requires search.embedding.url in .atlas/config.toml." },
                        "include_files": { "type": "boolean", "description": "Whether file nodes would be included in the result set (default false)." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current, repo_id, or all." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
    ]
}
