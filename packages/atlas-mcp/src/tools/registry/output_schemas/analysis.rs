//! JSON-Schema builders for the `analysis` tool family.
//!
//! Helpers are reachable through the umbrella re-exports in
//! `super` (`output_schemas/mod.rs`) when cross-file sharing is needed.

use crate::descriptors::normalized_tool_output_schema;
use serde_json::Value;

pub(crate) fn insight_severity_schema() -> Value {
    serde_json::json!({
        "type": "string",
        "enum": ["info", "low", "medium", "high"]
    })
}

pub(crate) fn confidence_tier_schema() -> Value {
    serde_json::json!({
        "type": "string",
        "enum": ["low", "medium", "high"]
    })
}

pub(crate) fn insight_line_range_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "start_line": { "type": "integer" },
            "end_line": { "type": "integer" }
        },
        "required": ["start_line", "end_line"]
    })
}

pub(crate) fn insight_evidence_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "file_path": { "type": "string" },
            "qualified_name": { "type": "string" },
            "node_kind": { "type": "string" },
            "edge_kind": { "type": "string" },
            "line_range": { "$ref": "#/$defs/insight_line_range" },
            "confidence_tier": { "$ref": "#/$defs/confidence_tier" }
        }
    })
}

pub(crate) fn insight_finding_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "type": "string" },
            "title": { "type": "string" },
            "severity": { "$ref": "#/$defs/insight_severity" },
            "category": { "type": "string" },
            "message": { "type": "string" },
            "evidence": { "type": "array", "items": { "$ref": "#/$defs/insight_evidence" } },
            "ranking_reason": { "type": "string" },
            "details": true,
            "score": { "type": "number" }
        },
        "required": ["id", "title", "severity", "category", "message", "ranking_reason", "score"]
    })
}

pub(crate) fn insight_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "total_findings": { "type": "integer" },
            "highest_severity": { "$ref": "#/$defs/insight_severity" },
            "generated_at": { "type": "string" }
        },
        "required": ["total_findings", "generated_at"]
    })
}

pub(crate) fn graph_freshness_warning_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "stale": { "type": "boolean" },
            "changed_files": { "type": "array", "items": { "type": "string" } },
            "stale_result_files": { "type": "array", "items": { "type": "string" } },
            "warning": { "type": "string" },
            "suggested_recovery": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["stale", "changed_files", "stale_result_files", "warning", "suggested_recovery"]
    })
}

pub(crate) fn insight_report_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "summary": { "$ref": "#/$defs/insight_summary" },
            "findings": { "type": "array", "items": { "$ref": "#/$defs/insight_finding" } },
            "atlas_provenance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] }
                },
                "required": ["indexed_file_count"]
            },
            "atlas_freshness": { "$ref": "#/$defs/graph_freshness_warning" }
        }),
        &["summary", "findings", "atlas_provenance"],
        Some(serde_json::json!({
            "insight_severity": insight_severity_schema(),
            "confidence_tier": confidence_tier_schema(),
            "insight_line_range": insight_line_range_schema(),
            "insight_evidence": insight_evidence_schema(),
            "insight_finding": insight_finding_schema(),
            "insight_summary": insight_summary_schema(),
            "graph_freshness_warning": graph_freshness_warning_schema()
        })),
    )
}

pub(crate) fn large_function_report_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string", "enum": ["large", "complex", "large-or-complex"] },
            "summary": { "$ref": "#/$defs/insight_summary" },
            "findings": { "type": "array", "items": { "$ref": "#/$defs/insight_finding" } },
            "atlas_provenance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] }
                },
                "required": ["indexed_file_count"]
            },
            "atlas_freshness": { "$ref": "#/$defs/graph_freshness_warning" }
        }),
        &["mode", "summary", "findings", "atlas_provenance"],
        Some(serde_json::json!({
            "insight_severity": insight_severity_schema(),
            "confidence_tier": confidence_tier_schema(),
            "insight_line_range": insight_line_range_schema(),
            "insight_evidence": insight_evidence_schema(),
            "insight_finding": insight_finding_schema(),
            "insight_summary": insight_summary_schema(),
            "graph_freshness_warning": graph_freshness_warning_schema()
        })),
    )
}

pub(crate) fn similar_function_report_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "source": { "type": "object" },
            "thresholds": { "type": "object" },
            "summary": { "$ref": "#/$defs/insight_summary" },
            "findings": { "type": "array", "items": { "$ref": "#/$defs/insight_finding" } },
            "matches": { "type": "array", "items": { "type": "object" } },
            "atlas_provenance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] }
                },
                "required": ["indexed_file_count"]
            },
            "atlas_freshness": { "$ref": "#/$defs/graph_freshness_warning" }
        }),
        &[
            "source",
            "thresholds",
            "summary",
            "findings",
            "matches",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "insight_severity": insight_severity_schema(),
            "confidence_tier": confidence_tier_schema(),
            "insight_line_range": insight_line_range_schema(),
            "insight_evidence": insight_evidence_schema(),
            "insight_finding": insight_finding_schema(),
            "insight_summary": insight_summary_schema(),
            "graph_freshness_warning": graph_freshness_warning_schema()
        })),
    )
}

