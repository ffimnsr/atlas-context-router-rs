//! JSON-Schema builders for the `review` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use super::*;
use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn workflow_focus_node_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "qualified_name": { "type": "string" },
            "kind": { "type": "string" },
            "file_path": { "type": "string" },
            "relevance_score": { "type": "number" },
            "selection_reason": { "type": "string" }
        },
        "required": ["qualified_name", "kind", "file_path", "relevance_score", "selection_reason"]
    })
}

pub(crate) fn workflow_component_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "label": { "type": "string" },
            "kind": { "type": "string" },
            "changed_node_count": { "type": "integer" },
            "impacted_node_count": { "type": "integer" },
            "file_count": { "type": "integer" },
            "summary": { "type": "string" }
        },
        "required": ["label", "kind", "changed_node_count", "impacted_node_count", "file_count", "summary"]
    })
}

pub(crate) fn workflow_call_chain_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "summary": { "type": "string" },
            "steps": { "type": "array", "items": { "type": "string" } },
            "edge_kinds": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["summary", "steps", "edge_kinds"]
    })
}

pub(crate) fn noise_reduction_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "retained_nodes": { "type": "integer" },
            "retained_edges": { "type": "integer" },
            "retained_files": { "type": "integer" },
            "dropped_nodes": { "type": "integer" },
            "dropped_edges": { "type": "integer" },
            "dropped_files": { "type": "integer" },
            "rules_applied": { "type": "array", "items": { "type": "string" } }
        },
        "required": [
            "retained_nodes",
            "retained_edges",
            "retained_files",
            "dropped_nodes",
            "dropped_edges",
            "dropped_files",
            "rules_applied"
        ]
    })
}

pub(crate) fn explain_test_impact_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "affected_test_count": { "type": "integer" },
            "uncovered_symbol_count": { "type": "integer" },
            "uncovered_symbols": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["affected_test_count", "uncovered_symbol_count", "uncovered_symbols"]
    })
}

pub(crate) fn coverage_gap_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "symbol": { "type": "string" }
        },
        "required": ["symbol"]
    })
}

pub(crate) fn review_risk_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "intent": { "type": "string" },
            "node_count": { "type": "integer" },
            "edge_count": { "type": "integer" },
            "file_count": { "type": "integer" },
            "truncated": { "type": "boolean" },
            "nodes_dropped": { "type": "integer" },
            "edges_dropped": { "type": "integer" },
            "files_dropped": { "type": "integer" },
            "ambiguity_present": { "type": "boolean" },
            "cross_repo_boundary": { "type": "boolean" }
        },
        "required": [
            "intent",
            "node_count",
            "edge_count",
            "file_count",
            "truncated",
            "nodes_dropped",
            "edges_dropped",
            "files_dropped",
            "ambiguity_present",
            "cross_repo_boundary"
        ]
    })
}

pub(crate) fn boundary_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "cross_module": { "type": "boolean" },
            "cross_module_count": { "type": "integer" },
            "cross_package": { "type": "boolean" },
            "cross_package_count": { "type": "integer" },
            "cross_repo": { "type": "boolean" },
            "cross_repo_count": { "type": "integer" },
            "violations": { "type": "array", "items": { "type": "string" } }
        },
        "required": [
            "cross_module",
            "cross_module_count",
            "cross_package",
            "cross_package_count",
            "cross_repo",
            "cross_repo_count",
            "violations"
        ]
    })
}

pub(crate) fn detect_changes_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "change_source": { "$ref": "#/$defs/change_source" },
            "files": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": { "type": "string" },
                        "change_type": { "type": "string" },
                        "old_path": { "type": ["string", "null"] },
                        "node_count": { "type": ["integer", "null"] },
                        "language": { "type": ["string", "null"] },
                        "is_added": { "type": "boolean" },
                        "is_modified": { "type": "boolean" },
                        "is_deleted": { "type": "boolean" },
                        "is_renamed": { "type": "boolean" },
                        "is_copied": { "type": "boolean" }
                    },
                    "required": [
                        "path",
                        "change_type",
                        "is_added",
                        "is_modified",
                        "is_deleted",
                        "is_renamed",
                        "is_copied"
                    ]
                }
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_file_count": { "type": "integer" },
                    "resolved_file_count": { "type": "integer" },
                    "deleted_file_count": { "type": "integer" },
                    "added_file_count": { "type": "integer" },
                    "modified_file_count": { "type": "integer" },
                    "renamed_file_count": { "type": "integer" },
                    "copied_file_count": { "type": "integer" },
                    "files_with_graph_nodes": { "type": "integer" }
                },
                "required": [
                    "changed_file_count",
                    "resolved_file_count",
                    "deleted_file_count",
                    "added_file_count",
                    "modified_file_count",
                    "renamed_file_count",
                    "copied_file_count",
                    "files_with_graph_nodes"
                ]
            },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "change_source",
            "files",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "change_source": change_source_schema(),
        })),
    )
}

