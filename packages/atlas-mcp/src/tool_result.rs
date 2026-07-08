use anyhow::Result;
use atlas_core::user_facing_error_message;
use serde::Serialize;
use serde_json::{Value, json};

use crate::output::{OutputFormat, render_value};

const MAX_TOOL_ERROR_TEXT_LEN: usize = 240;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceLink {
    #[serde(rename = "type")]
    pub(crate) item_type: &'static str,
    pub(crate) uri: String,
    pub(crate) name: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mime_type: Option<String>,
}

pub(crate) struct ToolResultBuilder {
    output_format: OutputFormat,
}

impl ToolResultBuilder {
    pub(crate) fn new(output_format: OutputFormat) -> Self {
        Self { output_format }
    }

    pub(crate) fn build_serializable<T: Serialize>(&self, value: &T) -> Result<Value> {
        let raw = serde_json::to_value(value)?;
        self.build_value(raw)
    }

    pub(crate) fn build_value(&self, raw: Value) -> Result<Value> {
        let rendered = render_value(&raw, self.output_format)?;
        let mut content = vec![json!({
            "type": "text",
            "text": rendered.text,
            "mimeType": rendered.actual_format.mime_type(),
        })];
        let resource_links = infer_resource_links(&raw);
        for link in resource_links {
            content.push(serde_json::to_value(link)?);
        }

        let mut response = json!({
            "content": content,
            "_meta": result_meta(
                rendered.actual_format.as_str(),
                rendered.requested_format.as_str(),
                rendered.fallback_reason.as_deref(),
            ),
        });

        if raw.is_object() {
            response["structuredContent"] = raw;
        }

        Ok(response)
    }
}

fn result_meta(
    actual_output_format: &str,
    requested_output_format: &str,
    fallback_reason: Option<&str>,
) -> Value {
    let mut meta = json!({
        "atlas:outputFormat": actual_output_format,
        "atlas:requestedOutputFormat": requested_output_format,
    });
    if let Some(reason) = fallback_reason {
        meta["atlas:fallbackReason"] = Value::String(reason.to_owned());
    }
    meta
}

