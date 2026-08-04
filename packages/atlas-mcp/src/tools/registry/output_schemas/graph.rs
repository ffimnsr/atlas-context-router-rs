//! JSON-Schema builders for the `graph` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn explain_query_input_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "text": { "type": "string" },
            "kind": { "type": ["string", "null"] },
            "language": { "type": ["string", "null"] },
            "limit": { "type": "integer" },
            "semantic": { "type": "boolean" },
            "expand": { "type": "boolean" },
            "expand_hops": { "type": "integer" },
            "regex": { "type": ["string", "null"] },
            "subpath": { "type": ["string", "null"] },
            "fuzzy": { "type": "boolean" },
            "hybrid": { "type": "boolean" },
            "include_files": { "type": "boolean" }
        },
        "required": [
            "text",
            "kind",
            "language",
            "limit",
            "semantic",
            "expand",
            "expand_hops",
            "regex",
            "subpath",
            "fuzzy",
            "hybrid",
            "include_files"
        ]
    })
}

pub(crate) fn explain_query_filters_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": { "type": "boolean" },
            "language": { "type": "boolean" },
            "subpath": { "type": "boolean" },
            "fuzzy": { "type": "boolean" },
            "hybrid": { "type": "boolean" },
            "semantic": { "type": "boolean" },
            "expand": { "type": "boolean" },
            "include_files": { "type": "boolean" }
        },
        "required": ["kind", "language", "subpath", "fuzzy", "hybrid", "semantic", "expand", "include_files"]
    })
}

pub(crate) fn backend_capabilities_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "lexical_fts": { "type": "boolean" },
            "dense_vector": { "type": "boolean" },
            "hybrid_lexical_vector": { "type": "boolean" },
            "sparse_bm25_native": { "type": "boolean" },
            "metadata_filtering": { "type": "boolean" }
        },
        "required": ["lexical_fts", "dense_vector", "hybrid_lexical_vector", "sparse_bm25_native", "metadata_filtering"]
    })
}

pub(crate) fn explain_query_match_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "score": { "type": "number" },
            "kind": { "type": "string" },
            "qualified_name": { "type": "string" },
            "file_path": { "type": "string" },
            "line_start": { "type": "integer" },
            "language": { "type": "string" },
            "ranking_evidence": { "type": "object" }
        },
        "required": ["score", "kind", "qualified_name", "file_path", "line_start", "language"]
    })
}

pub(crate) fn compact_node_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "qn": { "type": "string" },
            "kind": { "type": "string" },
            "file": { "type": "string" },
            "line": { "type": "integer" },
            "parent": { "type": "string" },
            "sig": { "type": "string" },
            "lang": { "type": "string" }
        },
        "required": ["qn", "kind", "file", "line", "lang"]
    })
}

pub(crate) fn compact_edge_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "from": { "type": "string" },
            "to": { "type": "string" },
            "kind": { "type": "string" }
        },
        "required": ["from", "to", "kind"]
    })
}

pub(crate) fn line_range_schema() -> Value {
    serde_json::json!({
        "type": "array",
        "prefixItems": [{ "type": "integer" }, { "type": "integer" }],
        "minItems": 2,
        "maxItems": 2
    })
}

pub(crate) fn packaged_selected_node_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "reason": { "type": "string" },
            "distance": { "type": "integer" },
            "context_ranking_evidence": { "type": "object" },
            "qn": { "type": "string" },
            "kind": { "type": "string" },
            "file": { "type": "string" },
            "line": { "type": "integer" },
            "parent": { "type": "string" },
            "sig": { "type": "string" },
            "lang": { "type": "string" }
        },
        "required": ["reason", "distance", "qn", "kind", "file", "line", "lang"]
    })
}

pub(crate) fn packaged_selected_edge_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "reason": { "type": "string" },
            "context_ranking_evidence": { "type": "object" },
            "from": { "type": "string" },
            "to": { "type": "string" },
            "kind": { "type": "string" }
        },
        "required": ["reason", "from", "to", "kind"]
    })
}

pub(crate) fn packaged_selected_file_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" },
            "reason": { "type": "string" },
            "line_ranges": { "type": "array", "items": { "$ref": "#/$defs/line_range" } }
        },
        "required": ["path", "reason"]
    })
}

pub(crate) fn change_source_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": { "type": "string" },
            "resolved_files": { "type": "array", "items": { "type": "string" } },
            "deleted_files": { "type": "array", "items": { "type": "string" } },
            "base": { "type": ["string", "null"] }
        },
        "required": ["kind", "resolved_files", "deleted_files", "base"]
    })
}

