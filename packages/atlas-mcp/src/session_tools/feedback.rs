//! ICM-C2 — MCP `feedback_record` tool.
//!
//! Mirrors `atlas feedback record` through the shared feedback service layer
//! in `atlas-session` so CLI and MCP cannot drift on validation or record
//! shape.

use anyhow::Result;
use atlas_adapters::derive_session_db_path;
use atlas_session::{NewFeedback, SessionStore};
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::normalized_tool_result_value as build_normalized_tool_result_value;

/// Record a correction when an analysis prediction was wrong.
pub fn tool_feedback_record(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let predicted = args
        .and_then(|a| a.get("predicted"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: predicted"))?;
    let actual = args
        .and_then(|a| a.get("actual"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: actual"))?;
    let correction = args
        .and_then(|a| a.get("correction"))
        .and_then(|v| v.as_str());
    let tool = args.and_then(|a| a.get("tool")).and_then(|v| v.as_str());
    let analysis_kind = args
        .and_then(|a| a.get("analysis_kind"))
        .and_then(|v| v.as_str());
    let symbol = args.and_then(|a| a.get("symbol")).and_then(|v| v.as_str());
    let file = args.and_then(|a| a.get("file")).and_then(|v| v.as_str());
    let source_id = args
        .and_then(|a| a.get("source_id"))
        .and_then(|v| v.as_str());

    // Same validation contract as `atlas feedback record`.
    let input = NewFeedback {
        repo_root: repo_root.to_owned(),
        session_id: None,
        tool_name: tool.unwrap_or("mcp").to_owned(),
        analysis_kind: analysis_kind.unwrap_or_default().to_owned(),
        predicted: predicted.to_owned(),
        actual: actual.to_owned(),
        correction: correction.unwrap_or_default().to_owned(),
        related_symbol: symbol.map(str::to_owned),
        related_file: file.map(str::to_owned),
        source_id: source_id.map(str::to_owned),
        metadata: serde_json::json!({}),
    };
    input.validate()?;

    let session_db = derive_session_db_path(db_path);
    let mut store = SessionStore::open(&session_db)?;
    let record = store.store_feedback(&input)?;

    build_normalized_tool_result_value(
        &serde_json::json!({
            "tool": "feedback_record",
            "repo_root": repo_root,
            "record": record,
            "summary": {
                "record_id": record.id,
                "analysis_kind": record.analysis_kind,
                "is_false_positive_evidence": record.is_false_positive_evidence(),
            },
            "warnings": [],
        }),
        output_format,
    )
}
