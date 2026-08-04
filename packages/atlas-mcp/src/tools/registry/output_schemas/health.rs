//! JSON-Schema builders for the `health` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn repo_registry_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "registry_path": { "type": "string" },
            "schema_version": { "type": "integer" },
            "root_repo_id": { "type": "string" },
            "registration_count": { "type": "integer" },
            "warning_count": { "type": "integer" },
            "registrations": { "type": "array", "items": { "type": "object" } },
            "warnings": { "type": "array", "items": { "type": "object" } }
        }),
        &[
            "registry_path",
            "schema_version",
            "root_repo_id",
            "registration_count",
            "warning_count",
            "registrations",
            "warnings",
        ],
        None,
    )
}

pub(crate) fn list_graph_stats_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "file_count": { "type": "integer" },
            "node_count": { "type": "integer" },
            "edge_count": { "type": "integer" },
            "nodes_by_kind": {
                "type": "array",
                "items": {
                    "type": "array",
                    "prefixItems": [
                        { "type": "string" },
                        { "type": "integer" }
                    ],
                    "minItems": 2,
                    "maxItems": 2
                }
            },
            "languages": {
                "type": "array",
                "items": { "type": "string" }
            },
            "last_indexed_at": {
                "type": ["string", "null"]
            }
        }),
        &[
            "file_count",
            "node_count",
            "edge_count",
            "nodes_by_kind",
            "languages",
            "last_indexed_at",
        ],
        None,
    )
}

pub(crate) fn broker_status_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "ok": { "type": "boolean" },
            "pid": { "type": "integer" },
            "version": { "type": "string" },
            "uptime_secs": { "type": "integer" },
            "worker_threads_configured": { "type": "integer" },
            "repo_root": { "type": "string" },
            "db_path": { "type": "string" }
        }),
        &[
            "ok",
            "pid",
            "version",
            "uptime_secs",
            "worker_threads_configured",
            "repo_root",
            "db_path",
        ],
        None,
    )
}

pub(crate) fn build_stage_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string" },
            "status": { "type": "string" },
            "item_count": { "type": "integer" },
            "details": { "type": "object" }
        },
        "required": ["name", "status", "item_count", "details"]
    })
}

pub(crate) fn postprocess_stage_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "stage": { "type": "string" },
            "status": { "type": "string" },
            "mode": { "type": "string" },
            "affected_file_count": { "type": "integer" },
            "item_count": { "type": "integer" },
            "elapsed_ms": { "type": "integer" },
            "error_code": { "type": ["string", "null"] },
            "message": { "type": ["string", "null"] },
            "details": { "type": "object" }
        },
        "required": [
            "stage",
            "status",
            "mode",
            "affected_file_count",
            "item_count",
            "elapsed_ms",
            "error_code",
            "message",
            "details"
        ]
    })
}

pub(crate) fn orphan_node_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": { "type": "string" },
            "qualified_name": { "type": "string" },
            "file_path": { "type": "string" },
            "line_start": { "type": "integer" }
        },
        "required": ["kind", "qualified_name", "file_path", "line_start"]
    })
}

pub(crate) fn dangling_edge_diagnostic_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "type": "integer" },
            "kind": { "type": "string" },
            "source_qn": { "type": "string" },
            "target_qn": { "type": "string" },
            "missing_side": { "type": "string" }
        },
        "required": ["id", "kind", "source_qn", "target_qn", "missing_side"]
    })
}

pub(crate) fn doctor_check_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string" },
            "status": { "type": "string" },
            "message": { "type": "string" },
            "details": { "type": "object" },
            "fix_hint": { "type": ["string", "null"] }
        },
        "required": ["name", "status", "message", "details", "fix_hint"]
    })
}

pub(crate) fn debug_top_file_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" },
            "node_count": { "type": "integer" }
        },
        "required": ["path", "node_count"]
    })
}

pub(crate) fn build_or_update_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "status": { "type": "string" },
            "source": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "target_kind": { "type": "string" },
                    "base_ref": { "type": ["string", "null"] },
                    "staged": { "type": "boolean" }
                },
                "required": ["target_kind", "base_ref", "staged"]
            },
            "files_scanned": { "type": "integer" },
            "files_changed": { "type": "integer" },
            "files_parsed": { "type": "integer" },
            "files_deleted": { "type": "integer" },
            "files_renamed": { "type": "integer" },
            "files_skipped_unsupported": { "type": "integer" },
            "files_skipped_unchanged": { "type": "integer" },
            "parse_error_count": { "type": "integer" },
            "chunk_upsert_failure_count": { "type": "integer" },
            "call_target_reconcile_failure_count": { "type": "integer" },
            "nodes_written": { "type": "integer" },
            "edges_written": { "type": "integer" },
            "duration_ms": { "type": "integer" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "stages": { "type": "array", "items": { "$ref": "#/$defs/build_stage" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "budget_status": { "type": "string" },
                    "budget_hit": { "type": "boolean" },
                    "partial": { "type": "boolean" },
                    "safe_to_answer": { "type": "boolean" },
                    "budget_counters": { "type": "object" }
                },
                "required": ["budget_status", "budget_hit", "partial", "safe_to_answer", "budget_counters"]
            },
            "build_status": { "type": ["object", "null"] },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "status",
            "files_scanned",
            "files_changed",
            "nodes_written",
            "edges_written",
            "duration_ms",
            "stages",
            "warnings",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "build_stage": build_stage_schema(),
        })),
    )
}