pub(crate) fn seed_budget_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "seed_kind": { "type": "string" },
            "requested_seed_count": { "type": "integer" },
            "accepted_seed_count": { "type": "integer" },
            "omitted_seed_count": { "type": "integer" },
            "budget_hit": { "type": "boolean" },
            "partial": { "type": "boolean" },
            "safe_to_answer": { "type": "boolean" },
            "suggested_narrower_query": { "type": "string" }
        },
        "required": [
            "seed_kind",
            "requested_seed_count",
            "accepted_seed_count",
            "omitted_seed_count",
            "budget_hit",
            "partial",
            "safe_to_answer"
        ]
    })
}

pub(crate) fn traversal_budget_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "requested_depth": { "type": "integer" },
            "accepted_depth": { "type": "integer" },
            "requested_node_budget": { "type": "integer" },
            "accepted_node_budget": { "type": "integer" },
            "requested_edge_budget": { "type": "integer" },
            "accepted_edge_budget": { "type": "integer" },
            "emitted_node_count": { "type": "integer" },
            "emitted_edge_count": { "type": "integer" },
            "omitted_edge_count": { "type": "integer" },
            "budget_hit": { "type": "boolean" },
            "suggested_narrower_query": { "type": "string" }
        },
        "required": [
            "requested_depth",
            "accepted_depth",
            "requested_node_budget",
            "accepted_node_budget",
            "requested_edge_budget",
            "accepted_edge_budget",
            "emitted_node_count",
            "emitted_edge_count",
            "omitted_edge_count",
            "budget_hit"
        ]
    })
}

pub(crate) fn context_source_mix_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "source_kind": { "type": "string" },
            "items_included": { "type": "integer" },
            "items_dropped": { "type": "integer" },
            "tokens_used": { "type": "integer" }
        },
        "required": ["source_kind", "items_included", "items_dropped", "tokens_used"]
    })
}

pub(crate) fn payload_truncation_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "bytes_requested": { "type": "integer" },
            "bytes_emitted": { "type": "integer" },
            "tokens_estimated": { "type": "integer" },
            "token_budget_applied": { "type": "integer" },
            "omitted_node_count": { "type": "integer" },
            "omitted_file_count": { "type": "integer" },
            "omitted_source_count": { "type": "integer" },
            "omitted_byte_count": { "type": "integer" },
            "continuation_hint": { "type": "string" },
            "source_mix": { "type": "array", "items": { "$ref": "#/$defs/context_source_mix" } }
        },
        "required": [
            "bytes_requested",
            "bytes_emitted",
            "tokens_estimated",
            "omitted_node_count",
            "omitted_file_count",
            "omitted_source_count",
            "omitted_byte_count"
        ]
    })
}

pub(crate) fn ambiguity_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": { "type": ["string", "null"] },
            "candidates": { "type": "array", "items": { "type": "string" } },
            "candidates_detailed": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "qualified_name": { "type": "string" },
                        "file_path": { "type": ["string", "null"] },
                        "kind": { "type": ["string", "null"] },
                        "repo": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "repo_id": { "type": ["string", "null"] },
                                "display_alias": { "type": ["string", "null"] }
                            },
                            "required": ["repo_id", "display_alias"]
                        }
                    },
                    "required": ["qualified_name", "file_path", "kind", "repo"]
                }
            }
        },
        "required": ["query", "candidates", "candidates_detailed"]
    })
}

pub(crate) fn ranked_symbol_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "qn": { "type": "string" },
            "reason": { "type": "string" },
            "distance": { "type": "integer" }
        },
        "required": ["qn", "reason", "distance"]
    })
}

pub(crate) fn ranked_edge_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "from": { "type": "string" },
            "to": { "type": "string" },
            "kind": { "type": "string" }
        },
        "required": ["from", "to", "kind"]
    })
}

pub(crate) fn ranked_file_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" },
            "reason": { "type": "string" }
        },
        "required": ["path", "reason"]
    })
}

pub(crate) fn explain_changed_by_kind_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "api_change": { "type": "integer" },
            "signature_change": { "type": "integer" },
            "internal_change": { "type": "integer" }
        },
        "required": ["api_change", "signature_change", "internal_change"]
    })
}

pub(crate) fn explain_changed_symbol_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "qn": { "type": "string" },
            "kind": { "type": "string" },
            "file": { "type": "string" },
            "line": { "type": "integer" },
            "change_kind": { "type": "string" },
            "lang": { "type": "string" },
            "sig": { "type": "string" }
        },
        "required": ["qn", "kind", "file", "line", "change_kind", "lang"]
    })
}

pub(crate) fn explain_boundary_violation_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": { "type": "string" },
            "description": { "type": "string" },
            "nodes": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["kind", "description", "nodes"]
    })
}

pub(crate) fn explain_diff_counts_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "added": { "type": "integer" },
            "modified": { "type": "integer" },
            "deleted": { "type": "integer" },
            "renamed": { "type": "integer" },
            "copied": { "type": "integer" }
        },
        "required": ["added", "modified", "deleted", "renamed", "copied"]
    })
}

