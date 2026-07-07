use anyhow::Result;
use atlas_core::user_facing_error_message;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::output::{OutputFormat, render_value};

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

pub(crate) fn tool_execution_error_value(
    tool_name: &str,
    output_format: OutputFormat,
    message: &str,
    detail: &str,
    retry_guidance: Option<&str>,
    structured: Option<Value>,
) -> Result<Value> {
    let mut object = Map::new();
    object.insert("tool".to_owned(), Value::String(tool_name.to_owned()));
    object.insert(
        "message".to_owned(),
        Value::String(user_facing_error_message(message, detail)),
    );
    object.insert(
        "retry_guidance".to_owned(),
        Value::String(
            retry_guidance
                .unwrap_or("Fix tool arguments or graph state, then retry.")
                .to_owned(),
        ),
    );
    if let Some(structured) = structured.and_then(|value| value.as_object().cloned()) {
        object.extend(structured);
    }

    let mut response = ToolResultBuilder::new(output_format).build_value(Value::Object(object))?;
    response["isError"] = Value::Bool(true);
    Ok(response)
}

pub(crate) fn structured_content(value: &Value) -> Option<&Value> {
    value.get("structuredContent")
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
    use super::{ToolResultBuilder, structured_content, tool_execution_error_value};
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
        let response = tool_execution_error_value(
            "query_graph",
            OutputFormat::Json,
            "invalid regex pattern: unclosed group",
            "invalid regex pattern: unclosed group",
            Some("Fix regex syntax or remove is_regex-style input, then retry."),
            None,
        )
        .expect("tool execution error result");

        assert_eq!(response["isError"], json!(true));
        assert_eq!(response["structuredContent"]["tool"], json!("query_graph"));
        assert!(
            response["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("retry_guidance")
        );
    }
}