pub(crate) fn get_impact_radius_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "change_source": { "$ref": "#/$defs/change_source" },
            "seed_files": { "type": "array", "items": { "type": "string" } },
            "changed_symbols": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
            "impacted_symbols": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
            "impacted_files": { "type": "array", "items": { "type": "string" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_file_count": { "type": "integer" },
                    "changed_symbol_count": { "type": "integer" },
                    "impacted_symbol_count": { "type": "integer" },
                    "impacted_file_count": { "type": "integer" },
                    "relevant_edge_count": { "type": "integer" },
                    "seed_budget_count": { "type": "integer" },
                    "traversal_budget_applied": { "type": "boolean" },
                    "cross_repo_boundary": { "type": "boolean" }
                },
                "required": [
                    "changed_file_count",
                    "changed_symbol_count",
                    "impacted_symbol_count",
                    "impacted_file_count",
                    "relevant_edge_count",
                    "seed_budget_count",
                    "traversal_budget_applied",
                    "cross_repo_boundary"
                ]
            },
            "boundary_summary": { "$ref": "#/$defs/boundary_summary" },
            "truncated": { "type": "boolean" },
            "relevant_edges": { "type": "array", "items": { "$ref": "#/$defs/compact_edge" } },
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
            "change_source",
            "seed_files",
            "changed_symbols",
            "impacted_symbols",
            "impacted_files",
            "summary",
            "truncated",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "change_source": change_source_schema(),
            "compact_node": compact_node_schema(),
            "compact_edge": compact_edge_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
            "boundary_summary": boundary_summary_schema(),
        })),
    )
}

pub(crate) fn get_review_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "change_source": { "$ref": "#/$defs/change_source" },
            "changed_repos": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "repo_id": { "type": "string" },
                        "display_alias": { "type": ["string", "null"] },
                        "changed_symbol_count": { "type": "integer" }
                    },
                    "required": ["repo_id", "display_alias", "changed_symbol_count"]
                }
            },
            "changed_files": { "type": "array", "items": { "type": "string" } },
            "changed_symbols": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_node" } },
            "neighbors": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_node" } },
            "critical_edges": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_edge" } },
            "risk_summary": { "$ref": "#/$defs/review_risk_summary" },
            "boundary_summary": { "$ref": "#/$defs/boundary_summary" },
            "artifacts": { "type": "array", "items": { "$ref": "#/$defs/artifact_saved_context" } },
            "intent": { "type": "string" },
            "node_count": { "type": "integer" },
            "nodes": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_node" } },
            "edge_count": { "type": "integer" },
            "edges": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_edge" } },
            "file_count": { "type": "integer" },
            "files": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_file" } },
            "truncated": { "type": "boolean" },
            "nodes_dropped": { "type": "integer" },
            "edges_dropped": { "type": "integer" },
            "files_dropped": { "type": "integer" },
            "ambiguity_query": { "type": ["string", "null"] },
            "ambiguity_candidates": { "type": "array", "items": { "type": "string" } },
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
            "ranking_evidence_legend": { "type": "object" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "change_source",
            "changed_files",
            "changed_symbols",
            "neighbors",
            "critical_edges",
            "risk_summary",
            "artifacts",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "change_source": change_source_schema(),
            "line_range": line_range_schema(),
            "packaged_selected_node": packaged_selected_node_schema(),
            "packaged_selected_edge": packaged_selected_edge_schema(),
            "packaged_selected_file": packaged_selected_file_schema(),
            "packaged_saved_source": packaged_saved_source_schema(),
            "artifact_saved_context": artifact_saved_context_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
            "context_source_mix": context_source_mix_schema(),
            "payload_truncation": payload_truncation_schema(),
            "review_risk_summary": review_risk_summary_schema(),
            "boundary_summary": boundary_summary_schema(),
        })),
    )
}

