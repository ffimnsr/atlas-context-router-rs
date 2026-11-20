//! Output-schema builders for the taxonomy read tools (`list_communities`,
//! `list_flows`).
//!
//! Split out of `graph.rs` because that file was approaching the 1K-LOC
//! threshold from the AGENTS.md file-size rule.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn list_communities_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "communities": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "id": { "type": "integer" },
                        "name": { "type": "string" },
                        "algorithm": { "type": ["string", "null"] },
                        "level": { "type": ["integer", "null"] },
                        "parent_community_id": { "type": ["integer", "null"] },
                        "member_count": { "type": "integer" },
                        "nodes": { "type": "array", "items": { "type": "string" } },
                        "nodes_truncated": { "type": "boolean" }
                    },
                    // `nodes`/`nodes_truncated` only exist with include_nodes=true,
                    // so they stay optional to keep the schema exact for both modes.
                    "required": [
                        "id",
                        "name",
                        "algorithm",
                        "level",
                        "parent_community_id",
                        "member_count"
                    ]
                }
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "community_count": { "type": "integer" },
                    "total_memberships": { "type": "integer" },
                    "include_nodes": { "type": "boolean" }
                },
                "required": ["community_count", "total_memberships", "include_nodes"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &["communities", "summary", "warnings"],
        None,
    )
}

pub(crate) fn list_flows_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "flows": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "id": { "type": "integer" },
                        "name": { "type": "string" },
                        "kind": { "type": ["string", "null"] },
                        "description": { "type": ["string", "null"] },
                        "member_count": { "type": "integer" },
                        "members": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "node_qn": { "type": "string" },
                                    "position": { "type": ["integer", "null"] },
                                    "role": { "type": ["string", "null"] }
                                },
                                "required": ["node_qn", "position", "role"]
                            }
                        },
                        "members_truncated": { "type": "boolean" }
                    },
                    "required": [
                        "id",
                        "name",
                        "kind",
                        "description",
                        "member_count",
                        "members",
                        "members_truncated"
                    ]
                }
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "flow_count": { "type": "integer" },
                    "total_memberships": { "type": "integer" },
                    "matched": { "type": "boolean" },
                    "filtered_by": { "type": ["string", "null"] }
                },
                "required": ["flow_count", "total_memberships", "matched", "filtered_by"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &["flows", "summary", "warnings"],
        None,
    )
}
