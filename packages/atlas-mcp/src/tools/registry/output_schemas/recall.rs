//! JSON-Schema builders for the `recall` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn get_global_memory_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "repo_root": { "type": "string" },
            "focus": { "type": ["object", "null"] },
            "frequent_symbols": { "type": "array", "items": { "type": "object" } },
            "frequent_files": { "type": "array", "items": { "type": "object" } },
            "workflow_patterns": { "type": "array", "items": { "type": "object" } },
            "relevant_sessions": { "type": "array", "items": { "type": "object" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "frequent_symbol_count": { "type": "integer" },
                    "frequent_file_count": { "type": "integer" },
                    "workflow_pattern_count": { "type": "integer" },
                    "relevant_session_count": { "type": "integer" }
                },
                "required": ["frequent_symbol_count", "frequent_file_count", "workflow_pattern_count", "relevant_session_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "repo_root",
            "frequent_symbols",
            "frequent_files",
            "workflow_patterns",
            "relevant_sessions",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn memory_record_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "type": "string" },
            "repo_root": { "type": "string" },
            "session_id": { "type": ["string", "null"] },
            "frontend": { "type": ["string", "null"] },
            "scope": { "type": "string" },
            "topic": { "type": "string" },
            "title": { "type": "string" },
            "body": { "type": "string" },
            "importance": { "type": "string" },
            "created_at": { "type": "string" },
            "updated_at": { "type": "string" },
            "last_accessed_at": { "type": "string" },
            "decay_score": { "type": "number" },
            "source_id": { "type": ["string", "null"] },
            "metadata": { "type": "object" }
        },
        "required": [
            "id", "repo_root", "session_id", "frontend", "scope", "topic", "title",
            "body", "importance", "created_at", "updated_at", "last_accessed_at",
            "decay_score", "source_id", "metadata"
        ]
    })
}

pub(crate) fn memory_store_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "repo_root": { "type": "string" },
            "memory": { "$ref": "#/$defs/memory_record" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "memory_id": { "type": "string" },
                    "scope": { "type": "string" },
                    "importance": { "type": "string" }
                },
                "required": ["memory_id", "scope", "importance"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "repo_root",
            "memory",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "memory_record": memory_record_schema(),
        })),
    )
}

pub(crate) fn memory_recall_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "repo_root": { "type": "string" },
            "query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "text": { "type": "string" },
                    "topic": { "type": ["string", "null"] },
                    "importance": { "type": ["string", "null"] },
                    "scope": { "type": ["string", "null"] },
                    "shared": { "type": "boolean" },
                    "requested_limit": { "type": "integer" },
                    "applied_limit": { "type": "integer" }
                },
                "required": ["text", "topic", "importance", "scope", "shared", "requested_limit", "applied_limit"]
            },
            "results": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "memory": { "$ref": "#/$defs/memory_record" },
                        "relevance_score": { "type": "integer" }
                    },
                    "required": ["memory", "relevance_score"]
                }
            },
            "retrieval_hints": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "kind": { "type": "string" },
                        "value": { "type": "string" }
                    },
                    "required": ["kind", "value"]
                }
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "match_count": { "type": "integer" },
                    "total_matches": { "type": "integer" },
                    "retrieval_hint_count": { "type": "integer" }
                },
                "required": ["match_count", "total_matches", "retrieval_hint_count"]
            },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "repo_root",
            "query",
            "results",
            "retrieval_hints",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "memory_record": memory_record_schema(),
        })),
    )
}
