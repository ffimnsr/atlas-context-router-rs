//! JSON-Schema builders for the `session` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn get_session_status_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "session_id": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "merged_agent_view": { "type": "boolean" },
            "status": { "type": "string" },
            "repo_root": { "type": ["string", "null"] },
            "frontend": { "type": ["string", "null"] },
            "worktree_id": { "type": ["string", "null"] },
            "created_at": { "type": ["string", "null"] },
            "updated_at": { "type": ["string", "null"] },
            "last_resume_at": { "type": ["string", "null"] },
            "last_compaction_at": { "type": ["string", "null"] },
            "event_count": { "type": "integer" },
            "resume_snapshot_exists": { "type": "boolean" },
            "snapshot_consumed": { "type": ["boolean", "null"] },
            "agent_partitions": { "type": "array", "items": { "type": "object" } },
            "delegated_tasks": { "type": "array", "items": { "type": "object" } },
            "agent_responsibilities": { "type": "array", "items": { "type": "object" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string" },
                    "has_session": { "type": "boolean" },
                    "event_count": { "type": "integer" },
                    "partition_count": { "type": "integer" },
                    "delegated_task_count": { "type": "integer" },
                    "responsibility_count": { "type": "integer" },
                    "resume_snapshot_exists": { "type": "boolean" }
                },
                "required": ["status", "has_session", "event_count", "partition_count", "delegated_task_count", "responsibility_count", "resume_snapshot_exists"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "session_id",
            "event_count",
            "resume_snapshot_exists",
            "last_compaction_at",
            "repo_root",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn record_session_event_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "tool": { "type": "string" },
            "event": { "type": "string" },
            "canonical_event": { "type": "string" },
            "frontend": { "type": "string" },
            "session_id": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "pending_resume": { "type": "boolean" },
            "stored": { "type": "boolean" },
            "event_id": { "type": ["integer", "null"] },
            "source_id": { "type": ["string", "null"] },
            "storage_kind": { "type": ["string", "null"] },
            "snapshot": { "type": ["object", "null"] },
            "actions": { "type": ["object", "null"] },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "event",
            "canonical_event",
            "frontend",
            "session_id",
            "pending_resume",
            "stored",
            "event_id",
            "source_id",
            "storage_kind",
            "snapshot",
            "actions",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn wake_up_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "tool": { "type": "string" },
            "repo_root": { "type": "string" },
            "session_id": { "type": "string" },
            "frontend": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "current_focus": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "intent": { "type": ["string", "null"] },
                    "reasoning": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "summary": { "type": ["string", "null"] },
                                "source_id": { "type": ["string", "null"] },
                                "at": { "type": ["string", "null"] }
                            },
                            "required": ["summary", "source_id", "at"]
                        }
                    }
                },
                "required": ["intent", "reasoning"]
            },
            "recent_decisions": { "type": "array", "items": { "type": "object" } },
            "critical_memories": { "type": "array", "items": { "type": "object" } },
            "recent_feedback": { "type": "array", "items": { "type": "object" } },
            "active_memoir_concepts": { "type": "array", "items": { "type": "string" } },
            "changed_files": { "type": "array", "items": { "type": "string" } },
            "graph_readiness": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "graph_built": { "type": "boolean" },
                    "graph_queryable": { "type": "boolean" },
                    "graph_current": { "type": "boolean" },
                    "stale_index": { "type": "boolean" },
                    "execution_state": { "type": "string" },
                    "pending_graph_change_count": { "type": "integer" },
                    "pending_graph_changes": { "type": "array", "items": { "type": "string" } },
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] },
                    "message": { "type": "string" }
                },
                "required": ["graph_built", "graph_queryable", "graph_current", "stale_index", "execution_state", "pending_graph_change_count", "pending_graph_changes", "indexed_file_count", "last_indexed_at", "message"]
            },
            "retrieval_hints": { "type": "array", "items": { "type": "object" } },
            "generated_at": { "type": "string" },
            "event_recorded": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "stored": { "type": "boolean" },
                    "event": { "type": ["string", "null"] },
                    "event_id": { "type": ["integer", "null"] },
                    "pending_resume": { "type": ["boolean", "null"] },
                    "lifecycle_status": { "type": ["string", "null"] },
                    "resume_loaded": { "type": ["boolean", "null"] },
                    "error": { "type": ["string", "null"] }
                },
                "required": ["stored"]
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string" },
                    "pending_resume": { "type": "boolean" },
                    "event_count": { "type": "integer" },
                    "decision_count": { "type": "integer" },
                    "critical_memory_count": { "type": "integer" },
                    "feedback_count": { "type": "integer" },
                    "concept_count": { "type": "integer" },
                    "changed_file_count": { "type": "integer" },
                    "retrieval_hint_count": { "type": "integer" },
                    "recorded": { "type": "string" }
                },
                "required": ["status", "pending_resume", "event_count", "decision_count", "critical_memory_count", "feedback_count", "concept_count", "changed_file_count", "retrieval_hint_count", "recorded"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "repo_root",
            "session_id",
            "frontend",
            "current_focus",
            "recent_decisions",
            "critical_memories",
            "recent_feedback",
            "active_memoir_concepts",
            "changed_files",
            "graph_readiness",
            "retrieval_hints",
            "generated_at",
            "event_recorded",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn compact_session_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "session_id": { "type": "string" },
            "before_counts": { "type": "object", "additionalProperties": false, "properties": { "events": { "type": "integer" } }, "required": ["events"] },
            "after_counts": { "type": "object", "additionalProperties": false, "properties": { "events": { "type": "integer" } }, "required": ["events"] },
            "promoted_events": { "type": "integer" },
            "removed_events": { "type": "integer" },
            "merged_groups": { "type": "integer" },
            "decayed_events": { "type": "integer" },
            "deduplicated_events": { "type": "integer" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string" },
                    "no_op": { "type": "boolean" },
                    "events_before": { "type": "integer" },
                    "events_after": { "type": "integer" },
                    "events_removed": { "type": "integer" }
                },
                "required": ["status", "no_op", "events_before", "events_after", "events_removed"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "session_id",
            "before_counts",
            "after_counts",
            "promoted_events",
            "removed_events",
            "merged_groups",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn resume_session_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "session_id": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "merged_agent_view": { "type": "boolean" },
            "snapshot_status": { "type": "string" },
            "snapshot": { "type": "object" },
            "event_count": { "type": "integer" },
            "consumed": { "type": "boolean" },
            "created_at": { "type": ["string", "null"] },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "event_count": { "type": "integer" },
                    "merged_agent_view": { "type": "boolean" },
                    "snapshot_consumed": { "type": "boolean" }
                },
                "required": ["event_count", "merged_agent_view", "snapshot_consumed"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "session_id",
            "snapshot_status",
            "snapshot",
            "consumed",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}