pub(crate) fn postprocess_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "scope": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_only": { "type": "boolean" },
                    "stage_filter": { "type": ["string", "null"] },
                    "changed_file_count": { "type": "integer" },
                    "changed_files": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["changed_only", "stage_filter", "changed_file_count", "changed_files"]
            },
            "dry_run": { "type": "boolean" },
            "planned_stages": { "type": "array", "items": { "$ref": "#/$defs/postprocess_stage" } },
            "executed_stages": { "type": "array", "items": { "$ref": "#/$defs/postprocess_stage" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "ok": { "type": "boolean" },
                    "noop": { "type": "boolean" },
                    "noop_reason": { "type": ["string", "null"] },
                    "error_code": { "type": "string" },
                    "error_code_docs": { "type": "string" },
                    "message": { "type": "string" },
                    "suggestions": { "type": "array", "items": { "type": "string" } },
                    "graph_built": { "type": "boolean" },
                    "state": { "type": "string" },
                    "started_at_ms": { "type": "integer" },
                    "finished_at_ms": { "type": "integer" },
                    "duration_ms": { "type": "integer" },
                    "stage_count": { "type": "integer" },
                    "supported_stage_count": { "type": "integer" }
                },
                "required": ["ok", "noop", "noop_reason", "error_code", "error_code_docs", "message", "suggestions", "graph_built", "state", "started_at_ms", "finished_at_ms", "duration_ms", "stage_count", "supported_stage_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "scope",
            "dry_run",
            "planned_stages",
            "executed_stages",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "postprocess_stage": postprocess_stage_schema(),
        })),
    )
}

pub(crate) fn status_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "graph_state": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "graph_built": { "type": "boolean" },
                    "build_state": { "type": ["string", "null"] },
                    "build_last_error": { "type": ["string", "null"] },
                    "build_budget_stop_reason": { "type": ["string", "null"] },
                    "stale_index": { "type": "boolean" },
                    "pending_graph_change_count": { "type": "integer" },
                    "pending_graph_changes": { "type": "array", "items": { "type": "string" } },
                    "execution_state": { "type": "string" },
                    "connection_mode": { "type": "string" },
                    "read_pool_active": { "type": "boolean" }
                },
                "required": ["graph_built", "build_state", "build_last_error", "build_budget_stop_reason", "stale_index", "pending_graph_change_count", "pending_graph_changes", "execution_state", "connection_mode", "read_pool_active"]
            },
            "db_state": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string" },
                    "exists": { "type": "boolean" },
                    "open_ok": { "type": "boolean" },
                    "open_error": { "type": ["string", "null"] },
                    "query_error": { "type": ["string", "null"] },
                    "build_status": { "type": ["object", "null"] }
                },
                "required": ["path", "exists", "open_ok", "open_error", "query_error", "build_status"]
            },
            "indexed_file_count": { "type": "integer" },
            "node_count": { "type": "integer" },
            "edge_count": { "type": "integer" },
            "last_indexed_at": { "type": ["string", "null"] },
            "failure_category": { "type": "string" },
            "ready": { "type": "boolean" },
            "safe_for_symbol_lookup": { "type": "boolean" },
            "safe_for_analysis": { "type": "boolean" },
            "retrieval_index": { "type": "object" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "message": { "type": "string" },
                    "suggestions": { "type": "array", "items": { "type": "string" } },
                    "error_code_docs": { "type": "string" }
                },
                "required": ["message", "suggestions", "error_code_docs"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "graph_state",
            "db_state",
            "indexed_file_count",
            "node_count",
            "edge_count",
            "last_indexed_at",
            "failure_category",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn doctor_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "overall_status": { "type": "string" },
            "checks": { "type": "array", "items": { "$ref": "#/$defs/doctor_check" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "total_count": { "type": "integer" },
                    "pass_count": { "type": "integer" },
                    "fail_count": { "type": "integer" },
                    "message": { "type": "string" },
                    "suggestions": { "type": "array", "items": { "type": "string" } },
                    "error_code_docs": { "type": "string" }
                },
                "required": ["total_count", "pass_count", "fail_count", "message", "suggestions", "error_code_docs"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "overall_status",
            "checks",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "doctor_check": doctor_check_schema(),
        })),
    )
}