pub(crate) fn explain_diff_file_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" },
            "change_type": { "type": "string" },
            "old_path": { "type": "string" },
            "changed_symbol_count": { "type": "integer" },
            "impacted_symbol_count": { "type": "integer" }
        },
        "required": ["path", "change_type", "changed_symbol_count", "impacted_symbol_count"]
    })
}

pub(crate) fn explain_diff_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "counts": { "$ref": "#/$defs/explain_diff_counts" },
            "files": { "type": "array", "items": { "$ref": "#/$defs/explain_diff_file" } }
        },
        "required": ["counts", "files"]
    })
}

pub(crate) fn traverse_edge_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "from": { "type": "string" },
            "to": { "type": "string" },
            "kind": { "type": "string" },
            "direction": { "type": "string" }
        },
        "required": ["from", "to", "kind", "direction"]
    })
}

pub(crate) fn query_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": { "$ref": "#/$defs/query_graph_query" },
            "matches": { "type": "array", "items": { "$ref": "#/$defs/query_graph_match" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "match_count": { "type": "integer" },
                    "returned_count": { "type": "integer" },
                    "usage_edges_included": { "type": "boolean" },
                    "relationship_tools": { "type": "array", "items": { "type": "string" } },
                    "ranking_evidence_legend": { "type": "object" }
                },
                "required": [
                    "match_count",
                    "returned_count",
                    "usage_edges_included",
                    "relationship_tools",
                    "ranking_evidence_legend"
                ]
            },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "query_intent": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": { "type": "string" },
                    "source_text": { "type": "string" },
                    "normalized_text": { "type": "string" },
                    "intent": { "type": "string" },
                    "accepted": { "type": "boolean" }
                },
                "required": ["kind", "source_text", "normalized_text", "intent", "accepted"]
            },
            "repo_scope_selection": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": { "type": "string" },
                    "repo_ids": { "type": "array", "items": { "type": "string" } },
                    "repo_count": { "type": "integer" }
                },
                "required": ["kind", "repo_ids", "repo_count"]
            },
            "query_graph_repo_scope": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "selection": { "anyOf": [{ "$ref": "#/$defs/repo_scope_selection" }, { "type": "null" }] },
                    "deprecated_input_fields": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["selection", "deprecated_input_fields"]
            },
            "query_graph_query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "text": { "type": "string" },
                    "normalized_text": { "type": "string" },
                    "regex": { "type": ["string", "null"] },
                    "kind": { "type": ["string", "null"] },
                    "language": { "type": ["string", "null"] },
                    "requested_limit": { "type": "integer" },
                    "applied_limit": { "type": "integer" },
                    "semantic": { "type": "boolean" },
                    "expand": { "type": "boolean" },
                    "expand_hops": { "type": "integer" },
                    "subpath": { "type": ["string", "null"] },
                    "fuzzy": { "type": "boolean" },
                    "hybrid": { "type": "boolean" },
                    "include_files": { "type": "boolean" },
                    "repo_scope": { "$ref": "#/$defs/query_graph_repo_scope" },
                    "query_intent": { "$ref": "#/$defs/query_intent" },
                    "active_query_mode": { "type": "string" }
                },
                "required": [
                    "text",
                    "normalized_text",
                    "regex",
                    "kind",
                    "language",
                    "requested_limit",
                    "applied_limit",
                    "semantic",
                    "expand",
                    "expand_hops",
                    "subpath",
                    "fuzzy",
                    "hybrid",
                    "include_files",
                    "repo_scope",
                    "query_intent",
                    "active_query_mode"
                ]
            },
            "query_graph_repo": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "repo_id": { "type": ["string", "null"] },
                    "display_alias": { "type": ["string", "null"] }
                },
                "required": ["repo_id", "display_alias"]
            },
            "query_graph_match": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "score": { "type": "number" },
                    "ranking_evidence": { "type": ["object", "null"] },
                    "repo": { "$ref": "#/$defs/query_graph_repo" },
                    "qn": { "type": "string" },
                    "kind": { "type": "string" },
                    "file": { "type": "string" },
                    "line": { "type": "integer" },
                    "parent": { "type": "string" },
                    "sig": { "type": "string" },
                    "lang": { "type": "string" }
                },
                "required": ["score", "repo", "qn", "kind", "file", "line", "lang"]
            }
        })),
    )
}

