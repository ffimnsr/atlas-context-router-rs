use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::output::{OutputFormat, render_value};
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};

use super::dispatch::is_known_tool_name;
use super::registry::{tool_descriptor_by_name, tool_descriptors, tool_result_contract};

const MAX_DESCRIPTION_CHARS: usize = 220;
const MAX_EXAMPLE_CHARS: usize = 240;
const MAX_SUGGESTIONS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualDocument {
    pub requested_namespace: String,
    pub requested_tool_name: String,
    pub resolved_tool_name: String,
    pub description: String,
    pub tool_structure: ToolManualStructure,
    pub input_args: Vec<ToolManualField>,
    pub output_response: ToolManualOutputResponse,
    pub usage: ToolManualUsage,
    pub error_cases: Vec<ToolManualErrorCase>,
    pub truncation: ToolManualTruncation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualStructure {
    pub purpose: String,
    pub operation_name: String,
    pub request_shape: String,
    pub response_shape: String,
    pub result_contract: String,
    pub annotations: ToolManualAnnotations,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualAnnotations {
    pub read_only: bool,
    pub state_changing: bool,
    pub destructive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualField {
    pub name: String,
    pub field_type: String,
    pub required: bool,
    pub default_value: Option<String>,
    pub enum_values: Vec<String>,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualOutputResponse {
    pub response_shape: String,
    pub structured_content_available: bool,
    pub response_fields: Vec<ToolManualField>,
    pub metadata_fields: Vec<ToolManualField>,
    pub error_payload_fields: Vec<ToolManualField>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualUsage {
    pub cli: String,
    pub mcp_manual_tool_call: String,
    pub target_tool_call_examples: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualErrorCase {
    pub code: String,
    pub when: String,
    pub behavior: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualTruncation {
    pub description_truncated: bool,
    pub usage_examples_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ManualLookupError {
    UnknownNamespace {
        namespace: String,
    },
    MissingToolName,
    HiddenInternalTool {
        tool_name: String,
    },
    UnknownTool {
        tool_name: String,
        suggestions: Vec<String>,
    },
}

pub fn tool_manual(namespace: &str, tool_name: &str) -> Result<ToolManualDocument> {
    match lookup_tool_manual(namespace, tool_name) {
        Ok(document) => Ok(document),
        Err(error) => Err(anyhow!(manual_lookup_error_message(&error))),
    }
}

pub fn render_tool_manual_text(document: &ToolManualDocument) -> String {
    let mut lines = vec![format!(
        "{} — {}",
        document.resolved_tool_name, document.description
    )];
    lines.push(String::new());
    lines.push("structure:".to_owned());
    lines.push(format!("  purpose: {}", document.tool_structure.purpose));
    lines.push(format!(
        "  operation_name: {}",
        document.tool_structure.operation_name
    ));
    lines.push(format!(
        "  request_shape: {}",
        document.tool_structure.request_shape
    ));
    lines.push(format!(
        "  response_shape: {}",
        document.tool_structure.response_shape
    ));
    lines.push(format!(
        "  result_contract: {}",
        document.tool_structure.result_contract
    ));
    lines.push(format!(
        "  annotations: read_only={}, state_changing={}, destructive={}",
        document.tool_structure.annotations.read_only,
        document.tool_structure.annotations.state_changing,
        document.tool_structure.annotations.destructive
    ));

    lines.push(String::new());
    lines.push("input_args:".to_owned());
    if document.input_args.is_empty() {
        lines.push("  - none".to_owned());
    } else {
        for field in &document.input_args {
            let required = if field.required {
                "required"
            } else {
                "optional"
            };
            let mut line = format!("  - {}: {} ({required})", field.name, field.field_type);
            if let Some(default_value) = &field.default_value {
                line.push_str(&format!(", default={default_value}"));
            }
            if !field.enum_values.is_empty() {
                line.push_str(&format!(", enum=[{}]", field.enum_values.join(", ")));
            }
            lines.push(line);
            lines.push(format!("    {}", field.description));
        }
    }

    lines.push(String::new());
    lines.push("output_response:".to_owned());
    lines.push(format!(
        "  response_shape: {}",
        document.output_response.response_shape
    ));
    lines.push(format!(
        "  structured_content_available: {}",
        document.output_response.structured_content_available
    ));
    lines.push("  response_fields:".to_owned());
    if document.output_response.response_fields.is_empty() {
        lines.push("    - schema unavailable for structuredContent; consume content text and metadata fields".to_owned());
    } else {
        for field in &document.output_response.response_fields {
            lines.push(format!(
                "    - {}: {} ({}) — {}",
                field.name,
                field.field_type,
                if field.required {
                    "required"
                } else {
                    "optional"
                },
                field.description
            ));
        }
    }
    lines.push("  metadata_fields:".to_owned());
    for field in &document.output_response.metadata_fields {
        lines.push(format!(
            "    - {}: {} ({}) — {}",
            field.name,
            field.field_type,
            if field.required {
                "required"
            } else {
                "optional"
            },
            field.description
        ));
    }
    lines.push("  error_payload_fields:".to_owned());
    for field in &document.output_response.error_payload_fields {
        lines.push(format!(
            "    - {}: {} ({}) — {}",
            field.name,
            field.field_type,
            if field.required {
                "required"
            } else {
                "optional"
            },
            field.description
        ));
    }

    lines.push(String::new());
    lines.push("usage:".to_owned());
    lines.push(format!("  cli: {}", document.usage.cli));
    lines.push(format!(
        "  mcp_manual_tool_call: {}",
        document.usage.mcp_manual_tool_call
    ));
    lines.push("  target_tool_call_examples:".to_owned());
    for example in &document.usage.target_tool_call_examples {
        lines.push(format!("    - {}", example));
    }

    lines.push(String::new());
    lines.push("error_cases:".to_owned());
    for error in &document.error_cases {
        lines.push(format!(
            "  - {}: {}. {}",
            error.code, error.when, error.behavior
        ));
    }

    if document.truncation.description_truncated || document.truncation.usage_examples_truncated {
        lines.push(String::new());
        lines.push(format!(
            "truncation: description_truncated={}, usage_examples_truncated={}",
            document.truncation.description_truncated, document.truncation.usage_examples_truncated
        ));
    }

    lines.join("\n")
}

pub(crate) fn tool_man(args: Option<&Value>, output_format: OutputFormat) -> Result<Value> {
    let namespace = args
        .and_then(|value| value.get("namespace"))
        .and_then(Value::as_str)
        .ok_or(ManualLookupError::UnknownNamespace {
            namespace: String::new(),
        });
    let tool_name = args
        .and_then(|value| value.get("tool_name"))
        .and_then(Value::as_str)
        .ok_or(ManualLookupError::MissingToolName);

    let response = match (namespace, tool_name) {
        (Ok(namespace), Ok(tool_name)) => match lookup_tool_manual(namespace, tool_name) {
            Ok(document) => build_manual_tool_response(&document, output_format)?,
            Err(error) => return manual_lookup_error_response(error, output_format),
        },
        (Err(error), _) => return manual_lookup_error_response(error, output_format),
        (_, Err(error)) => return manual_lookup_error_response(error, output_format),
    };

    Ok(response)
}

fn lookup_tool_manual(
    namespace: &str,
    tool_name: &str,
) -> std::result::Result<ToolManualDocument, ManualLookupError> {
    if namespace != "mcp" {
        return Err(ManualLookupError::UnknownNamespace {
            namespace: namespace.to_owned(),
        });
    }
    if tool_name.trim().is_empty() {
        return Err(ManualLookupError::MissingToolName);
    }

    let Some(tool) = tool_descriptor_by_name(tool_name) else {
        if is_known_tool_name(tool_name) || tool_name.starts_with("__") {
            return Err(ManualLookupError::HiddenInternalTool {
                tool_name: tool_name.to_owned(),
            });
        }
        return Err(ManualLookupError::UnknownTool {
            tool_name: tool_name.to_owned(),
            suggestions: suggest_tool_names(tool_name),
        });
    };

    let description = truncate_with_flag(&tool.description, MAX_DESCRIPTION_CHARS);
    let input_args = schema_fields(&tool.input_schema);
    let response_fields = tool
        .output_schema
        .as_ref()
        .map(schema_fields)
        .unwrap_or_default();
    let target_examples = target_tool_examples(&tool.name, &tool.input_schema);
    let mcp_manual_call = truncate_text(
        &json!({
            "name": "man",
            "arguments": {
                "namespace": "mcp",
                "tool_name": tool.name,
                "output_format": "json"
            }
        })
        .to_string(),
        MAX_EXAMPLE_CHARS,
    );
    let usage_examples_truncated = mcp_manual_call.chars().count() >= MAX_EXAMPLE_CHARS
        || target_examples
            .iter()
            .any(|example| example.chars().count() >= MAX_EXAMPLE_CHARS);

    Ok(ToolManualDocument {
        requested_namespace: namespace.to_owned(),
        requested_tool_name: tool_name.to_owned(),
        resolved_tool_name: tool.name.clone(),
        description: description.value,
        tool_structure: ToolManualStructure {
            purpose: truncate_text(&tool.description, MAX_DESCRIPTION_CHARS),
            operation_name: tool.name.clone(),
            request_shape: top_level_shape("request object", &tool.input_schema),
            response_shape: if tool.output_schema.is_some() {
                "MCP tool result envelope with structuredContent object and metadata fields"
                    .to_owned()
            } else {
                "MCP tool result envelope with content text, metadata fields, and no advertised structuredContent schema"
                    .to_owned()
            },
            result_contract: tool_result_contract_label(&tool.name).to_owned(),
            annotations: ToolManualAnnotations {
                read_only: tool.annotations.read_only_hint,
                state_changing: tool.annotations.state_changing_hint,
                destructive: tool.annotations.destructive_hint,
            },
        },
        input_args,
        output_response: ToolManualOutputResponse {
            response_shape: if tool.output_schema.is_some() {
                "structuredContent follows advertised outputSchema when present".to_owned()
            } else {
                "structuredContent schema unavailable for this tool; rely on content text, metadata, and error envelope".to_owned()
            },
            structured_content_available: tool.output_schema.is_some(),
            response_fields,
            metadata_fields: metadata_fields(),
            error_payload_fields: error_payload_fields(),
        },
        usage: ToolManualUsage {
            cli: format!("man mcp {}", tool.name),
            mcp_manual_tool_call: mcp_manual_call,
            target_tool_call_examples: target_examples,
        },
        error_cases: error_cases(),
        truncation: ToolManualTruncation {
            description_truncated: description.truncated,
            usage_examples_truncated,
        },
    })
}

fn build_manual_tool_response(
    document: &ToolManualDocument,
    output_format: OutputFormat,
) -> Result<Value> {
    let raw = serde_json::to_value(document)?;
    let text = render_tool_manual_text(document);
    let rendered = match output_format {
        OutputFormat::Json => render_value(&raw, output_format)?,
        OutputFormat::Toon => crate::output::RenderedPayload {
            requested_format: output_format,
            actual_format: output_format,
            text,
            fallback_reason: None,
        },
    };

    let mut response = json!({
        "content": [{
            "type": "text",
            "text": rendered.text,
            "mimeType": rendered.actual_format.mime_type(),
        }],
        "structuredContent": raw,
        "_meta": {
            "atlas:outputFormat": rendered.actual_format.as_str(),
            "atlas:requestedOutputFormat": rendered.requested_format.as_str(),
        },
    });
    if let Some(reason) = rendered.fallback_reason {
        response["_meta"]["atlas:fallbackReason"] = Value::String(reason);
    }
    Ok(response)
}

fn manual_lookup_error_response(
    error: ManualLookupError,
    output_format: OutputFormat,
) -> Result<Value> {
    let payload = match error {
        ManualLookupError::UnknownNamespace { namespace } => ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            format!(
                "unsupported manual namespace '{}' ; only 'mcp' is supported",
                if namespace.is_empty() {
                    "<missing>"
                } else {
                    namespace.as_str()
                }
            ),
        )
        .with_tool("man")
        .with_retry_guidance("Use exact form: man mcp <mcp_tool_name>.")
        .with_details(json!({
            "requested_namespace": namespace,
            "accepted_namespaces": ["mcp"],
            "retry_example": {
                "namespace": "mcp",
                "tool_name": "query_graph"
            }
        })),
        ManualLookupError::MissingToolName => ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            "missing MCP tool name for manual lookup",
        )
        .with_tool("man")
        .with_retry_guidance("Use exact form: man mcp <mcp_tool_name>.")
        .with_details(json!({
            "requested_namespace": "mcp",
            "retry_example": {
                "namespace": "mcp",
                "tool_name": "query_graph"
            }
        })),
        ManualLookupError::HiddenInternalTool { tool_name } => ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            format!(
                "tool '{}' is hidden or internal and has no public manual entry",
                tool_name
            ),
        )
        .with_tool("man")
        .with_retry_guidance("Request a visible exported MCP tool name from tools/list.")
        .with_details(json!({
            "requested_tool_name": tool_name,
            "reason": "hidden_or_internal_tool"
        })),
        ManualLookupError::UnknownTool {
            tool_name,
            suggestions,
        } => ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            format!("unknown MCP tool '{}'", tool_name),
        )
        .with_tool("man")
        .with_retry_guidance("Call tools/list or retry with an exact exported MCP tool name.")
        .with_details(json!({
            "requested_tool_name": tool_name,
            "suggestions": suggestions,
        })),
    };
    tool_execution_error_value(output_format, &payload)
}

fn manual_lookup_error_message(error: &ManualLookupError) -> String {
    match error {
        ManualLookupError::UnknownNamespace { namespace } => format!(
            "unsupported manual namespace '{}' ; only 'mcp' is supported",
            if namespace.is_empty() {
                "<missing>"
            } else {
                namespace.as_str()
            }
        ),
        ManualLookupError::MissingToolName => "missing MCP tool name for manual lookup".to_owned(),
        ManualLookupError::HiddenInternalTool { tool_name } => {
            format!(
                "tool '{}' is hidden or internal and has no public manual entry",
                tool_name
            )
        }
        ManualLookupError::UnknownTool {
            tool_name,
            suggestions,
        } => {
            if suggestions.is_empty() {
                format!("unknown MCP tool '{}'", tool_name)
            } else {
                format!(
                    "unknown MCP tool '{}'; nearest visible tools: {}",
                    tool_name,
                    suggestions.join(", ")
                )
            }
        }
    }
}

fn metadata_fields() -> Vec<ToolManualField> {
    vec![
        ToolManualField {
            name: "content".to_owned(),
            field_type: "array<object>".to_owned(),
            required: true,
            default_value: None,
            enum_values: Vec::new(),
            description: "Human-readable rendered payload. Present on success and error results."
                .to_owned(),
        },
        ToolManualField {
            name: "structuredContent".to_owned(),
            field_type: "object".to_owned(),
            required: false,
            default_value: None,
            enum_values: Vec::new(),
            description:
                "Machine-readable payload when the tool returns object-shaped structured output."
                    .to_owned(),
        },
        ToolManualField {
            name: "_meta".to_owned(),
            field_type: "object".to_owned(),
            required: true,
            default_value: None,
            enum_values: Vec::new(),
            description:
                "Atlas output metadata including requested and actual output format, plus fallback reason when TOON falls back to JSON."
                    .to_owned(),
        },
        ToolManualField {
            name: "atlas_provenance".to_owned(),
            field_type: "object".to_owned(),
            required: true,
            default_value: None,
            enum_values: Vec::new(),
            description:
                "Atlas repo/db provenance metadata injected by dispatch for authoritative runtime context."
                    .to_owned(),
        },
        ToolManualField {
            name: "atlas_freshness".to_owned(),
            field_type: "object".to_owned(),
            required: false,
            default_value: None,
            enum_values: Vec::new(),
            description:
                "Freshness warning metadata when working-tree changes can make graph-backed answers stale."
                    .to_owned(),
        },
        ToolManualField {
            name: "atlas_readiness".to_owned(),
            field_type: "object".to_owned(),
            required: false,
            default_value: None,
            enum_values: Vec::new(),
            description:
                "Readiness gating metadata for graph-backed tools when Atlas evaluates safe-to-answer state."
                    .to_owned(),
        },
    ]
}

fn error_payload_fields() -> Vec<ToolManualField> {
    vec![
        ToolManualField {
            name: "isError".to_owned(),
            field_type: "boolean".to_owned(),
            required: true,
            default_value: None,
            enum_values: Vec::new(),
            description: "True when the tool returned a structured execution error payload."
                .to_owned(),
        },
        ToolManualField {
            name: "structuredContent.code".to_owned(),
            field_type: "string".to_owned(),
            required: true,
            default_value: None,
            enum_values: vec![
                "invalid_input".to_owned(),
                "file_not_found".to_owned(),
                "symbol_not_found".to_owned(),
                "graph_stale".to_owned(),
                "timeout".to_owned(),
                "dependency_failed".to_owned(),
                "internal_tool_error".to_owned(),
            ],
            description: "Stable Atlas tool error code.".to_owned(),
        },
        ToolManualField {
            name: "structuredContent.message".to_owned(),
            field_type: "string".to_owned(),
            required: true,
            default_value: None,
            enum_values: Vec::new(),
            description: "User-facing tool failure summary.".to_owned(),
        },
        ToolManualField {
            name: "structuredContent.retry_guidance".to_owned(),
            field_type: "string".to_owned(),
            required: false,
            default_value: None,
            enum_values: Vec::new(),
            description: "Deterministic retry guidance when Atlas can suggest a next action."
                .to_owned(),
        },
        ToolManualField {
            name: "structuredContent.details".to_owned(),
            field_type: "object".to_owned(),
            required: false,
            default_value: None,
            enum_values: Vec::new(),
            description: "Additional structured error details when available.".to_owned(),
        },
    ]
}

fn error_cases() -> Vec<ToolManualErrorCase> {
    vec![
        ToolManualErrorCase {
            code: "unknown_tool".to_owned(),
            when: "requested tool name does not match any visible exported MCP tool exactly"
                .to_owned(),
            behavior: "returns invalid_input with deterministic nearest-tool suggestions when available"
                .to_owned(),
        },
        ToolManualErrorCase {
            code: "deprecated_tool".to_owned(),
            when: "requested tool remains in registry but is explicitly marked deprecated"
                .to_owned(),
            behavior: "manual should return deprecation guidance instead of treating legacy prose as authoritative"
                .to_owned(),
        },
        ToolManualErrorCase {
            code: "hidden_internal_tool".to_owned(),
            when: "requested tool exists internally but is not exported in the visible MCP registry"
                .to_owned(),
            behavior: "returns invalid_input and refuses to expose internal-only manual details"
                .to_owned(),
        },
        ToolManualErrorCase {
            code: "schema_unavailable".to_owned(),
            when: "tool exports no outputSchema or field-level schema metadata is unavailable"
                .to_owned(),
            behavior: "manual keeps response and usage sections but marks structuredContent schema unavailable"
                .to_owned(),
        },
    ]
}

fn schema_fields(schema: &Value) -> Vec<ToolManualField> {
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();

    let mut names = props.keys().cloned().collect::<Vec<_>>();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            props
                .get(&name)
                .map(|field| schema_field(&name, field, &required))
        })
        .collect()
}

fn schema_field(
    name: &str,
    field: &Value,
    required: &std::collections::BTreeSet<String>,
) -> ToolManualField {
    ToolManualField {
        name: name.to_owned(),
        field_type: schema_type(field),
        required: required.contains(name),
        default_value: explicit_default_value(name, field),
        enum_values: field
            .get("enum")
            .and_then(Value::as_array)
            .map(|values| values.iter().map(json_literal).collect())
            .unwrap_or_default(),
        description: truncate_text(
            field
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("No schema description available."),
            MAX_DESCRIPTION_CHARS,
        ),
    }
}

fn schema_type(field: &Value) -> String {
    match field.get("type") {
        Some(Value::String(kind)) => {
            if kind == "array" {
                if let Some(items) = field.get("items") {
                    format!("array<{}>", nested_schema_type(items))
                } else {
                    "array".to_owned()
                }
            } else {
                kind.clone()
            }
        }
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | "),
        _ => nested_schema_type(field),
    }
}

fn nested_schema_type(field: &Value) -> String {
    match field.get("type") {
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | "),
        _ if field.get("properties").is_some() => "object".to_owned(),
        _ if field.get("enum").is_some() => "enum".to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn explicit_default_value(name: &str, field: &Value) -> Option<String> {
    if let Some(default_value) = field.get("default") {
        return Some(json_literal(default_value));
    }

    let description = field.get("description").and_then(Value::as_str)?;
    parse_default_from_description(name, description)
}

fn parse_default_from_description(name: &str, description: &str) -> Option<String> {
    if name == "output_format" && description.contains("'toon' (default)") {
        return Some("toon".to_owned());
    }

    let lower = description.to_ascii_lowercase();
    let marker = "default ";
    let start = lower.find(marker)? + marker.len();
    let tail = &description[start..];
    let trimmed = tail
        .trim_start_matches([':', ' '])
        .split([')', '.', ';', ','])
        .next()
        .unwrap_or_default()
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.trim_matches('`').trim_matches('\'').to_owned())
}

fn top_level_shape(label: &str, schema: &Value) -> String {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(Map::len)
        .unwrap_or(0);
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    format!("{label}: object with {properties} top-level fields ({required} required)")
}

fn target_tool_examples(tool_name: &str, input_schema: &Value) -> Vec<String> {
    let props = input_schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut args = Map::new();
    for field_name in required {
        if let Some(field) = props.get(&field_name) {
            args.insert(field_name.clone(), example_value(&field_name, field));
        }
    }
    if args.is_empty() {
        if let Some(primary) = first_example_field(&props)
            && let Some(field) = props.get(primary)
        {
            args.insert(primary.to_owned(), example_value(primary, field));
        }
        args.insert("output_format".to_owned(), Value::String("json".to_owned()));
    }

    let examples = vec![truncate_text(
        &json!({
            "name": tool_name,
            "arguments": Value::Object(args),
        })
        .to_string(),
        MAX_EXAMPLE_CHARS,
    )];
    examples
}

fn first_example_field(props: &Map<String, Value>) -> Option<&str> {
    [
        "text",
        "query",
        "name",
        "qname",
        "symbol",
        "tool_name",
        "file",
        "files",
    ]
    .into_iter()
    .find(|field| props.contains_key(*field))
}

fn example_value(field_name: &str, field: &Value) -> Value {
    if let Some(default_value) = field.get("default") {
        return default_value.clone();
    }
    if let Some(values) = field.get("enum").and_then(Value::as_array)
        && let Some(first) = values.first()
    {
        return first.clone();
    }
    match field.get("type") {
        Some(Value::String(kind)) => match kind.as_str() {
            "boolean" => Value::Bool(true),
            "integer" => Value::from(1),
            "number" => Value::from(1),
            "array" => Value::Array(Vec::new()),
            "string" if field_name == "output_format" => Value::String("json".to_owned()),
            "string" => Value::String(format!("<{field_name}>")),
            _ => Value::String(format!("<{field_name}>")),
        },
        _ => Value::String(format!("<{field_name}>")),
    }
}

fn suggest_tool_names(input: &str) -> Vec<String> {
    let needle = input.to_ascii_lowercase();
    let mut scored = tool_descriptors()
        .into_iter()
        .map(|tool| {
            let hay = tool.name.to_ascii_lowercase();
            let starts_with = hay.starts_with(&needle);
            let contains = hay.contains(&needle);
            let distance = levenshtein(&needle, &hay);
            (starts_with, contains, distance, tool.name)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .reverse()
            .then(left.1.cmp(&right.1).reverse())
            .then(left.2.cmp(&right.2))
            .then(left.3.len().cmp(&right.3.len()))
            .then(left.3.cmp(&right.3))
    });
    scored
        .into_iter()
        .filter(|(starts_with, contains, distance, _)| *starts_with || *contains || *distance <= 6)
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, _, name)| name)
        .collect()
}

fn levenshtein(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0usize; right_chars.len() + 1];

    for (left_idx, left_char) in left.chars().enumerate() {
        curr[0] = left_idx + 1;
        for (right_idx, right_char) in right_chars.iter().enumerate() {
            let cost = usize::from(left_char != *right_char);
            curr[right_idx + 1] = (curr[right_idx] + 1)
                .min(prev[right_idx + 1] + 1)
                .min(prev[right_idx] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[right_chars.len()]
}

fn tool_result_contract_label(tool_name: &str) -> &'static str {
    match tool_result_contract(tool_name) {
        super::registry::ToolResultContract::StableObject => "stable-object",
        super::registry::ToolResultContract::TextOnly => "text-only",
        super::registry::ToolResultContract::MixedNeedsRedesign => "mixed-needs-redesign",
    }
}

struct TruncatedText {
    value: String,
    truncated: bool,
}

fn truncate_with_flag(value: &str, max_chars: usize) -> TruncatedText {
    let truncated = value.chars().count() > max_chars;
    let value = truncate_text(value, max_chars);
    TruncatedText { value, truncated }
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    let mut truncated = chars.into_iter().take(keep).collect::<String>();
    truncated.push('…');
    truncated
}

fn json_literal(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_lookup_uses_exact_visible_name() {
        let doc = tool_manual("mcp", "query_graph").expect("manual doc");
        assert_eq!(doc.resolved_tool_name, "query_graph");
        assert_eq!(doc.requested_tool_name, "query_graph");
        assert!(
            doc.input_args
                .iter()
                .any(|field| field.name == "text" && !field.required)
        );
    }

    #[test]
    fn manual_lookup_rejects_unknown_namespace() {
        let error = tool_manual("sql", "query_graph").expect_err("must fail");
        assert!(error.to_string().contains("only 'mcp' is supported"));
    }

    #[test]
    fn manual_lookup_suggests_nearest_visible_tools() {
        let result = lookup_tool_manual("mcp", "query_grap").expect_err("must fail");
        let ManualLookupError::UnknownTool { suggestions, .. } = result else {
            panic!("expected unknown tool error");
        };
        assert_eq!(suggestions.first().map(String::as_str), Some("query_graph"));
    }

    #[test]
    fn manual_lookup_hides_internal_tools() {
        let result = lookup_tool_manual("mcp", "__test_sleep").expect_err("must fail");
        assert!(matches!(
            result,
            ManualLookupError::HiddenInternalTool { .. }
        ));
    }

    #[test]
    fn text_renderer_includes_required_sections() {
        let doc = tool_manual("mcp", "query_graph").expect("manual doc");
        let text = render_tool_manual_text(&doc);
        for section in [
            "structure:",
            "input_args:",
            "output_response:",
            "usage:",
            "error_cases:",
        ] {
            assert!(text.contains(section), "missing section {section}");
        }
        assert!(text.contains("man mcp query_graph"));
    }
}
