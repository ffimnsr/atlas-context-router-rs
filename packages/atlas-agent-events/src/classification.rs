//! Deterministic, local classification of bounded runtime output.
//!
//! The classifier never stores raw matched text. It reports the stable rule id
//! that matched and the field names that produced the match, so resume and
//! wake-up surfaces can group failures without replaying command output.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::payload::{extract_hook_command, extract_hook_status, extract_tool_name};
use crate::policy::{MAX_CLASSIFIER_INPUT_BYTES, MAX_CLASSIFIER_MATCHED_FIELDS};

/// Stable runtime classification kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeClassificationKind {
    Panic,
    Exception,
    Stacktrace,
    CompilerError,
    TestFailure,
    TestSuccess,
    BuildSuccess,
    DeprecationWarning,
    DeadCodeWarning,
    PermissionDenied,
    CommandTimeout,
    GraphNotReady,
    RetrievalFailure,
}

/// Severity assigned to a runtime classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeClassificationSeverity {
    Error,
    Warning,
    Info,
}

/// Classification metadata attached to a hook event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeClassification {
    pub kind: RuntimeClassificationKind,
    pub severity: RuntimeClassificationSeverity,
    pub rule_id: String,
    pub matched_fields: Vec<String>,
}

impl RuntimeClassification {
    fn new(
        kind: RuntimeClassificationKind,
        severity: RuntimeClassificationSeverity,
        rule_id: &'static str,
        matched_fields: Vec<String>,
    ) -> Self {
        Self {
            kind,
            severity,
            rule_id: rule_id.to_owned(),
            matched_fields: normalize_fields(matched_fields),
        }
    }
}

const INPUT_FIELDS: &[&str] = &[
    "stderr", "stdout", "output", "error", "message", "result", "extra",
];

/// Field name plus bounded lowercase text captured for one input field.
#[derive(Debug, Clone)]
struct InputText {
    field: String,
    text: String,
}

/// Classify one bounded runtime payload.
///
/// Returns `None` when no stable rule matches. There is deliberately no
/// `other` fallback: unclassified output is simply not annotated.
pub fn classify_runtime_payload(payload: &Value) -> Option<RuntimeClassification> {
    let status = extract_hook_status(payload).unwrap_or_default();
    let tool = extract_tool_name(payload)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let command = extract_hook_command(payload).unwrap_or_else(|| tool.clone());
    let command = command.to_ascii_lowercase();
    let inputs = collect_inputs(payload);

    if let Some(classification) = classify_status(&status, &inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_panic(&inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_exception(&inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_stacktrace(&inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_compiler_error(&inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_test_failure(&inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_test_success(&status, &command, &inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_build_success(&status, &command, &inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_deprecation_warning(&inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_dead_code_warning(&inputs) {
        return Some(classification);
    }
    if let Some(classification) = classify_graph_not_ready(&inputs) {
        return Some(classification);
    }
    classify_retrieval_failure(&inputs)
}

fn classify_status(status: &str, inputs: &[InputText]) -> Option<RuntimeClassification> {
    let permission_markers = [
        "permission_denied",
        "permission denied",
        "permissiondenied",
        "operation not permitted",
        "access is denied",
        "eacces",
        "eperm",
    ];
    if status.contains("permission_denied") || status.contains("permission denied") {
        return Some(RuntimeClassification::new(
            RuntimeClassificationKind::PermissionDenied,
            RuntimeClassificationSeverity::Error,
            "status_permission_denied",
            vec!["status".to_owned()],
        ));
    }
    let fields = matching_fields(inputs, &permission_markers);
    if !fields.is_empty() {
        return Some(RuntimeClassification::new(
            RuntimeClassificationKind::PermissionDenied,
            RuntimeClassificationSeverity::Error,
            "text_permission_denied",
            fields,
        ));
    }

    if status.contains("timeout") || status.contains("timed_out") {
        return Some(RuntimeClassification::new(
            RuntimeClassificationKind::CommandTimeout,
            RuntimeClassificationSeverity::Error,
            "status_command_timeout",
            vec!["status".to_owned()],
        ));
    }
    let timeout_markers = [
        "command timed out",
        "timed out",
        "timeout expired",
        "deadline exceeded",
        "operation timed out",
        "etimedout",
    ];
    let fields = matching_fields(inputs, &timeout_markers);
    if !fields.is_empty() {
        return Some(RuntimeClassification::new(
            RuntimeClassificationKind::CommandTimeout,
            RuntimeClassificationSeverity::Error,
            "text_command_timeout",
            fields,
        ));
    }
    None
}

fn classify_panic(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let fields = matching_fields(inputs, &["panicked at", "rust_backtrace"]);
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::Panic,
            RuntimeClassificationSeverity::Error,
            "text_panic",
            fields,
        )
    })
}

fn classify_exception(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let fields = matching_fields(
        inputs,
        &[
            "traceback (most recent call last)",
            "exception in thread",
            "unhandled exception",
            "uncaught exception",
            "exception:",
        ],
    );
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::Exception,
            RuntimeClassificationSeverity::Error,
            "text_exception",
            fields,
        )
    })
}

fn classify_stacktrace(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let fields = matching_fields(inputs, &["stack backtrace:", "\n    at ", "\n\tat "]);
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::Stacktrace,
            RuntimeClassificationSeverity::Error,
            "text_stacktrace",
            fields,
        )
    })
}

