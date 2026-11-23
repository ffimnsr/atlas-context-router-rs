use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::output::{OutputFormat, render_value};

use super::dispatch::is_known_tool_name;
use super::manual_bounds::{ManualLimits, apply_manual_limits, schema_field_description};
use super::manual_errors::{
    ManualLookupError, manual_lookup_error_response, tool_help_error_response,
};
use super::manual_guidance::{few_shot_prompts, special_target_tool_examples, tool_usage_guidance};
use super::registry;
use super::registry::{tool_descriptor_by_name, tool_descriptors, tool_result_contract};

const MAX_DESCRIPTION_CHARS: usize = 220;
pub(crate) const MAX_EXAMPLE_CHARS: usize = 240;
const MAX_SUGGESTIONS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualDocument {
    /// Stable tool-name alias mirroring `resolved_tool_name`.
    pub name: String,
    pub requested_namespace: String,
    pub requested_tool_name: String,
    pub resolved_tool_name: String,
    pub description: String,
    pub tool_structure: ToolManualStructure,
    pub input_args: Vec<ToolManualField>,
    pub input_contract: ToolManualInputContract,
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
    pub idempotent: bool,
    pub open_world: bool,
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
pub struct ToolManualInputContract {
    pub canonical_form: String,
    pub families: Vec<ToolManualInputFamily>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualInputFamily {
    pub family_name: String,
    pub family_kind: String,
    pub discriminant_field: Option<String>,
    pub accepted_values: Vec<String>,
    pub mutually_exclusive_legacy_fields: Vec<String>,
    pub variants: Vec<ToolManualInputVariant>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolManualInputVariant {
    pub value: String,
    pub required_companion_fields: Vec<String>,
    pub minimal_example: Value,
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
    pub tool_usage: Vec<String>,
    pub target_tool_call_examples: Vec<String>,
    pub few_shot_prompts: Vec<String>,
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
    #[serde(default)]
    pub payload_truncated: bool,
}

pub fn tool_manual(namespace: &str, tool_name: &str) -> Result<ToolManualDocument> {
    match lookup_tool_manual(namespace, tool_name) {
        Ok(mut document) => {
            // Keep the CLI and MCP manual surfaces byte-identical by applying
            // the same default compact/byte limits at the shared entry point.
            apply_manual_limits(&mut document, ManualLimits::default());
            Ok(document)
        }
        Err(error) => Err(anyhow!(error.message())),
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
        "  annotations: read_only={}, state_changing={}, destructive={}, idempotent={}, open_world={}",
        document.tool_structure.annotations.read_only,
        document.tool_structure.annotations.state_changing,
        document.tool_structure.annotations.destructive,
        document.tool_structure.annotations.idempotent,
        document.tool_structure.annotations.open_world
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
    lines.push("input_contract:".to_owned());
    lines.push(format!(
        "  canonical_form: {}",
        document.input_contract.canonical_form
    ));
    lines.push("  families:".to_owned());
    if document.input_contract.families.is_empty() {
        lines.push("    - none".to_owned());
    } else {
        for family in &document.input_contract.families {
            lines.push(format!("    - family_name: {}", family.family_name));
            lines.push(format!("      family_kind: {}", family.family_kind));
            lines.push(format!(
                "      discriminant_field: {}",
                family.discriminant_field.as_deref().unwrap_or("none")
            ));
            lines.push(format!(
                "      accepted_values: [{}]",
                family.accepted_values.join(", ")
            ));
            lines.push(format!(
                "      mutually_exclusive_legacy_fields: [{}]",
                family.mutually_exclusive_legacy_fields.join(", ")
            ));
            lines.push("      variants:".to_owned());
            for variant in &family.variants {
                lines.push(format!("        - value: {}", variant.value));
                lines.push(format!(
                    "          required_companion_fields: [{}]",
                    variant.required_companion_fields.join(", ")
                ));
                lines.push(format!(
                    "          minimal_example: {}",
                    variant.minimal_example
                ));
            }
        }
    }
    if !document.input_contract.notes.is_empty() {
        lines.push("  notes:".to_owned());
        for note in &document.input_contract.notes {
            lines.push(format!("    - {note}"));
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
    lines.push("  tool_usage:".to_owned());
    for guidance in &document.usage.tool_usage {
        lines.push(format!("    - {guidance}"));
    }
    lines.push("  target_tool_call_examples:".to_owned());
    for example in &document.usage.target_tool_call_examples {
        lines.push(format!("    - {example}"));
    }
    lines.push("  few_shot_prompts:".to_owned());
    for prompt in &document.usage.few_shot_prompts {
        lines.push(format!("    - {prompt}"));
    }

    lines.push(String::new());
    lines.push("error_cases:".to_owned());
    for error in &document.error_cases {
        lines.push(format!(
            "  - {}: {}. {}",
            error.code, error.when, error.behavior
        ));
    }

    if document.truncation.description_truncated
        || document.truncation.usage_examples_truncated
        || document.truncation.payload_truncated
    {
        lines.push(String::new());
        lines.push(format!(
            "truncation: description_truncated={}, usage_examples_truncated={}, payload_truncated={}",
            document.truncation.description_truncated,
            document.truncation.usage_examples_truncated,
            document.truncation.payload_truncated
        ));
    }

    lines.join("\n")
}

pub(crate) fn tool_help(args: Option<&Value>, output_format: OutputFormat) -> Result<Value> {
    let tool_name = args
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .ok_or(ManualLookupError::MissingToolName);

    let response = match tool_name {
        Ok(tool_name) => match lookup_tool_manual("mcp", tool_name) {
            Ok(document) => {
                build_manual_tool_response(&document, output_format, ManualLimits::from_args(args))?
            }
            Err(error) => return tool_help_error_response(error, output_format),
        },
        Err(error) => return tool_help_error_response(error, output_format),
    };

    Ok(response)
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
            Ok(document) => {
                build_manual_tool_response(&document, output_format, ManualLimits::from_args(args))?
            }
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

    let input_schema = Value::Object((*tool.input_schema).clone());
    let output_schema = tool
        .output_schema
        .as_ref()
        .map(|schema| Value::Object((**schema).clone()));
    let description = truncate_with_flag(
        tool.description.as_deref().unwrap_or_default(),
        MAX_DESCRIPTION_CHARS,
    );
    let input_args = schema_fields(&input_schema);
    let response_fields = output_schema
        .as_ref()
        .map(schema_fields)
        .unwrap_or_default();
    let target_examples = target_tool_examples(&tool.name, &input_schema);
    let mcp_manual_call = truncate_text(
        &json!({
            "name": "man",
            "arguments": {
                "namespace": "mcp",
                "tool_name": tool.name
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
        name: tool.name.to_string(),
        requested_namespace: namespace.to_owned(),
        requested_tool_name: tool_name.to_owned(),
        resolved_tool_name: tool.name.to_string(),
        description: description.value,
        tool_structure: ToolManualStructure {
            purpose: truncate_text(
                tool.description.as_deref().unwrap_or_default(),
                MAX_DESCRIPTION_CHARS,
            ),
            operation_name: tool.name.to_string(),
            request_shape: top_level_shape("request object", &input_schema),
            response_shape: if tool.output_schema.is_some() {
                "MCP tool result envelope with structuredContent object and metadata fields"
                    .to_owned()
            } else {
                "MCP tool result envelope with content text, metadata fields, and no advertised structuredContent schema"
                    .to_owned()
            },
            result_contract: tool_result_contract_label(&tool.name).to_owned(),
            annotations: ToolManualAnnotations {
                read_only: tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.read_only_hint)
                    .unwrap_or(false),
                state_changing: !tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.read_only_hint)
                    .unwrap_or(false),
                destructive: tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.destructive_hint)
                    .unwrap_or(false),
                idempotent: tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.idempotent_hint)
                    .unwrap_or(true),
                open_world: tool
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.open_world_hint)
                    .unwrap_or(false),
            },
        },
        input_args,
        input_contract: tool_input_contract(&tool.name, &input_schema),
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
            tool_usage: tool_usage_guidance(&tool.name),
            target_tool_call_examples: target_examples,
            few_shot_prompts: few_shot_prompts(
                &tool.name,
                tool.description.as_deref().unwrap_or_default(),
            ),
        },
        error_cases: error_cases(),
        truncation: ToolManualTruncation {
            description_truncated: description.truncated,
            usage_examples_truncated,
            payload_truncated: false,
        },
    })
}

fn build_manual_tool_response(
    document: &ToolManualDocument,
    _output_format: OutputFormat,
    limits: ManualLimits,
) -> Result<Value> {
    let mut document = document.clone();
    apply_manual_limits(&mut document, limits);
    let raw = serde_json::to_value(&document)?;
    let text = render_value(&raw)?.text;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": text,
        }],
        "structuredContent": raw,
        "_meta": {},
    }))
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
                "Atlas metadata map for additional result annotations when present."
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

