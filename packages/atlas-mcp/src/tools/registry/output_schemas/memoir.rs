//! JSON-Schema builders for the memoir tool family (ICM-F).

use crate::descriptors::normalized_tool_output_schema;
use serde_json::{Value, json};

fn memoir_graph_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "type": "string" },
            "repo_root": { "type": "string" },
            "scope": { "type": "string" },
            "name": { "type": "string" },
            "description": { "type": "string" },
            "created_at": { "type": "string" },
            "updated_at": { "type": "string" },
            "metadata": { "type": "object" }
        },
        "required": [
            "id", "repo_root", "scope", "name", "description", "created_at", "updated_at",
            "metadata"
        ]
    })
}

fn memoir_concept_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "type": "string" },
            "graph_id": { "type": "string" },
            "repo_root": { "type": "string" },
            "name": { "type": "string" },
            "description": { "type": "string" },
            "kind": { "type": "string" },
            "source_id": { "type": ["string", "null"] },
            "created_at": { "type": "string" },
            "updated_at": { "type": "string" },
            "metadata": { "type": "object" }
        },
        "required": [
            "id", "graph_id", "repo_root", "name", "description", "kind", "source_id",
            "created_at", "updated_at", "metadata"
        ]
    })
}

fn memoir_relation_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "type": "string" },
            "graph_id": { "type": "string" },
            "repo_root": { "type": "string" },
            "source_concept_id": { "type": "string" },
            "target_concept_id": { "type": "string" },
            "relation_type": { "type": "string" },
            "confidence": { "type": "number" },
            "source_id": { "type": ["string", "null"] },
            "created_at": { "type": "string" },
            "metadata": { "type": "object" }
        },
        "required": [
            "id", "graph_id", "repo_root", "source_concept_id", "target_concept_id",
            "relation_type", "confidence", "source_id", "created_at", "metadata"
        ]
    })
}

fn memoir_inspection_edge_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "relation_id": { "type": "string" },
            "relation_type": { "type": "string" },
            "direction": { "type": "string", "enum": ["outgoing", "incoming"] },
            "depth": { "type": "integer" },
            "from_concept_id": { "type": "string" },
            "from_concept": { "type": "string" },
            "to_concept_id": { "type": "string" },
            "to_concept": { "type": "string" },
            "confidence": { "type": "number" },
            "source_id": { "type": ["string", "null"] },
            "created_at": { "type": "string" },
            "metadata": { "type": "object" }
        },
        "required": [
            "relation_id", "relation_type", "direction", "depth", "from_concept_id",
            "from_concept", "to_concept_id", "to_concept", "confidence", "source_id",
            "created_at", "metadata"
        ]
    })
}

fn memoir_inspection_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "graph": { "$ref": "#/$defs/memoir_graph" },
            "root": { "$ref": "#/$defs/memoir_concept" },
            "depth": { "type": "integer" },
            "relation": { "type": ["string", "null"] },
            "edges": { "type": "array", "items": { "$ref": "#/$defs/memoir_inspection_edge" } },
            "truncated": { "type": "boolean" }
        },
        "required": ["graph", "root", "depth", "relation", "edges", "truncated"]
    })
}

fn memoir_defs() -> Value {
    json!({
        "memoir_graph": memoir_graph_schema(),
        "memoir_concept": memoir_concept_schema(),
        "memoir_relation": memoir_relation_schema(),
        "memoir_inspection_edge": memoir_inspection_edge_schema(),
        "memoir_inspection": memoir_inspection_schema(),
        "memoir_delete_result": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "graph": { "type": "string" },
                "scope": { "type": "string" },
                "found": { "type": "boolean" },
                "deleted": { "type": "boolean" },
                "dry_run": { "type": "boolean" },
                "concepts_deleted": { "type": "integer" },
                "relations_deleted": { "type": "integer" }
            },
            "required": [
                "graph", "scope", "found", "deleted", "dry_run",
                "concepts_deleted", "relations_deleted"
            ]
        },
    })
}

pub(crate) fn memoir_create_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "graph": { "$ref": "#/$defs/memoir_graph" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "graph_id": { "type": "string" },
                    "name": { "type": "string" },
                    "scope": { "type": "string" },
                    "is_new": { "type": "boolean" }
                },
                "required": ["graph_id", "name", "scope", "is_new"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } }
        }),
        &["tool", "repo_root", "graph", "summary", "warnings"],
        Some(memoir_defs()),
    )
}

pub(crate) fn memoir_add_concept_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "concept": { "$ref": "#/$defs/memoir_concept" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "concept_id": { "type": "string" },
                    "graph_id": { "type": "string" },
                    "name": { "type": "string" },
                    "kind": { "type": "string" }
                },
                "required": ["concept_id", "graph_id", "name", "kind"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } }
        }),
        &["tool", "repo_root", "concept", "summary", "warnings"],
        Some(memoir_defs()),
    )
}

pub(crate) fn memoir_link_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "relation": { "$ref": "#/$defs/memoir_relation" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "relation_id": { "type": "string" },
                    "relation_type": { "type": "string" },
                    "source_concept_id": { "type": "string" },
                    "target_concept_id": { "type": "string" },
                    "confidence": { "type": "number" },
                    "created_missing_concepts": { "type": "boolean" }
                },
                "required": [
                    "relation_id", "relation_type", "source_concept_id", "target_concept_id",
                    "confidence", "created_missing_concepts"
                ]
            },
            "warnings": { "type": "array", "items": { "type": "string" } }
        }),
        &["tool", "repo_root", "relation", "summary", "warnings"],
        Some(memoir_defs()),
    )
}

pub(crate) fn memoir_inspect_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "inspection": { "$ref": "#/$defs/memoir_inspection" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "graph_id": { "type": "string" },
                    "graph_name": { "type": "string" },
                    "concept_id": { "type": "string" },
                    "concept_name": { "type": "string" },
                    "depth": { "type": "integer" },
                    "edge_count": { "type": "integer" },
                    "truncated": { "type": "boolean" }
                },
                "required": [
                    "graph_id", "graph_name", "concept_id", "concept_name", "depth",
                    "edge_count", "truncated"
                ]
            },
            "warnings": { "type": "array", "items": { "type": "string" } }
        }),
        &["tool", "repo_root", "inspection", "summary", "warnings"],
        Some(memoir_defs()),
    )
}

pub(crate) fn memoir_delete_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "result": { "$ref": "#/$defs/memoir_delete_result" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "graph": { "type": "string" },
                    "scope": { "type": "string" },
                    "found": { "type": "boolean" },
                    "deleted": { "type": "boolean" },
                    "dry_run": { "type": "boolean" },
                    "concepts_deleted": { "type": "integer" },
                    "relations_deleted": { "type": "integer" }
                },
                "required": [
                    "graph", "scope", "found", "deleted", "dry_run",
                    "concepts_deleted", "relations_deleted"
                ]
            },
            "warnings": { "type": "array", "items": { "type": "string" } }
        }),
        &["tool", "repo_root", "result", "summary", "warnings"],
        Some(memoir_defs()),
    )
}
