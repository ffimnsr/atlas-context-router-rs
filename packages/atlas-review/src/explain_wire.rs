//! Shared `explain_change` wire payload.
//!
//! The CLI emitted `changed_by_kind` at the top level while MCP emitted
//! `change_kinds` plus `coverage_gaps`. This module adds the MCP key names to
//! the CLI payload so both surfaces expose the same identity fields, while
//! keeping the legacy CLI counts for existing consumers.

use serde_json::{Value, json};

use crate::explain_change::ExplainChangeSummary;

/// Serialize one explain-change summary into the shared wire shape.
///
/// Adds `changed_files`, `change_kinds`, and `coverage_gaps` alongside the
/// existing summary fields. `change_kinds` is an alias of `changed_by_kind`.
pub fn explain_change_wire_payload(summary: &ExplainChangeSummary) -> Value {
    let mut payload = serde_json::to_value(summary).unwrap_or(Value::Null);
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };

    object.insert(
        "changed_files".to_owned(),
        json!(summary.diff_summary.files),
    );
    let changed_by_kind = object
        .get("changed_by_kind")
        .cloned()
        .unwrap_or_else(|| json!({}));
    object.insert("change_kinds".to_owned(), changed_by_kind);
    object.insert(
        "coverage_gaps".to_owned(),
        Value::Array(
            summary
                .test_impact
                .uncovered_symbols
                .iter()
                .map(|symbol| json!({ "symbol": symbol }))
                .collect(),
        ),
    );
    payload
}
