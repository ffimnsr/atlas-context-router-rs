//! JSON-Schema builders for the `discovery` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn tool_list_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "total_tools": { "type": "integer" },
            "returned_tools": { "type": "integer" },
            "applied_category": { "type": ["string", "null"] },
            "tools": {
                "type": "array",
                "items": { "$ref": "#/$defs/tool_inventory_entry" }
            },
            "guidance": { "$ref": "#/$defs/tool_inventory_guidance" }
        }),
        &[
            "total_tools",
            "returned_tools",
            "applied_category",
            "tools",
            "guidance",
        ],
        Some(serde_json::json!({
            "tool_inventory_entry": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" },
                    "title": { "type": "string" },
                    "description": { "type": "string" },
                    "category": { "type": "string" },
                    "result_contract": { "type": "string" },
                    "read_only": { "type": "boolean" },
                    "state_changing": { "type": "boolean" },
                    "destructive": { "type": "boolean" }
                },
                "required": [
                    "name",
                    "title",
                    "description",
                    "category",
                    "result_contract",
                    "read_only",
                    "state_changing",
                    "destructive"
                ]
            },
            "tool_inventory_guidance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "list": { "type": "string" },
                    "search": { "type": "string" },
                    "help": { "type": "string" }
                },
                "required": ["list", "search", "help"]
            }
        })),
    )
}

pub(crate) fn tool_search_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": { "type": "string" },
            "total_matches": { "type": "integer" },
            "returned_matches": { "type": "integer" },
            "matches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "name": { "type": "string" },
                        "title": { "type": "string" },
                        "description": { "type": "string" },
                        "category": { "type": "string" },
                        "result_contract": { "type": "string" },
                        "score": { "type": "integer" },
                        "match_reasons": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "score_factors": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "factor": { "type": "string" },
                                    "contribution": { "type": "integer" },
                                    "detail": { "type": ["string", "null"] }
                                },
                                "required": ["factor", "contribution", "detail"]
                            }
                        }
                    },
                    "required": [
                        "name",
                        "title",
                        "description",
                        "category",
                        "result_contract",
                        "score",
                        "match_reasons",
                        "score_factors"
                    ]
                }
            },
            "suggestions": {
                "type": "array",
                "items": { "type": "string" }
            },
            "guidance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "list": { "type": "string" },
                    "search": { "type": "string" },
                    "help": { "type": "string" }
                },
                "required": ["list", "search", "help"]
            }
        }),
        &[
            "query",
            "total_matches",
            "returned_matches",
            "matches",
            "suggestions",
            "guidance",
        ],
        None,
    )
}

pub(crate) fn man_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "requested_namespace": { "type": "string" },
            "requested_tool_name": { "type": "string" },
            "resolved_tool_name": { "type": "string" },
            "description": { "type": "string" },
            "tool_structure": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "purpose": { "type": "string" },
                    "operation_name": { "type": "string" },
                    "request_shape": { "type": "string" },
                    "response_shape": { "type": "string" },
                    "result_contract": { "type": "string" },
                    "annotations": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "read_only": { "type": "boolean" },
                            "state_changing": { "type": "boolean" },
                            "destructive": { "type": "boolean" }
                        },
                        "required": ["read_only", "state_changing", "destructive"]
                    }
                },
                "required": [
                    "purpose",
                    "operation_name",
                    "request_shape",
                    "response_shape",
                    "result_contract",
                    "annotations"
                ]
            },
            "input_args": {
                "type": "array",
                "items": { "$ref": "#/$defs/manual_field" }
            },
            "input_contract": { "$ref": "#/$defs/manual_input_contract" },
            "output_response": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "response_shape": { "type": "string" },
                    "structured_content_available": { "type": "boolean" },
                    "response_fields": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/manual_field" }
                    },
                    "metadata_fields": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/manual_field" }
                    },
                    "error_payload_fields": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/manual_field" }
                    }
                },
                "required": [
                    "response_shape",
                    "structured_content_available",
                    "response_fields",
                    "metadata_fields",
                    "error_payload_fields"
                ]
            },
            "usage": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "cli": { "type": "string" },
                    "mcp_manual_tool_call": { "type": "string" },
                    "target_tool_call_examples": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["cli", "mcp_manual_tool_call", "target_tool_call_examples"]
            },
            "error_cases": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "code": { "type": "string" },
                        "when": { "type": "string" },
                        "behavior": { "type": "string" }
                    },
                    "required": ["code", "when", "behavior"]
                }
            },
            "truncation": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "description_truncated": { "type": "boolean" },
                    "usage_examples_truncated": { "type": "boolean" }
                },
                "required": ["description_truncated", "usage_examples_truncated"]
            }
        }),
        &[
            "requested_namespace",
            "requested_tool_name",
            "resolved_tool_name",
            "description",
            "tool_structure",
            "input_args",
            "input_contract",
            "output_response",
            "usage",
            "error_cases",
            "truncation",
        ],
        Some(serde_json::json!({
            "manual_field": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" },
                    "field_type": { "type": "string" },
                    "required": { "type": "boolean" },
                    "default_value": { "type": ["string", "null"] },
                    "enum_values": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "description": { "type": "string" }
                },
                "required": [
                    "name",
                    "field_type",
                    "required",
                    "default_value",
                    "enum_values",
                    "description"
                ]
            },
            "manual_input_variant": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "value": { "type": "string" },
                    "required_companion_fields": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "minimal_example": { "type": "object" }
                },
                "required": ["value", "required_companion_fields", "minimal_example"]
            },
            "manual_input_family": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "family_name": { "type": "string" },
                    "family_kind": { "type": "string" },
                    "discriminant_field": { "type": ["string", "null"] },
                    "accepted_values": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "mutually_exclusive_legacy_fields": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "variants": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/manual_input_variant" }
                    }
                },
                "required": [
                    "family_name",
                    "family_kind",
                    "discriminant_field",
                    "accepted_values",
                    "mutually_exclusive_legacy_fields",
                    "variants"
                ]
            },
            "manual_input_contract": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "canonical_form": { "type": "string" },
                    "families": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/manual_input_family" }
                    },
                    "notes": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["canonical_form", "families", "notes"]
            }
        })),
    )
}