fn tool_input_contract(tool_name: &str, input_schema: &Value) -> ToolManualInputContract {
    let mut families = match tool_name {
        "get_context" => vec![target_input_family()],
        "read_file_excerpt" => vec![excerpt_selector_input_family()],
        "get_docs_section" => vec![docs_selector_input_family()],
        "detect_changes" => vec![change_source_input_family(false)],
        "get_impact_radius" | "get_review_context" | "get_minimal_context" | "explain_change" => {
            vec![change_source_input_family(true)]
        }
        "update_graph" => vec![change_source_input_family(true)],
        "batch_query_graph" => vec![batch_query_items_input_family()],
        _ => Vec::new(),
    };

    if schema_has_property(input_schema, "repo_scope") {
        families.push(repo_scope_input_family());
    }

    let canonical_form = if families.is_empty() {
        "single object input; populate documented fields directly and avoid undocumented extras"
            .to_owned()
    } else {
        "prefer canonical discriminated objects or preferred field families for new calls"
            .to_owned()
    };

    let mut notes = Vec::new();
    if tool_name == "batch_query_graph" {
        notes.push(
            "Collection field is items; send 1-20 query_graph-shaped objects per call.".to_owned(),
        );
    }

    ToolManualInputContract {
        canonical_form,
        families,
        notes,
    }
}

mod examples;
mod input;

pub(crate) use self::examples::{suggest_tool_names, truncate_text};
use self::examples::{target_tool_examples, tool_result_contract_label, truncate_with_flag};
use self::input::batch_query_items_input_family;
use self::input::change_source_input_family;
use self::input::docs_selector_input_family;
use self::input::excerpt_selector_input_family;
use self::input::repo_scope_input_family;
use self::input::schema_fields;
use self::input::schema_has_property;
use self::input::target_input_family;
use self::input::top_level_shape;