fn classify_compiler_error(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let fields = matching_fields(
        inputs,
        &[
            "error[e",
            "failed to compile",
            "could not compile",
            "error:",
            "\n--> ",
            " --> ",
        ],
    );
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::CompilerError,
            RuntimeClassificationSeverity::Error,
            "text_compiler_error",
            fields,
        )
    })
}

fn classify_test_failure(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let fields = matching_fields(
        inputs,
        &[
            "test result: failed",
            "failed tests/",
            "\nfailures:",
            "error: test failed",
            "test failed",
        ],
    );
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::TestFailure,
            RuntimeClassificationSeverity::Error,
            "text_test_failure",
            fields,
        )
    })
}

fn classify_test_success(
    status: &str,
    command: &str,
    inputs: &[InputText],
) -> Option<RuntimeClassification> {
    if command.contains("test") || command.contains("nextest") {
        if status == "ok" || status == "success" || status == "succeeded" {
            return Some(RuntimeClassification::new(
                RuntimeClassificationKind::TestSuccess,
                RuntimeClassificationSeverity::Info,
                "status_test_success",
                vec!["status".to_owned(), "command".to_owned()],
            ));
        }
        let fields = matching_fields(inputs, &["test result: ok", "all tests passed", "passed"]);
        if !fields.is_empty() {
            return Some(RuntimeClassification::new(
                RuntimeClassificationKind::TestSuccess,
                RuntimeClassificationSeverity::Info,
                "text_test_success",
                fields,
            ));
        }
    }
    let fields = matching_fields(inputs, &["test result: ok", "all tests passed"]);
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::TestSuccess,
            RuntimeClassificationSeverity::Info,
            "text_test_success",
            fields,
        )
    })
}

fn classify_build_success(
    status: &str,
    command: &str,
    inputs: &[InputText],
) -> Option<RuntimeClassification> {
    let build_command = command.contains("build")
        || command.contains("check")
        || command.contains("clippy")
        || command.contains("compile");
    if build_command && (status == "ok" || status == "success" || status == "succeeded") {
        return Some(RuntimeClassification::new(
            RuntimeClassificationKind::BuildSuccess,
            RuntimeClassificationSeverity::Info,
            "status_build_success",
            vec!["status".to_owned(), "command".to_owned()],
        ));
    }
    let fields = matching_fields(inputs, &["finished `", "finished in", "build succeeded"]);
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::BuildSuccess,
            RuntimeClassificationSeverity::Info,
            "text_build_success",
            fields,
        )
    })
}

fn classify_deprecation_warning(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let fields = matching_fields(inputs, &["deprecation", "deprecated"]);
    if fields.is_empty() {
        return None;
    }
    Some(RuntimeClassification::new(
        RuntimeClassificationKind::DeprecationWarning,
        RuntimeClassificationSeverity::Warning,
        "text_deprecation_warning",
        fields,
    ))
}

fn classify_dead_code_warning(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let has_unused = inputs.iter().any(|input| {
        input.text.contains("warning:")
            && (input.text.contains("unused")
                || input.text.contains("never used")
                || input.text.contains("dead_code"))
    });
    if !has_unused {
        return None;
    }
    let fields = matching_fields(inputs, &["unused", "never used", "dead_code"]);
    Some(RuntimeClassification::new(
        RuntimeClassificationKind::DeadCodeWarning,
        RuntimeClassificationSeverity::Warning,
        "text_dead_code_warning",
        fields,
    ))
}

fn classify_graph_not_ready(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let fields = matching_fields(
        inputs,
        &[
            "graph_stale",
            "graph is stale",
            "graph_unavailable",
            "graph_not_ready",
            "graph_not_built",
            "graph_partial",
            "graph_corrupt",
            "graph_missing",
        ],
    );
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::GraphNotReady,
            RuntimeClassificationSeverity::Error,
            "text_graph_not_ready",
            fields,
        )
    })
}

