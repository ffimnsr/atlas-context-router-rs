//! JSON-Schema builders for the overview tool family (ICM-G).

use crate::descriptors::normalized_tool_output_schema;
use serde_json::{Value, json};

fn summary_schema(required: &[&str], properties: Value) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required
    })
}

fn warnings_schema() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

pub(crate) fn overview_export_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "result": { "type": "object" },
            "summary": summary_schema(
                &[
                    "packet_count",
                    "subject_count",
                    "omitted_subject_count",
                    "graph_freshness",
                    "safe_to_answer",
                    "budget_hit"
                ],
                json!({
                    "packet_count": { "type": "integer" },
                    "subject_count": { "type": "integer" },
                    "omitted_subject_count": { "type": "integer" },
                    "graph_freshness": { "type": "string" },
                    "safe_to_answer": { "type": "boolean" },
                    "budget_hit": { "type": "boolean" }
                })
            ),
            "warnings": warnings_schema()
        }),
        &["tool", "repo_root", "result", "summary", "warnings"],
        None,
    )
}

pub(crate) fn overview_ingest_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "record": { "type": "object" },
            "summary": summary_schema(
                &[
                    "overview_id",
                    "subject_id",
                    "status",
                    "subject_resolved",
                    "packet_validated",
                    "redacted",
                    "body_routed"
                ],
                json!({
                    "overview_id": { "type": "string" },
                    "subject_id": { "type": "string" },
                    "status": { "type": "string" },
                    "subject_resolved": { "type": "boolean" },
                    "packet_validated": { "type": "boolean" },
                    "redacted": { "type": "boolean" },
                    "body_routed": { "type": "boolean" }
                })
            ),
            "warnings": warnings_schema()
        }),
        &["tool", "repo_root", "record", "summary", "warnings"],
        None,
    )
}

pub(crate) fn overview_search_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "query": { "type": "string" },
            "level": { "type": "string" },
            "hits": { "type": "array", "items": { "type": "object" } },
            "summary": summary_schema(
                &["hit_count", "limit"],
                json!({
                    "hit_count": { "type": "integer" },
                    "limit": { "type": "integer" }
                })
            ),
            "warnings": warnings_schema()
        }),
        &["tool", "repo_root", "query", "hits", "summary", "warnings"],
        None,
    )
}

pub(crate) fn overview_show_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "found": { "type": "boolean" },
            "code": { "type": "string" },
            "subject": { "type": "string" },
            "level": { "type": "string" },
            "view": { "type": "object" },
            "record": { "type": ["object", "null"] },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": summary_schema(
                &[
                    "overview_id",
                    "subject_id",
                    "subject_kind",
                    "freshness",
                    "confidence"
                ],
                json!({
                    "overview_id": { "type": "string" },
                    "subject_id": { "type": "string" },
                    "subject_kind": { "type": "string" },
                    "freshness": { "type": "string" },
                    "confidence": { "type": "number" }
                })
            ),
            "warnings": warnings_schema()
        }),
        &["tool", "repo_root", "found", "matches", "warnings"],
        None,
    )
}

pub(crate) fn overview_refresh_plan_output_schema() -> Value {
    normalized_tool_output_schema(
        json!({
            "repo_root": { "type": "string" },
            "plan": { "type": "object" },
            "summary": summary_schema(
                &[
                    "changed_file_count",
                    "direct_stale_count",
                    "possibly_stale_count",
                    "orphaned_count",
                    "unchanged_count",
                    "apply",
                    "applied_count"
                ],
                json!({
                    "changed_file_count": { "type": "integer" },
                    "direct_stale_count": { "type": "integer" },
                    "possibly_stale_count": { "type": "integer" },
                    "orphaned_count": { "type": "integer" },
                    "unchanged_count": { "type": "integer" },
                    "apply": { "type": "boolean" },
                    "applied_count": { "type": "integer" }
                })
            ),
            "warnings": warnings_schema()
        }),
        &["tool", "repo_root", "plan", "summary", "warnings"],
        None,
    )
}
