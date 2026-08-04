//! JSON-Schema builders for the `content` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn search_files_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "pattern": { "type": "string" },
                    "globs": { "type": "array", "items": { "type": "string" } },
                    "exclude_globs": { "type": "array", "items": { "type": "string" } },
                    "case_sensitive": { "type": "boolean" }
                },
                "required": ["pattern", "globs", "exclude_globs", "case_sensitive"]
            },
            "subpath": { "type": ["string", "null"] },
            "matches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": { "type": "string" },
                        "file_name": { "type": "string" },
                        "extension": { "type": ["string", "null"] }
                    },
                    "required": ["path", "file_name", "extension"]
                }
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "returned_count": { "type": "integer" },
                    "result_limit": { "type": "integer" },
                    "scope": { "type": "string", "enum": ["repo_root", "subpath"] },
                    "has_matches": { "type": "boolean" }
                },
                "required": ["returned_count", "result_limit", "scope", "has_matches"]
            },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "subpath",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn search_content_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": { "type": "object" },
            "mode": { "type": "string", "enum": ["literal", "regex"] },
            "subpath": { "type": ["string", "null"] },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "mode",
            "subpath",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn read_file_excerpt_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "file": { "type": "string" },
            "selection_mode": { "type": "string" },
            "ranges": { "type": "array", "items": { "type": "object" } },
            "snippets": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "file",
            "selection_mode",
            "ranges",
            "snippets",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn get_docs_section_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "file": { "type": "string" },
            "selector_mode": { "type": "string" },
            "heading": { "type": ["object", "null"] },
            "slug": { "type": ["string", "null"] },
            "line_start": { "type": ["integer", "null"] },
            "line_end": { "type": ["integer", "null"] },
            "content": { "type": ["string", "null"] },
            "file_hash": { "type": ["string", "null"] },
            "resolved": { "type": "boolean" },
            "query": { "type": ["string", "null"] },
            "candidates": { "type": "array", "items": { "type": "object" } },
            "lines": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "file",
            "selector_mode",
            "heading",
            "slug",
            "line_start",
            "line_end",
            "content",
            "file_hash",
            "resolved",
            "candidates",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn read_file_around_match_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "file": { "type": "string" },
            "match_mode": { "type": "string", "enum": ["literal", "regex"] },
            "query": { "type": "string" },
            "before": { "type": "integer" },
            "after": { "type": "integer" },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "file",
            "match_mode",
            "query",
            "before",
            "after",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn search_templates_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "kind": { "type": ["string", "null"] },
            "subpath": { "type": ["string", "null"] },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "kind",
            "subpath",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn search_text_assets_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "kind": { "type": ["string", "null"] },
            "subpath": { "type": ["string", "null"] },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "kind",
            "subpath",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}
