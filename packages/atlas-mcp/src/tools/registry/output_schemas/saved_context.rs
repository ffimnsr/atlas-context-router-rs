//! JSON-Schema builders for the `saved_context` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn packaged_saved_source_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "source_id": { "type": "string" },
            "label": { "type": "string" },
            "source_type": { "type": "string" },
            "session_id": { "type": "string" },
            "agent_id": { "type": "string" },
            "preview": { "type": "string" },
            "retrieval_hint": { "type": "string" },
            "relevance_score": { "type": "number" },
            "context_ranking_evidence": { "type": "object" }
        },
        "required": ["source_id", "label", "source_type", "preview", "retrieval_hint", "relevance_score"]
    })
}

pub(crate) fn artifact_saved_context_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "artifact_kind": { "type": "string" },
            "source_id": { "type": "string" },
            "label": { "type": "string" },
            "source_type": { "type": "string" },
            "session_id": { "type": "string" },
            "agent_id": { "type": "string" },
            "preview": { "type": "string" },
            "retrieval_hint": { "type": "string" },
            "relevance_score": { "type": "number" },
            "context_ranking_evidence": { "type": "object" }
        },
        "required": [
            "artifact_kind",
            "source_id",
            "label",
            "source_type",
            "preview",
            "retrieval_hint",
            "relevance_score"
        ]
    })
}

pub(crate) fn search_saved_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": { "$ref": "#/$defs/saved_context_query" },
            "matches": { "type": "array", "items": { "$ref": "#/$defs/saved_context_match" } },
            "linked_decisions": { "type": "array", "items": { "type": "object" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "match_count": { "type": "integer" },
                    "total_matches": { "type": "integer" },
                    "linked_decision_count": { "type": "integer" }
                },
                "required": ["match_count", "total_matches", "linked_decision_count"]
            },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "matches",
            "linked_decisions",
            "summary",
            "truncated",
            "warnings",
        ],
        Some(serde_json::json!({
            "saved_context_query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "text": { "type": "string" },
                    "session_id": { "type": ["string", "null"] },
                    "agent_id": { "type": ["string", "null"] },
                    "cross_session": { "type": "boolean" },
                    "merge_agent_partitions": { "type": "boolean" },
                    "source_type": { "type": ["string", "null"] },
                    "requested_limit": { "type": "integer" },
                    "applied_limit": { "type": "integer" },
                    "repo_scope": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "repo_roots": { "type": "array", "items": { "type": "string" } },
                            "repo_count": { "type": "integer" }
                        },
                        "required": ["repo_roots", "repo_count"]
                    }
                },
                "required": [
                    "text",
                    "session_id",
                    "agent_id",
                    "cross_session",
                    "merge_agent_partitions",
                    "source_type",
                    "requested_limit",
                    "applied_limit",
                    "repo_scope"
                ]
            },
            "saved_context_match": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "source_id": { "type": "string" },
                    "chunk_id": { "type": "string" },
                    "chunk_index": { "type": "integer" },
                    "repo_roots": { "type": "array", "items": { "type": "string" } },
                    "title": { "type": ["string", "null"] },
                    "label": { "type": ["string", "null"] },
                    "agent_id": { "type": ["string", "null"] },
                    "source_type": { "type": ["string", "null"] },
                    "identity_kind": { "type": ["string", "null"] },
                    "identity_value": { "type": ["string", "null"] },
                    "preview": { "type": "string" },
                    "content_type": { "type": "string" }
                },
                "required": [
                    "source_id",
                    "chunk_id",
                    "chunk_index",
                    "repo_roots",
                    "label",
                    "source_type",
                    "identity_kind",
                    "identity_value",
                    "preview",
                    "content_type"
                ]
            }
        })),
    )
}

pub(crate) fn search_decisions_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "text": { "type": "string" },
                    "session_id": { "type": ["string", "null"] },
                    "agent_id": { "type": ["string", "null"] },
                    "requested_limit": { "type": "integer" }
                },
                "required": ["text", "session_id", "agent_id", "requested_limit"]
            },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "match_count": { "type": "integer" },
                    "total_matches": { "type": "integer" }
                },
                "required": ["match_count", "total_matches"]
            },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "matches",
            "summary",
            "truncated",
            "warnings",
        ],
        None,
    )
}