fn classify_retrieval_failure(inputs: &[InputText]) -> Option<RuntimeClassification> {
    let fields = matching_fields(
        inputs,
        &[
            "retrieval_index_unavailable",
            "retrieval index unavailable",
            "content_store_unavailable",
            "content store unavailable",
            "retrieval_failure",
        ],
    );
    (!fields.is_empty()).then(|| {
        RuntimeClassification::new(
            RuntimeClassificationKind::RetrievalFailure,
            RuntimeClassificationSeverity::Error,
            "text_retrieval_failure",
            fields,
        )
    })
}

fn matching_fields(inputs: &[InputText], needles: &[&str]) -> Vec<String> {
    let mut fields = Vec::new();
    for input in inputs {
        if needles.iter().any(|needle| input.text.contains(needle))
            && !fields.iter().any(|field| field == &input.field)
        {
            fields.push(input.field.clone());
        }
    }
    normalize_fields(fields)
}

fn normalize_fields(mut fields: Vec<String>) -> Vec<String> {
    fields.retain(|field| !field.trim().is_empty());
    fields.sort();
    fields.dedup();
    fields.truncate(MAX_CLASSIFIER_MATCHED_FIELDS);
    fields
}

fn collect_inputs(payload: &Value) -> Vec<InputText> {
    let mut inputs = Vec::new();
    let mut remaining = MAX_CLASSIFIER_INPUT_BYTES;
    collect_value_inputs(payload, None, &mut inputs, &mut remaining);
    inputs
}

fn collect_value_inputs(
    value: &Value,
    inherited_field: Option<&str>,
    inputs: &mut Vec<InputText>,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                if let Some(field) = inherited_field {
                    collect_text(nested, field, inputs, remaining);
                } else if INPUT_FIELDS.contains(&key.as_str()) {
                    collect_text(nested, key, inputs, remaining);
                } else {
                    collect_value_inputs(nested, None, inputs, remaining);
                }
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_value_inputs(nested, inherited_field, inputs, remaining);
            }
        }
        Value::String(_) | Value::Number(_) | Value::Bool(_) => {
            if let Some(field) = inherited_field {
                collect_text(value, field, inputs, remaining);
            }
        }
        Value::Null => {}
    }
}

fn collect_text(value: &Value, field: &str, inputs: &mut Vec<InputText>, remaining: &mut usize) {
    match value {
        Value::String(text) => {
            if *remaining == 0 {
                return;
            }
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            let text = truncate_to_budget(trimmed, remaining);
            if text.is_empty() {
                return;
            }
            inputs.push(InputText {
                field: field.to_owned(),
                text: text.to_ascii_lowercase(),
            });
        }
        Value::Array(values) => {
            for nested in values {
                collect_text(nested, field, inputs, remaining);
            }
        }
        Value::Object(map) => {
            for nested in map.values() {
                collect_text(nested, field, inputs, remaining);
            }
        }
        Value::Number(_) | Value::Bool(_) => {
            if *remaining == 0 {
                return;
            }
            let text = value.to_string();
            let text = truncate_to_budget(&text, remaining);
            if !text.is_empty() {
                inputs.push(InputText {
                    field: field.to_owned(),
                    text: text.to_ascii_lowercase(),
                });
            }
        }
        Value::Null => {}
    }
}

