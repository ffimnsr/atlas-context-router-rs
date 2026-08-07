//! JSON-Schema builders for the `relationships` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use super::*;
use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn traverse_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "root_symbol": { "type": "string" },
            "direction": { "type": "string" },
            "depth": { "type": "integer" },
            "nodes": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
            "edges": { "type": "array", "items": { "$ref": "#/$defs/traverse_edge" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_symbol_count": { "type": "integer" },
                    "impacted_symbol_count": { "type": "integer" },
                    "impacted_file_count": { "type": "integer" },
                    "relevant_edge_count": { "type": "integer" }
                },
                "required": [
                    "changed_symbol_count",
                    "impacted_symbol_count",
                    "impacted_file_count",
                    "relevant_edge_count"
                ]
            },
            "truncated": { "type": "boolean" },
            "impacted_files": { "type": "array", "items": { "type": "string" } },
            "seed_budgets": { "type": "array", "items": { "$ref": "#/$defs/seed_budget" } },
            "traversal_budget": {
                "oneOf": [
                    { "$ref": "#/$defs/traversal_budget" },
                    { "type": "null" }
                ]
            },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "root_symbol",
            "direction",
            "depth",
            "nodes",
            "edges",
            "summary",
            "truncated",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "compact_node": compact_node_schema(),
            "compact_edge": compact_edge_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
            "traverse_edge": traverse_edge_schema(),
        })),
    )
}

pub(crate) fn get_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "target": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": { "type": "string" },
                    "query": { "type": ["string", "null"] },
                    "file": { "type": ["string", "null"] },
                    "files": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["kind", "query", "file", "files"]
            },
            "query": { "type": ["string", "null"] },
            "file": { "type": ["string", "null"] },
            "files": { "type": "array", "items": { "type": "string" } },
            "ranked_symbols": { "type": "array", "items": { "$ref": "#/$defs/ranked_symbol_summary" } },
            "ranked_edges": { "type": "array", "items": { "$ref": "#/$defs/ranked_edge_summary" } },
            "ranked_files": { "type": "array", "items": { "$ref": "#/$defs/ranked_file_summary" } },
            "assets": { "type": "array", "items": { "$ref": "#/$defs/artifact_saved_context" } },
            "ambiguity": { "$ref": "#/$defs/ambiguity" },
            "context_files": { "type": "array", "items": { "type": "string" } },
            "detail_controls": { "type": "object" },
            "agent_scope": { "type": "object" },
            "ranking_evidence_legend": { "type": "object" },
            "lookup": { "type": "object" },
            "cross_repo_context_hops": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "enabled": { "type": "boolean" },
                    "edge_count": { "type": "integer" }
                },
                "required": ["enabled", "edge_count"]
            },
            "intent": { "type": "string" },
            "node_count": { "type": "integer" },
            "nodes": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_node" } },
            "edge_count": { "type": "integer" },
            "edges": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_edge" } },
            "file_count": { "type": "integer" },
            "files_dropped": { "type": "integer" },
            "truncated": { "type": "boolean" },
            "nodes_dropped": { "type": "integer" },
            "edges_dropped": { "type": "integer" },
            "seed_budgets": { "type": "array", "items": { "$ref": "#/$defs/seed_budget" } },
            "traversal_budget": {
                "oneOf": [
                    { "$ref": "#/$defs/traversal_budget" },
                    { "type": "null" }
                ]
            },
            "payload_truncation": {
                "oneOf": [
                    { "$ref": "#/$defs/payload_truncation" },
                    { "type": "null" }
                ]
            },
            "source_mix": { "type": "array", "items": { "$ref": "#/$defs/context_source_mix" } },
            "token_budget_applied": { "type": ["integer", "null"] },
            "budget_status": { "type": "string" },
            "linked_decisions": { "type": "array", "items": { "type": "object" } },
            "decision_lookup_query": { "type": "string" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "target",
            "query",
            "file",
            "files",
            "ranked_symbols",
            "ranked_edges",
            "ranked_files",
            "assets",
            "ambiguity",
            "context_files",
            "detail_controls",
            "agent_scope",
            "lookup",
            "truncated",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "line_range": line_range_schema(),
            "ranked_symbol_summary": ranked_symbol_summary_schema(),
            "ranked_edge_summary": ranked_edge_summary_schema(),
            "ranked_file_summary": ranked_file_summary_schema(),
            "artifact_saved_context": artifact_saved_context_schema(),
            "ambiguity": ambiguity_schema(),
            "packaged_selected_node": packaged_selected_node_schema(),
            "packaged_selected_edge": packaged_selected_edge_schema(),
            "packaged_saved_source": packaged_saved_source_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
            "context_source_mix": context_source_mix_schema(),
            "payload_truncation": payload_truncation_schema(),
            "token_accounting": token_accounting_schema(),
        })),
    )
}

pub(crate) fn symbol_neighbors_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "symbol": { "type": "object" },
            "callers": { "type": "array", "items": { "type": "object" } },
            "callees": { "type": "array", "items": { "type": "object" } },
            "call_sites": { "type": "array", "items": { "type": "object" } },
            "tests": { "type": "array", "items": { "type": "object" } },
            "siblings": { "type": "array", "items": { "type": "object" } },
            "imports": { "type": "array", "items": { "type": "object" } },
            "lookup": { "type": "object" },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "symbol",
            "callers",
            "callees",
            "call_sites",
            "tests",
            "siblings",
            "imports",
            "lookup",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn cross_file_links_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "source_file": { "type": "string" },
            "linked_files": { "type": "array", "items": { "type": "object" } },
            "coupling_metric": { "type": "object" },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "source_file",
            "linked_files",
            "coupling_metric",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn concept_clusters_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "seed_files": { "type": "array", "items": { "type": "string" } },
            "clusters": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "seed_files",
            "clusters",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn resolve_symbol_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": { "type": "object" },
            "best_match": { "type": ["object", "null"] },
            "ambiguity": { "type": "object" },
            "suggestions": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "best_match",
            "ambiguity",
            "suggestions",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}
