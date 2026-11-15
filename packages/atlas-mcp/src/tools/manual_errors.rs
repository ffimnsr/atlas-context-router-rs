//! Shared `man` / `tool_help` error contract.
//!
//! Kept in its own module so the manual lookup error payload and text cannot
//! drift between the two surfaces and so `manual.rs` stays below the file-size
//! threshold.

use anyhow::Result;
use serde_json::{Value, json};

use crate::output::OutputFormat;
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ManualLookupError {
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

impl ManualLookupError {
    /// Stable, punctuation-clean message shared by the CLI and MCP surfaces.
    pub(super) fn message(&self) -> String {
        match self {
            Self::UnknownNamespace { namespace } => {
                let requested = if namespace.is_empty() {
                    "<missing>"
                } else {
                    namespace.as_str()
                };
                format!("unsupported manual namespace '{requested}'; only 'mcp' is supported")
            }
            Self::MissingToolName => "missing MCP tool name for manual lookup".to_owned(),
            Self::HiddenInternalTool { tool_name } => {
                format!("tool '{tool_name}' is hidden or internal and has no public manual entry")
            }
            Self::UnknownTool {
                tool_name,
                suggestions,
            } => {
                if suggestions.is_empty() {
                    format!("unknown MCP tool '{tool_name}'")
                } else {
                    format!(
                        "unknown MCP tool '{tool_name}'; nearest visible tools: {}",
                        suggestions.join(", ")
                    )
                }
            }
        }
    }
}

pub(super) fn manual_lookup_error_response(
    error: ManualLookupError,
    output_format: OutputFormat,
) -> Result<Value> {
    let payload = manual_error_payload("man", &error);
    tool_execution_error_value(output_format, &payload)
}

pub(super) fn tool_help_error_response(
    error: ManualLookupError,
    output_format: OutputFormat,
) -> Result<Value> {
    let payload = manual_error_payload("tool_help", &error);
    tool_execution_error_value(output_format, &payload)
}

/// One error payload builder for both manual surfaces so `man` and `tool_help`
/// cannot drift in code, suggestions, or retry guidance.
fn manual_error_payload(tool: &str, error: &ManualLookupError) -> ToolErrorPayload {
    match error {
        ManualLookupError::UnknownNamespace { namespace } => ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            error.message(),
        )
        .with_tool(tool)
        .with_retry_guidance("Use namespace 'mcp' with an exact exported tool name, then retry.")
        .with_details(json!({
            "requested_namespace": namespace,
            "accepted_namespaces": ["mcp"],
            "retry_example": {
                "namespace": "mcp",
                "tool_name": "query_graph"
            }
        })),
        ManualLookupError::MissingToolName => {
            ToolErrorPayload::new(ToolErrorCode::InvalidInput, error.message())
                .with_tool(tool)
                .with_retry_guidance(
                    "Call tool_list or tool_search, then retry with one exact exported MCP tool name.",
                )
                .with_details(json!({
                    "retry_example": {
                        "name": "query_graph"
                    }
                }))
        }
        ManualLookupError::HiddenInternalTool { tool_name } => {
            ToolErrorPayload::new(ToolErrorCode::InvalidInput, error.message())
                .with_tool(tool)
                .with_retry_guidance(
                    "Use tool_list or tool_search to pick a visible exported MCP tool.",
                )
                .with_details(json!({
                    "requested_tool_name": tool_name,
                    "reason": "hidden_or_internal_tool",
                    "suggestions": []
                }))
        }
        ManualLookupError::UnknownTool {
            tool_name,
            suggestions,
        } => ToolErrorPayload::new(ToolErrorCode::InvalidInput, error.message())
            .with_tool(tool)
            .with_retry_guidance(
                "Call tool_search when the exact name is unclear, then retry with one exact exported tool name.",
            )
            .with_details(json!({
                "requested_tool_name": tool_name,
                "suggestions": suggestions
            })),
    }
}