pub(crate) fn tool_result_value<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<Value> {
    ToolResultBuilder::new(output_format).build_serializable(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolErrorCode {
    InvalidInput,
    FileNotFound,
    SymbolNotFound,
    GraphStale,
    Timeout,
    DependencyFailed,
    InternalToolError,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InputShapeErrorSpec {
    pub(crate) offending_fields: Vec<String>,
    pub(crate) normalization_performed: Vec<String>,
    pub(crate) accepted_argument_families: Vec<String>,
    pub(crate) retry_example: Option<Value>,
    pub(crate) fail_closed_reason: Option<String>,
    pub(crate) retry_guidance: Option<String>,
    pub(crate) extra_details: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ToolErrorPayload {
    pub(crate) code: ToolErrorCode,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_guidance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) details: Option<Value>,
}

impl ToolErrorPayload {
    pub(crate) fn new(code: ToolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: normalize_tool_error_text(message.into()),
            retry_guidance: None,
            tool: None,
            details: None,
        }
    }

    pub(crate) fn from_tool_error(
        tool_name: &str,
        code: ToolErrorCode,
        message: &str,
        detail: &str,
    ) -> Self {
        Self::new(code, user_facing_error_message(message, detail)).with_tool(tool_name)
    }

    pub(crate) fn with_retry_guidance(mut self, retry_guidance: impl Into<String>) -> Self {
        self.retry_guidance = Some(normalize_tool_error_text(retry_guidance.into()));
        self
    }

    pub(crate) fn with_tool(mut self, tool_name: impl Into<String>) -> Self {
        self.tool = Some(tool_name.into());
        self
    }

    pub(crate) fn with_details(mut self, details: Value) -> Self {
        if details.is_object() {
            self.details = Some(details);
        }
        self
    }

    pub(crate) fn with_input_shape_details(
        mut self,
        detail: impl Into<String>,
        spec: InputShapeErrorSpec,
    ) -> Self {
        let mut details = json!({
            "detail": detail.into(),
            "offending_fields": spec.offending_fields,
            "accepted_argument_families": spec.accepted_argument_families,
        });
        if !spec.normalization_performed.is_empty() {
            details["normalization_performed"] =
                serde_json::to_value(spec.normalization_performed).unwrap_or(Value::Null);
        }
        if let Some(retry_example) = spec.retry_example {
            details["retry_example"] = retry_example;
        }
        if let Some(reason) = spec.fail_closed_reason {
            details["fail_closed_reason"] = Value::String(reason);
        }
        if let Some(extra) = spec.extra_details {
            merge_detail_objects(&mut details, extra);
        }
        if let Some(retry_guidance) = spec.retry_guidance {
            self = self.with_retry_guidance(retry_guidance);
        }
        self.with_details(details)
    }

    fn normalized(&self) -> Self {
        Self {
            code: self.code,
            message: normalize_tool_error_text(&self.message),
            retry_guidance: self.retry_guidance.as_ref().map(normalize_tool_error_text),
            tool: self.tool.clone(),
            details: self.details.clone(),
        }
    }
}

pub(crate) fn tool_execution_error_value(
    output_format: OutputFormat,
    payload: &ToolErrorPayload,
) -> Result<Value> {
    let payload = payload.normalized();
    let structured = serde_json::to_value(&payload)?;
    Ok(json!({
        "content": [{
            "type": "text",
            "text": concise_tool_error_text(&payload),
            "mimeType": "text/plain",
        }],
        "structuredContent": structured,
        "isError": true,
        "_meta": result_meta(output_format.as_str(), output_format.as_str(), None),
    }))
}

pub(crate) fn normalize_tool_execution_error(
    tool_name: &str,
    output_format: OutputFormat,
    error: anyhow::Error,
) -> Result<Value> {
    let detail = format!("{error:#}");
    let message = error.to_string();
    let (code, details) = classify_tool_execution_error(message.as_str(), detail.as_str());
    let payload = ToolErrorPayload::from_tool_error(tool_name, code, &message, &detail)
        .with_retry_guidance(tool_retry_guidance(message.as_str()))
        .with_details(details);
    tool_execution_error_value(output_format, &payload)
}

pub(crate) fn structured_content(value: &Value) -> Option<&Value> {
    value.get("structuredContent")
}

fn concise_tool_error_text(payload: &ToolErrorPayload) -> String {
    let should_append_guidance = matches!(
        payload.code,
        ToolErrorCode::InvalidInput | ToolErrorCode::FileNotFound
    );
    match (should_append_guidance, payload.retry_guidance.as_deref()) {
        (true, Some(guidance)) if !payload.message.contains(guidance) => {
            normalize_tool_error_text(format!("{} {}", payload.message, guidance))
        }
        _ => payload.message.clone(),
    }
}

fn merge_detail_objects(target: &mut Value, extra: Value) {
    let Some(target_obj) = target.as_object_mut() else {
        return;
    };
    let Some(extra_obj) = extra.as_object() else {
        return;
    };
    for (key, value) in extra_obj {
        target_obj.insert(key.clone(), value.clone());
    }
}

pub(crate) fn input_shape_error_payload(
    tool_name: &str,
    message: impl Into<String>,
    detail: impl Into<String>,
    spec: InputShapeErrorSpec,
) -> ToolErrorPayload {
    ToolErrorPayload::new(ToolErrorCode::InvalidInput, message)
        .with_tool(tool_name)
        .with_input_shape_details(detail, spec)
}

fn normalize_tool_error_text(text: impl AsRef<str>) -> String {
    let mut normalized = text
        .as_ref()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() > MAX_TOOL_ERROR_TEXT_LEN {
        normalized = normalized
            .chars()
            .take(MAX_TOOL_ERROR_TEXT_LEN.saturating_sub(1))
            .collect();
        normalized.push('…');
    }
    if normalized.is_empty() {
        "internal tool error".to_owned()
    } else {
        normalized
    }
}

fn tool_retry_guidance(detail: &str) -> &'static str {
    if detail.contains("invalid regex pattern") {
        "Fix regex syntax, or switch to literal-search mode if regex is not required, then retry."
    } else if detail.contains("unsupported output_format") {
        "Use supported output_format value 'toon' or 'json', then retry."
    } else if detail.contains("graph not ready") || detail.contains("stale graph") {
        "Run graph build/update or allow stale/partial mode when supported, then retry."
    } else if detail.contains("missing required")
        || detail.contains("missing ")
        || detail.contains("invalid ")
        || detail.contains("must be ")
        || detail.contains("provide exactly one selector")
    {
        "Fix tool arguments, then retry."
    } else if detail.contains("file not found") {
        "Use repo-relative file path inside current root, then retry."
    } else {
        "Fix tool arguments or graph state, then retry."
    }
}

fn classify_tool_execution_error(message: &str, detail: &str) -> (ToolErrorCode, Value) {
    let lowered = detail.to_ascii_lowercase();
    let mut details = json!({ "detail": detail });

    if let Some(path) = message.strip_prefix("file not found: ") {
        details["path"] = Value::String(path.to_owned());
        return (ToolErrorCode::FileNotFound, details);
    }

    if let Some(path) = message
        .strip_prefix("invalid file path '")
        .and_then(|rest| rest.split_once('\'').map(|(path, _)| path))
    {
        details["path"] = Value::String(path.to_owned());
    }
    if let Some(path) = message
        .strip_prefix("invalid subpath '")
        .and_then(|rest| rest.split_once('\'').map(|(path, _)| path))
    {
        details["path"] = Value::String(path.to_owned());
    }

    let code = if lowered.contains("file not found") {
        ToolErrorCode::FileNotFound
    } else if lowered.contains("symbol not found") {
        ToolErrorCode::SymbolNotFound
    } else if lowered.contains("graph not ready") || lowered.contains("stale graph") {
        ToolErrorCode::GraphStale
    } else if lowered.contains("timeout") || lowered.contains("timed out") {
        ToolErrorCode::Timeout
    } else if lowered.contains("dependency failed")
        || lowered.contains("service unavailable")
        || lowered.contains("connection refused")
    {
        ToolErrorCode::DependencyFailed
    } else if lowered.contains("invalid regex pattern")
        || lowered.contains("unsupported output_format")
        || lowered.contains("invalid subpath")
        || lowered.contains("invalid file path")
        || lowered.contains("invalid pattern glob")
        || lowered.contains("invalid globs filter")
        || lowered.contains("invalid exclude_globs filter")
        || lowered.contains("invalid template extension patterns")
        || lowered.contains("invalid text-asset extension patterns")
        || lowered.contains("missing required")
        || lowered.contains("provide exactly one selector")
        || lowered.contains("line numbers are 1-based")
        || lowered.contains("invalid line range")
        || lowered.contains("exceeds file length")
        || lowered.contains("line-context selector")
        || lowered.contains("single-range selector")
        || lowered.contains("ambiguous change source")
        || lowered.contains("non-empty")
        || lowered.contains("requires")
    {
        ToolErrorCode::InvalidInput
    } else {
        ToolErrorCode::InternalToolError
    };

    (code, details)
}

fn infer_resource_links(raw: &Value) -> Vec<ResourceLink> {
    let mut links = Vec::new();
    let Some(object) = raw.as_object() else {
        return links;
    };

    if let Some(source_id) = object.get("source_id").and_then(Value::as_str)
        && !source_id.trim().is_empty()
        && !object
            .get("source_id")
            .is_some_and(|value| matches!(value, Value::Null))
    {
        links.push(ResourceLink {
            item_type: "resource_link",
            uri: format!("atlas://saved-context/{source_id}"),
            name: "saved_context".to_owned(),
            title: object
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Saved context {source_id}")),
            mime_type: Some("text/plain".to_owned()),
        });
    }

    let file = object.get("file").and_then(Value::as_str);
    let heading = object
        .get("heading_path")
        .and_then(Value::as_str)
        .or_else(|| object.get("heading_slug").and_then(Value::as_str));
    if let (Some(file), Some(heading)) = (file, heading) {
        links.push(ResourceLink {
            item_type: "resource_link",
            uri: format!("atlas://docs/{file}#{heading}"),
            name: "docs_section".to_owned(),
            title: object
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Docs section {heading}")),
            mime_type: Some("text/markdown".to_owned()),
        });
    }

    links
}

#[cfg(test)]
mod tests {
    use super::{
        InputShapeErrorSpec, ToolErrorCode, ToolErrorPayload, ToolResultBuilder,
        input_shape_error_payload, normalize_tool_error_text, structured_content,
        tool_execution_error_value,
    };
    use crate::output::OutputFormat;
    use serde_json::json;

    #[test]
    fn builder_emits_structured_content_for_object_payloads() {
        let response = ToolResultBuilder::new(OutputFormat::Json)
            .build_value(json!({"ok": true, "count": 2}))
            .expect("tool result");

        assert_eq!(response["structuredContent"]["ok"], json!(true));
        assert!(
            response["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("\"count\": 2")
        );
    }

    #[test]
    fn builder_adds_saved_context_resource_link_content_items() {
        let response = ToolResultBuilder::new(OutputFormat::Json)
            .build_value(json!({
                "routing": "pointer",
                "source_id": "src-123",
                "label": "Review bundle"
            }))
            .expect("tool result");

        let content = response["content"].as_array().expect("content");
        assert_eq!(content[1]["type"], json!("resource_link"));
        assert_eq!(content[1]["uri"], json!("atlas://saved-context/src-123"));
        assert_eq!(content[1]["title"], json!("Review bundle"));
    }

    #[test]
    fn builder_adds_docs_resource_link_content_items() {
        let response = ToolResultBuilder::new(OutputFormat::Json)
            .build_value(json!({
                "file": "wiki/mcp-reference.md",
                "heading_path": "document.mcp-reference.tools",
                "title": "Tools"
            }))
            .expect("tool result");

        let content = response["content"].as_array().expect("content");
        assert_eq!(content[1]["type"], json!("resource_link"));
        assert_eq!(
            content[1]["uri"],
            json!("atlas://docs/wiki/mcp-reference.md#document.mcp-reference.tools")
        );
    }

    #[test]
    fn builder_skips_structured_content_for_scalar_payloads() {
        let response = ToolResultBuilder::new(OutputFormat::Json)
            .build_value(json!("hello"))
            .expect("tool result");
        assert!(structured_content(&response).is_none());
    }

    #[test]
    fn builder_skips_structured_content_for_array_payloads() {
        let response = ToolResultBuilder::new(OutputFormat::Json)
            .build_value(json!([1, 2, 3]))
            .expect("tool result");
        assert!(structured_content(&response).is_none());
        assert!(
            response["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("[\n  1,")
        );
    }

    #[test]
    fn tool_result_falls_back_to_json_when_toon_root_empty() {
        let response = ToolResultBuilder::new(OutputFormat::Toon)
            .build_value(json!({}))
            .expect("tool result");
        assert_eq!(response["_meta"]["atlas:outputFormat"], json!("json"));
        assert_eq!(
            response["_meta"]["atlas:requestedOutputFormat"],
            json!("toon")
        );
        assert!(response["_meta"].get("atlas:fallbackReason").is_some());
    }

    #[test]
    fn tool_execution_error_builder_emits_is_error_and_structured_content() {
        let payload = ToolErrorPayload::from_tool_error(
            "query_graph",
            ToolErrorCode::InvalidInput,
            "invalid regex pattern: unclosed group",
            "invalid regex pattern: unclosed group",
        )
        .with_retry_guidance("Fix regex syntax or remove is_regex-style input, then retry.")
        .with_details(json!({"detail": "invalid regex pattern: unclosed group"}));
        let response = tool_execution_error_value(OutputFormat::Json, &payload)
            .expect("tool execution error result");

        assert_eq!(response["isError"], json!(true));
        assert_eq!(response["structuredContent"]["tool"], json!("query_graph"));
        assert_eq!(
            response["structuredContent"]["code"],
            json!("invalid_input")
        );
        assert_eq!(
            response["structuredContent"]["details"]["detail"],
            json!("invalid regex pattern: unclosed group")
        );
        assert_eq!(
            response["content"][0]["text"],
            json!(
                "invalid regex pattern: unclosed group Fix regex syntax or remove is_regex-style input, then retry."
            )
        );
        assert_eq!(response["content"][0]["mimeType"], json!("text/plain"));
    }

    #[test]
    fn tool_error_text_is_single_line_and_concise() {
        let normalized = normalize_tool_error_text(
            "  invalid regex pattern:\n\n  unclosed group with extra detail that should stay concise  ",
        );
        assert_eq!(
            normalized,
            "invalid regex pattern: unclosed group with extra detail that should stay concise"
        );

        let long = format!("{} tail", "x".repeat(300));
        let normalized_long = normalize_tool_error_text(long);
        assert!(normalized_long.chars().count() <= 240);
        assert!(normalized_long.ends_with('…'));
    }

    #[test]
    fn tool_execution_error_value_matches_expected_schema() {
        let payload = ToolErrorPayload::new(
            ToolErrorCode::FileNotFound,
            "file not found: src/missing.rs",
        )
        .with_tool("read_file_around_match")
        .with_retry_guidance("Use repo-relative file path inside current root, then retry.")
        .with_details(json!({
            "detail": "file not found: src/missing.rs",
            "path": "src/missing.rs"
        }));

        let response = tool_execution_error_value(OutputFormat::Toon, &payload)
            .expect("tool execution error result");

        assert_eq!(
            response,
            json!({
                "content": [{
                    "type": "text",
                    "text": "file not found: src/missing.rs Use repo-relative file path inside current root, then retry.",
                    "mimeType": "text/plain"
                }],
                "structuredContent": {
                    "code": "file_not_found",
                    "message": "file not found: src/missing.rs",
                    "retry_guidance": "Use repo-relative file path inside current root, then retry.",
                    "tool": "read_file_around_match",
                    "details": {
                        "detail": "file not found: src/missing.rs",
                        "path": "src/missing.rs"
                    }
                },
                "isError": true,
                "_meta": {
                    "atlas:outputFormat": "toon",
                    "atlas:requestedOutputFormat": "toon"
                }
            })
        );
    }

    #[test]
    fn input_shape_error_helper_emits_shared_retry_contract_fields() {
        let payload = input_shape_error_payload(
            "query_graph",
            "query_graph needs non-empty 'text', non-empty 'regex', or both",
            "query_graph rejected empty text and regex after normalization",
            InputShapeErrorSpec {
                offending_fields: vec!["text".to_owned(), "regex".to_owned()],
                normalization_performed: vec![
                    "trimmed whitespace-only text to empty".to_owned(),
                    "normalized empty regex to missing".to_owned(),
                ],
                accepted_argument_families: vec![
                    "text".to_owned(),
                    "regex".to_owned(),
                    "text + regex".to_owned(),
                ],
                retry_example: Some(json!({"text": "compute"})),
                fail_closed_reason: Some(
                    "Atlas refused to guess because both searchable inputs were empty".to_owned(),
                ),
                retry_guidance: Some("Provide one accepted query shape and retry.".to_owned()),
                extra_details: None,
            },
        );

        let response = tool_execution_error_value(OutputFormat::Json, &payload).expect("result");
        let details = &response["structuredContent"]["details"];
        assert_eq!(details["offending_fields"], json!(["text", "regex"]));
        assert_eq!(
            details["normalization_performed"],
            json!([
                "trimmed whitespace-only text to empty",
                "normalized empty regex to missing"
            ])
        );
        assert_eq!(
            details["accepted_argument_families"],
            json!(["text", "regex", "text + regex"])
        );
        assert_eq!(details["retry_example"], json!({"text": "compute"}));
        assert_eq!(
            details["fail_closed_reason"],
            json!("Atlas refused to guess because both searchable inputs were empty")
        );
        assert_eq!(
            response["content"][0]["text"],
            json!(
                "query_graph needs non-empty 'text', non-empty 'regex', or both Provide one accepted query shape and retry."
            )
        );
    }
}