pub(crate) fn batch_query_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "items": { "type": "array", "items": { "$ref": "#/$defs/batch_query_item" } },
            "results": { "type": "array", "items": { "$ref": "#/$defs/batch_query_result" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query_count": { "type": "integer" },
                    "ranking_evidence_legend": { "type": "object" }
                },
                "required": ["query_count", "ranking_evidence_legend"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "items",
            "results",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "query_intent": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": { "type": "string" },
                    "source_text": { "type": "string" },
                    "normalized_text": { "type": "string" },
                    "intent": { "type": "string" },
                    "accepted": { "type": "boolean" }
                },
                "required": ["kind", "source_text", "normalized_text", "intent", "accepted"]
            },
            "batch_query_item": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query_index": { "type": "integer" },
                    "text": { "type": "string" },
                    "normalized_text": { "type": "string" },
                    "regex": { "type": ["string", "null"] },
                    "kind": { "type": ["string", "null"] },
                    "language": { "type": ["string", "null"] },
                    "requested_limit": { "type": "integer" },
                    "applied_limit": { "type": "integer" },
                    "semantic": { "type": "boolean" },
                    "expand": { "type": "boolean" },
                    "expand_hops": { "type": "integer" },
                    "subpath": { "type": ["string", "null"] },
                    "fuzzy": { "type": "boolean" },
                    "hybrid": { "type": "boolean" },
                    "include_files": { "type": "boolean" },
                    "query_intent": { "$ref": "#/$defs/query_intent" }
                },
                "required": [
                    "query_index",
                    "text",
                    "normalized_text",
                    "regex",
                    "kind",
                    "language",
                    "requested_limit",
                    "applied_limit",
                    "semantic",
                    "expand",
                    "expand_hops",
                    "subpath",
                    "fuzzy",
                    "hybrid",
                    "include_files",
                    "query_intent"
                ]
            },
            "batch_query_match": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "score": { "type": "number" },
                    "ranking_evidence": { "type": ["object", "null"] },
                    "repo": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "repo_id": { "type": ["string", "null"] },
                            "display_alias": { "type": ["string", "null"] }
                        },
                        "required": ["repo_id", "display_alias"]
                    },
                    "name": { "type": "string" },
                    "qualified_name": { "type": "string" },
                    "kind": { "type": "string" },
                    "file_path": { "type": "string" },
                    "line_start": { "type": "integer" },
                    "language": { "type": "string" }
                },
                "required": ["score", "repo", "name", "qualified_name", "kind", "file_path", "line_start", "language"]
            },
            "batch_query_result": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query_index": { "type": "integer" },
                    "matches": { "type": "array", "items": { "$ref": "#/$defs/batch_query_match" } },
                    "summary": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "match_count": { "type": "integer" },
                            "returned_count": { "type": "integer" }
                        },
                        "required": ["match_count", "returned_count"]
                    },
                    "truncated": { "type": "boolean" },
                    "warnings": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["query_index", "matches", "summary", "truncated", "warnings"]
            }
        })),
    )
}

pub(crate) fn explain_query_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "input": { "$ref": "#/$defs/explain_query_input" },
            "normalized_query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "active_query_mode": { "type": "string" },
                    "search_path": { "type": "string" },
                    "indexed_node_count": { "type": ["integer", "null"] },
                    "db_exists": { "type": "boolean" },
                    "ranking_factors": { "type": "array", "items": { "type": "string" } },
                    "filters_applied": { "$ref": "#/$defs/explain_query_filters" },
                    "active_capabilities": { "$ref": "#/$defs/backend_capabilities" }
                },
                "required": ["active_query_mode", "search_path", "indexed_node_count", "db_exists", "ranking_factors", "filters_applied", "active_capabilities"]
            },
            "tokenization": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "fts_tokens": { "type": "array", "items": { "type": "string" } },
                    "fts_phrase": { "type": ["string", "null"] }
                },
                "required": ["fts_tokens", "fts_phrase"]
            },
            "fts_plan": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "enabled": { "type": "boolean" },
                    "phrase": { "type": ["string", "null"] },
                    "token_count": { "type": "integer" },
                    "limit": { "type": "integer" },
                    "semantic": { "type": "boolean" },
                    "expand": { "type": "boolean" },
                    "include_files": { "type": "boolean" }
                },
                "required": ["enabled", "phrase", "token_count", "limit", "semantic", "expand", "include_files"]
            },
            "regex_plan": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "enabled": { "type": "boolean" },
                    "pattern": { "type": ["string", "null"] },
                    "valid": { "type": "boolean" },
                    "error": { "type": ["string", "null"] }
                },
                "required": ["enabled", "pattern", "valid", "error"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "latency_ms": { "type": ["integer", "null"] },
            "result_count": { "type": "integer" },
            "matches": { "type": "array", "items": { "$ref": "#/$defs/explain_query_match" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "input",
            "normalized_query",
            "tokenization",
            "fts_plan",
            "regex_plan",
            "warnings",
            "matches",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "explain_query_input": explain_query_input_schema(),
            "explain_query_filters": explain_query_filters_schema(),
            "backend_capabilities": backend_capabilities_schema(),
            "explain_query_match": explain_query_match_schema(),
        })),
    )
}