pub(crate) fn duplicate_report_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "thresholds": { "type": "object" },
            "summary": { "$ref": "#/$defs/insight_summary" },
            "findings": { "type": "array", "items": { "$ref": "#/$defs/insight_finding" } },
            "groups": { "type": "array", "items": { "type": "object" } },
            "atlas_provenance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] }
                },
                "required": ["indexed_file_count"]
            },
            "atlas_freshness": { "$ref": "#/$defs/graph_freshness_warning" }
        }),
        &[
            "thresholds",
            "summary",
            "findings",
            "groups",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "insight_severity": insight_severity_schema(),
            "confidence_tier": confidence_tier_schema(),
            "insight_line_range": insight_line_range_schema(),
            "insight_evidence": insight_evidence_schema(),
            "insight_finding": insight_finding_schema(),
            "insight_summary": insight_summary_schema(),
            "graph_freshness_warning": graph_freshness_warning_schema()
        })),
    )
}

pub(crate) fn inferred_module_report_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "summary": { "$ref": "#/$defs/insight_summary" },
            "findings": { "type": "array", "items": { "$ref": "#/$defs/insight_finding" } },
            "modules": { "type": "array", "items": { "type": "object" } },
            "atlas_provenance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] }
                },
                "required": ["indexed_file_count"]
            },
            "atlas_freshness": { "$ref": "#/$defs/graph_freshness_warning" }
        }),
        &["summary", "findings", "modules", "atlas_provenance"],
        Some(serde_json::json!({
            "insight_severity": insight_severity_schema(),
            "confidence_tier": confidence_tier_schema(),
            "insight_line_range": insight_line_range_schema(),
            "insight_evidence": insight_evidence_schema(),
            "insight_finding": insight_finding_schema(),
            "insight_summary": insight_summary_schema(),
            "graph_freshness_warning": graph_freshness_warning_schema()
        })),
    )
}

pub(crate) fn component_label_report_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "summary": { "$ref": "#/$defs/insight_summary" },
            "findings": { "type": "array", "items": { "$ref": "#/$defs/insight_finding" } },
            "assignments": { "type": "array", "items": { "type": "object" } },
            "atlas_provenance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] }
                },
                "required": ["indexed_file_count"]
            },
            "atlas_freshness": { "$ref": "#/$defs/graph_freshness_warning" }
        }),
        &["summary", "findings", "assignments", "atlas_provenance"],
        Some(serde_json::json!({
            "insight_severity": insight_severity_schema(),
            "confidence_tier": confidence_tier_schema(),
            "insight_line_range": insight_line_range_schema(),
            "insight_evidence": insight_evidence_schema(),
            "insight_finding": insight_finding_schema(),
            "insight_summary": insight_summary_schema(),
            "graph_freshness_warning": graph_freshness_warning_schema()
        })),
    )
}

pub(crate) fn analyze_safety_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "symbol": { "type": "object" },
            "fan_in": { "type": "integer" },
            "fan_out": { "type": "integer" },
            "test_adjacency": { "type": "object" },
            "cross_module_callers": { "type": "integer" },
            "safety_score": { "type": "number" },
            "safety_band": { "type": "string" },
            "suggested_validations": { "type": "array", "items": { "type": "string" } },
            "factor_evidence": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "symbol",
            "fan_in",
            "fan_out",
            "test_adjacency",
            "cross_module_callers",
            "safety_score",
            "safety_band",
            "suggested_validations",
            "factor_evidence",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn analyze_remove_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "symbols": { "type": "array", "items": { "type": "object" } },
            "definite_impacts": { "type": "array", "items": { "type": "object" } },
            "probable_impacts": { "type": "array", "items": { "type": "object" } },
            "weak_impacts": { "type": "array", "items": { "type": "object" } },
            "tests": { "type": "array", "items": { "type": "object" } },
            "uncertainty_flags": { "type": "array", "items": { "type": "string" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "object" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "symbols",
            "definite_impacts",
            "probable_impacts",
            "weak_impacts",
            "tests",
            "uncertainty_flags",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn analyze_dead_code_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "scope": { "type": "object" },
            "candidates": { "type": "array", "items": { "type": "object" } },
            "blockers": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "scope",
            "candidates",
            "blockers",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

pub(crate) fn analyze_dependency_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "symbol": { "type": "string" },
            "removable": { "type": "boolean" },
            "blocking_references": { "type": "array", "items": { "type": "object" } },
            "confidence_tier": { "type": "string" },
            "suggested_cleanups": { "type": "array", "items": { "type": "string" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "symbol",
            "removable",
            "blocking_references",
            "confidence_tier",
            "suggested_cleanups",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}