fn truncate_to_budget(text: &str, remaining: &mut usize) -> String {
    if *remaining == 0 {
        return String::new();
    }
    let bytes = text.as_bytes();
    let take = bytes.len().min(*remaining);
    let boundary = (0..=take)
        .rev()
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0);
    *remaining -= boundary;
    text[..boundary].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn classify(payload: Value) -> RuntimeClassification {
        classify_runtime_payload(&payload).expect("expected a classification")
    }

    #[test]
    fn classifies_panic() {
        let classification = classify(json!({
            "stderr": "thread 'main' panicked at src/main.rs:4:5"
        }));
        assert_eq!(classification.kind, RuntimeClassificationKind::Panic);
        assert_eq!(
            classification.severity,
            RuntimeClassificationSeverity::Error
        );
        assert_eq!(classification.matched_fields, vec!["stderr"]);
    }

    #[test]
    fn classifies_exception_and_stacktrace() {
        let exception = classify(json!({
            "stderr": "Traceback (most recent call last):\n  File \"x.py\", line 1"
        }));
        assert_eq!(exception.kind, RuntimeClassificationKind::Exception);
        assert_eq!(exception.matched_fields, vec!["stderr"]);

        let stacktrace = classify(json!({
            "output": "stack backtrace:\n    at foo.rs:1"
        }));
        assert_eq!(stacktrace.kind, RuntimeClassificationKind::Stacktrace);
    }

    #[test]
    fn classifies_compiler_error() {
        let classification = classify(json!({
            "stderr": "error[E0308]: mismatched types\n --> src/lib.rs:2:3"
        }));
        assert_eq!(
            classification.kind,
            RuntimeClassificationKind::CompilerError
        );
        assert_eq!(
            classification.severity,
            RuntimeClassificationSeverity::Error
        );
    }

    #[test]
    fn classifies_test_failure_and_success() {
        let failure = classify(json!({
            "stderr": "test result: FAILED. 1 passed; 2 failed"
        }));
        assert_eq!(failure.kind, RuntimeClassificationKind::TestFailure);

        let success = classify(json!({
            "command": "cargo test",
            "status": "ok"
        }));
        assert_eq!(success.kind, RuntimeClassificationKind::TestSuccess);
        assert_eq!(success.severity, RuntimeClassificationSeverity::Info);
        assert_eq!(success.matched_fields, vec!["command", "status"]);
    }

    #[test]
    fn classifies_build_success() {
        let classification = classify(json!({
            "command": "cargo build",
            "status": "ok"
        }));
        assert_eq!(classification.kind, RuntimeClassificationKind::BuildSuccess);
        assert_eq!(classification.severity, RuntimeClassificationSeverity::Info);
    }

    #[test]
    fn classifies_permission_denied_before_output_patterns() {
        let classification = classify(json!({
            "status": "permission_denied",
            "stderr": "error[E0308]: mismatched types"
        }));
        assert_eq!(
            classification.kind,
            RuntimeClassificationKind::PermissionDenied
        );
        assert_eq!(classification.matched_fields, vec!["status"]);
    }

    #[test]
    fn classifies_command_timeout() {
        let classification = classify(json!({
            "error": "command timed out after 30s"
        }));
        assert_eq!(
            classification.kind,
            RuntimeClassificationKind::CommandTimeout
        );
        assert_eq!(classification.matched_fields, vec!["error"]);
    }

    #[test]
    fn classifies_warnings() {
        let deprecation = classify(json!({
            "stderr": "warning: use of deprecated function"
        }));
        assert_eq!(
            deprecation.kind,
            RuntimeClassificationKind::DeprecationWarning
        );
        assert_eq!(deprecation.severity, RuntimeClassificationSeverity::Warning);

        let dead_code = classify(json!({
            "stderr": "warning: function `f` is never used"
        }));
        assert_eq!(dead_code.kind, RuntimeClassificationKind::DeadCodeWarning);
    }

    #[test]
    fn classifies_graph_and_retrieval_failures() {
        let graph = classify(json!({
            "error": "graph_stale: run atlas update"
        }));
        assert_eq!(graph.kind, RuntimeClassificationKind::GraphNotReady);

        let retrieval = classify(json!({
            "message": "retrieval_index_unavailable"
        }));
        assert_eq!(retrieval.kind, RuntimeClassificationKind::RetrievalFailure);
    }

    #[test]
    fn unmatched_payload_returns_none() {
        assert!(classify_runtime_payload(&json!({"stdout": "all good"})).is_none());
        assert!(classify_runtime_payload(&json!({})).is_none());
    }

    #[test]
    fn input_bytes_are_bounded() {
        let huge = "x".repeat(MAX_CLASSIFIER_INPUT_BYTES * 2);
        let payload = json!({"stderr": huge, "unused": "panicked at src/lib.rs:1"});
        assert!(classify_runtime_payload(&payload).is_none());
    }

    #[test]
    fn output_never_contains_raw_text() {
        let output = "panicked at secret-token-123";
        let classification = classify(json!({"stderr": output}));
        let encoded = serde_json::to_string(&classification).unwrap();
        assert!(!encoded.contains("secret-token-123"));
        assert!(!encoded.contains("panicked"));
        assert!(encoded.contains("stderr"));
    }

    #[test]
    fn matched_fields_are_deduplicated_and_clamped() {
        let classification = classify(json!({
            "stderr": "error: boom",
            "stdout": "error: boom",
            "output": "error: boom",
            "error": "error: boom",
            "message": "error: boom",
            "result": "error: boom",
            "extra": "error: boom"
        }));
        assert!(classification.matched_fields.len() <= MAX_CLASSIFIER_MATCHED_FIELDS);
        let mut sorted = classification.matched_fields.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, classification.matched_fields);
    }
}