pub(crate) fn get_minimal_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "change_source": { "$ref": "#/$defs/change_source" },
            "changed_symbols": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
            "immediate_impact": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "impacted_symbols": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
                    "impacted_files": { "type": "array", "items": { "type": "string" } },
                    "relevant_edges": { "type": "array", "items": { "$ref": "#/$defs/compact_edge" } }
                },
                "required": ["impacted_symbols", "impacted_files", "relevant_edges"]
            },
            "risk_flags": { "type": "array", "items": { "type": "string" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_file_count": { "type": "integer" },
                    "deleted_file_count": { "type": "integer" },
                    "changed_symbol_count": { "type": "integer" },
                    "impacted_symbol_count": { "type": "integer" },
                    "impacted_file_count": { "type": "integer" },
                    "truncated": { "type": "boolean" }
                },
                "required": [
                    "changed_file_count",
                    "deleted_file_count",
                    "changed_symbol_count",
                    "impacted_symbol_count",
                    "impacted_file_count",
                    "truncated"
                ]
            },

            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "change_source",
            "changed_symbols",
            "immediate_impact",
            "risk_flags",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "change_source": change_source_schema(),
            "compact_node": compact_node_schema(),
            "compact_edge": compact_edge_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
        })),
    )
}

pub(crate) fn explain_change_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "change_source": { "$ref": "#/$defs/change_source" },
            "changed_files": { "type": "array", "items": { "$ref": "#/$defs/explain_diff_file" } },
            "change_kinds": { "$ref": "#/$defs/explain_changed_by_kind" },
            "risk_level": { "type": "string" },
            "boundary_violations": { "type": "array", "items": { "$ref": "#/$defs/explain_boundary_violation" } },
            "coverage_gaps": { "type": "array", "items": { "$ref": "#/$defs/coverage_gap" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "text": { "type": "string" },
                    "changed_file_count": { "type": "integer" },
                    "changed_symbol_count": { "type": "integer" },
                    "impacted_file_count": { "type": "integer" },
                    "impacted_node_count": { "type": "integer" }
                },
                "required": [
                    "text",
                    "changed_file_count",
                    "changed_symbol_count",
                    "impacted_file_count",
                    "impacted_node_count"
                ]
            },
            "diff_summary": { "$ref": "#/$defs/explain_diff_summary" },
            "changed_symbols": { "type": "array", "items": { "$ref": "#/$defs/explain_changed_symbol" } },
            "high_impact_nodes": { "type": "array", "items": { "$ref": "#/$defs/workflow_focus_node" } },
            "impacted_components": { "type": "array", "items": { "$ref": "#/$defs/workflow_component" } },
            "call_chains": { "type": "array", "items": { "$ref": "#/$defs/workflow_call_chain" } },
            "ripple_effects": { "type": "array", "items": { "type": "string" } },
            "test_impact": { "$ref": "#/$defs/explain_test_impact" },
            "noise_reduction": { "$ref": "#/$defs/noise_reduction_summary" },
            "budget_status": { "type": "string" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "change_source",
            "changed_files",
            "change_kinds",
            "risk_level",
            "boundary_violations",
            "coverage_gaps",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "change_source": change_source_schema(),
            "explain_changed_by_kind": explain_changed_by_kind_schema(),
            "explain_changed_symbol": explain_changed_symbol_schema(),
            "explain_boundary_violation": explain_boundary_violation_schema(),
            "explain_diff_counts": explain_diff_counts_schema(),
            "explain_diff_file": explain_diff_file_schema(),
            "explain_diff_summary": explain_diff_summary_schema(),
            "workflow_focus_node": workflow_focus_node_schema(),
            "workflow_component": workflow_component_schema(),
            "workflow_call_chain": workflow_call_chain_schema(),
            "noise_reduction_summary": noise_reduction_summary_schema(),
            "explain_test_impact": explain_test_impact_schema(),
            "coverage_gap": coverage_gap_schema(),
        })),
    )
}