pub(crate) fn cross_session_search_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "text": { "type": "string" },
                    "repo_root": { "type": "string" },
                    "cross_session": { "type": "boolean" },
                    "agent_id": { "type": ["string", "null"] },
                    "merge_agent_partitions": { "type": "boolean" },
                    "source_type": { "type": ["string", "null"] },
                    "requested_limit": { "type": "integer" },
                    "applied_limit": { "type": "integer" },
                    "repo_scope": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "repo_roots": { "type": "array", "items": { "type": "string" } },
                            "repo_count": { "type": "integer" }
                        },
                        "required": ["repo_roots", "repo_count"]
                    }
                },
                "required": [
                    "text",
                    "repo_root",
                    "cross_session",
                    "agent_id",
                    "merge_agent_partitions",
                    "source_type",
                    "requested_limit",
                    "applied_limit",
                    "repo_scope"
                ]
            },
            "sessions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "session_id": { "type": "string" },
                        "agent_id": { "type": ["string", "null"] }
                    },
                    "required": ["session_id", "agent_id"]
                }
            },
            "matches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "source_id": { "type": "string" },
                        "chunk_id": { "type": "string" },
                        "chunk_index": { "type": "integer" },
                        "repo_roots": { "type": "array", "items": { "type": "string" } },
                        "session_id": { "type": ["string", "null"] },
                        "agent_id": { "type": ["string", "null"] },
                        "title": { "type": ["string", "null"] },
                        "label": { "type": ["string", "null"] },
                        "source_type": { "type": ["string", "null"] },
                        "preview": { "type": "string" }
                    },
                    "required": [
                        "source_id",
                        "chunk_id",
                        "chunk_index",
                        "repo_roots",
                        "session_id",
                        "label",
                        "source_type",
                        "preview"
                    ]
                }
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "match_count": { "type": "integer" },
                    "total_matches": { "type": "integer" },
                    "session_count": { "type": "integer" }
                },
                "required": ["match_count", "total_matches", "session_count"]
            },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "sessions",
            "matches",
            "summary",
            "truncated",
            "warnings",
        ],
        None,
    )
}

pub(crate) fn read_saved_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "found": { "type": "boolean" },
            "access_status": { "type": "string" },
            "source_id": { "type": "string" },
            "content": { "type": ["string", "null"] },
            "content_format": { "type": ["string", "null"] },
            "chunk_offset": { "type": "integer" },
            "next_chunk_offset": { "type": ["integer", "null"] },
            "truncated": { "type": "boolean" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string" },
                    "byte_count": { "type": "integer" },
                    "chunk_count": { "type": "integer" },
                    "returned_chunk_count": { "type": "integer" }
                },
                "required": ["status", "byte_count", "chunk_count", "returned_chunk_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "artifact_kind": { "type": "string" },
            "identity_kind": { "type": "string" },
            "identity_value": { "type": "string" },
            "created_at": { "type": "string" },
            "session_id": { "type": ["string", "null"] },
            "agent_id": { "type": ["string", "null"] },
            "repo_scope": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "repo_roots": { "type": "array", "items": { "type": "string" } },
                    "repo_count": { "type": "integer" },
                    "requested_repo_roots": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["repo_roots", "repo_count", "requested_repo_roots"]
            },
            "merged_agent_view": { "type": "boolean" },
            "label": { "type": "string" },
            "byte_count": { "type": "integer" },
            "chunk_count": { "type": "integer" },
            "last_included_chunk": { "type": ["integer", "null"] },
            "last_included_chunk_id": { "type": ["string", "null"] },
            "returned_chunk_ids": { "type": "array", "items": { "type": "string" } },
            "next_chunk_id": { "type": ["string", "null"] },
            "continuation_hint": { "type": ["string", "null"] },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "source_id",
            "content",
            "content_format",
            "chunk_offset",
            "next_chunk_offset",
            "truncated",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn save_context_artifact_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "storage_mode": { "type": "string" },
            "source_id": { "type": ["string", "null"] },
            "label": { "type": "string" },
            "source_type": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "preview": { "type": ["string", "null"] },
            "inline_content": { "type": ["string", "null"] },
            "content_size_bytes": { "type": "integer" },
            "chunk_count": { "type": "integer" },
            "resource_link": { "type": ["object", "null"] },
            "retrieval_hint": { "type": ["string", "null"] },
            "repo_scope": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "repo_roots": { "type": "array", "items": { "type": "string" } },
                    "repo_count": { "type": "integer" }
                },
                "required": ["repo_roots", "repo_count"]
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": { "type": "string" },
                    "stored": { "type": "boolean" },
                    "inline": { "type": "boolean" },
                    "content_type": { "type": "string" }
                },
                "required": ["session_id", "stored", "inline", "content_type"]
            },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "storage_mode",
            "source_id",
            "preview",
            "content_size_bytes",
            "chunk_count",
            "resource_link",
            "summary",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn purge_saved_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "session_id": { "type": ["string", "null"] },
            "agent_id": { "type": ["string", "null"] },
            "cutoff_days": { "type": "integer" },
            "deleted_sources": { "type": "integer" },
            "deleted_chunks": { "type": "integer" },
            "deleted_bridge_files": { "type": "integer" },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "session_id",
            "cutoff_days",
            "deleted_sources",
            "deleted_chunks",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}