pub(crate) fn db_check_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "ok": { "type": "boolean" },
            "integrity": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "ok": { "type": "boolean" },
                    "issues": { "type": "array", "items": { "type": "string" } },
                    "issue_count": { "type": "integer" }
                },
                "required": ["ok", "issues", "issue_count"]
            },
            "orphan_nodes": { "type": "array", "items": { "$ref": "#/$defs/orphan_node" } },
            "dangling_edges": { "type": "array", "items": { "$ref": "#/$defs/dangling_edge_diagnostic" } },
            "noncanonical_path_rows": { "type": "array", "items": { "type": "string" } },
            "session_db": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string" },
                    "exists": { "type": "boolean" },
                    "ok": { "type": "boolean" },
                    "memory_schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "ok": { "type": "boolean" },
                            "issues": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["ok", "issues"]
                    }
                },
                "required": ["path", "exists", "ok", "memory_schema"]
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "ok": { "type": "boolean" },
                    "failure_category": { "type": "string" },
                    "message": { "type": "string" },
                    "suggestions": { "type": "array", "items": { "type": "string" } },
                    "error_code_docs": { "type": "string" },
                    "orphan_node_count": { "type": "integer" },
                    "dangling_edge_count": { "type": "integer" },
                    "noncanonical_path_row_count": { "type": "integer" }
                },
                "required": ["ok", "failure_category", "message", "suggestions", "error_code_docs", "orphan_node_count", "dangling_edge_count", "noncanonical_path_row_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "db_path": { "type": "string" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "ok",
            "integrity",
            "orphan_nodes",
            "dangling_edges",
            "noncanonical_path_rows",
            "session_db",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "orphan_node": orphan_node_schema(),
            "dangling_edge_diagnostic": dangling_edge_diagnostic_schema(),
        })),
    )
}

pub(crate) fn debug_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "node_counts_by_kind": { "type": "array", "items": { "type": "array" } },
            "edge_counts_by_kind": { "type": "array", "items": { "type": "array" } },
            "top_files": { "type": "array", "items": { "$ref": "#/$defs/debug_top_file" } },
            "orphan_nodes": { "type": "array", "items": { "$ref": "#/$defs/orphan_node" } },
            "dangling_edges": { "type": "array", "items": { "$ref": "#/$defs/dangling_edge_diagnostic" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "node_count": { "type": "integer" },
                    "edge_count": { "type": "integer" },
                    "file_count": { "type": "integer" },
                    "top_file_count": { "type": "integer" },
                    "orphan_node_count": { "type": "integer" },
                    "dangling_edge_count": { "type": "integer" }
                },
                "required": ["node_count", "edge_count", "file_count", "top_file_count", "orphan_node_count", "dangling_edge_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "node_counts_by_kind",
            "edge_counts_by_kind",
            "top_files",
            "orphan_nodes",
            "dangling_edges",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "debug_top_file": debug_top_file_schema(),
            "orphan_node": orphan_node_schema(),
            "dangling_edge_diagnostic": dangling_edge_diagnostic_schema(),
        })),
    )
}

pub(crate) fn get_context_stats_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "session_id": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "event_count": { "type": "integer" },
            "source_count": { "type": "integer" },
            "chunk_count": { "type": "integer" },
            "bridge_file_count": { "type": "integer" },
            "content_db_path": { "type": "string" },
            "session_db_path": { "type": "string" },
            "bridge_dir_path": { "type": "string" },
            "retrieval_index": {
                "type": ["object", "null"],
                "additionalProperties": false,
                "properties": {
                    "state": { "type": "string" },
                    "files_discovered": { "type": "integer" },
                    "files_indexed": { "type": "integer" },
                    "chunks_written": { "type": "integer" },
                    "chunks_reused": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] },
                    "last_error": { "type": ["string", "null"] },
                    "updated_at": { "type": "string" },
                    "searchable": { "type": "boolean" }
                },
                "required": [
                    "state",
                    "files_discovered",
                    "files_indexed",
                    "chunks_written",
                    "chunks_reused",
                    "last_indexed_at",
                    "last_error",
                    "updated_at",
                    "searchable"
                ]
            }
        }),
        &[
            "session_id",
            "agent_id",
            "event_count",
            "source_count",
            "chunk_count",
            "bridge_file_count",
            "content_db_path",
            "session_db_path",
            "bridge_dir_path",
            "retrieval_index",
        ],
        None,
    )
}
