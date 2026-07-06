use anyhow::Result;
use serde::Serialize;
use serde_json::{Value, json};

use crate::output::{OutputFormat, render_value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceLink {
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
        let mut response = json!({
            "content": [{
                "type": "text",
                "text": rendered.text,
                "mimeType": rendered.actual_format.mime_type(),
            }],
            "atlas_output_format": rendered.actual_format.as_str(),
            "atlas_requested_output_format": rendered.requested_format.as_str(),
        });

        if raw.is_object() || raw.is_array() {
            response["structuredContent"] = raw.clone();
        }

        if let Some(reason) = rendered.fallback_reason {
            response["atlas_fallback_reason"] = Value::String(reason);
        }

        let resource_links = infer_resource_links(&raw);
        if !resource_links.is_empty() {
            response["resourceLinks"] = serde_json::to_value(resource_links)?;
        }

        Ok(response)
    }
}

pub(crate) fn tool_result_value<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<Value> {
    ToolResultBuilder::new(output_format).build_serializable(value)
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
    use super::{ToolResultBuilder, structured_content};
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
    fn builder_adds_saved_context_resource_links() {
        let response = ToolResultBuilder::new(OutputFormat::Json)
            .build_value(json!({
                "routing": "pointer",
                "source_id": "src-123",
                "label": "Review bundle"
            }))
            .expect("tool result");

        let links = response["resourceLinks"].as_array().expect("links");
        assert_eq!(links[0]["uri"], json!("atlas://saved-context/src-123"));
        assert_eq!(links[0]["title"], json!("Review bundle"));
    }

    #[test]
    fn builder_adds_docs_resource_links() {
        let response = ToolResultBuilder::new(OutputFormat::Json)
            .build_value(json!({
                "file": "wiki/mcp-reference.md",
                "heading_path": "document.mcp-reference.tools",
                "title": "Tools"
            }))
            .expect("tool result");

        let links = response["resourceLinks"].as_array().expect("links");
        assert_eq!(
            links[0]["uri"],
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
    fn tool_result_falls_back_to_json_when_toon_root_empty() {
        let response = ToolResultBuilder::new(OutputFormat::Toon)
            .build_value(json!({}))
            .expect("tool result");
        assert_eq!(response["atlas_output_format"], json!("json"));
        assert!(response.get("atlas_fallback_reason").is_some());
    }
}
