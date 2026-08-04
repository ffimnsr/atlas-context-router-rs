//! Tool result-contract classification and the generated MCP tools
//! markdown inventory (`tool_list_markdown`).

use super::*;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolResultContract {
    StableObject,
    #[allow(dead_code)]
    TextOnly,
}

impl ToolResultContract {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::StableObject => "stable-object",
            Self::TextOnly => "text-only",
        }
    }

    fn output_schema_note(self) -> &'static str {
        match self {
            Self::StableObject => "exact structuredContent schema",
            Self::TextOnly => "none",
        }
    }

    pub(crate) fn guidance(self) -> &'static str {
        match self {
            Self::StableObject => {
                "Returns object structuredContent in JSON mode; outputSchema validates that object."
            }
            Self::TextOnly => {
                "Do not rely on structuredContent; consume text content or resource links only."
            }
        }
    }
}

pub(crate) fn tool_result_contract(name: &str) -> ToolResultContract {
    match name {
        "list_graph_stats"
        | "repo_registry"
        | "tool_list"
        | "tool_search"
        | "tool_help"
        | "broker_status"
        | "get_context_stats"
        | "man"
        | "detect_changes"
        | "get_impact_radius"
        | "get_review_context"
        | "get_minimal_context"
        | "explain_change"
        | "traverse_graph"
        | "get_context"
        | "build_or_update_graph"
        | "postprocess_graph"
        | "status"
        | "doctor"
        | "db_check"
        | "debug_graph"
        | "explain_query"
        | "analyze_architecture"
        | "analyze_metrics"
        | "assess_risk"
        | "analyze_patterns"
        | "find_large_functions"
        | "find_complex_functions"
        | "find_similar_functions"
        | "find_duplicates"
        | "infer_modules"
        | "label_components"
        | "get_session_status"
        | "compact_session"
        | "resume_session"
        | "record_session_event"
        | "wake_up"
        | "read_saved_context"
        | "save_context_artifact"
        | "purge_saved_context"
        | "get_global_memory"
        | "symbol_neighbors"
        | "cross_file_links"
        | "concept_clusters"
        | "memory_store"
        | "memory_recall"
        | "analyze_safety"
        | "analyze_remove"
        | "analyze_dead_code"
        | "analyze_dependency"
        | "resolve_symbol"
        | "search_files"
        | "search_content"
        | "read_file_excerpt"
        | "get_docs_section"
        | "read_file_around_match"
        | "search_templates"
        | "search_text_assets"
        | "query_graph"
        | "batch_query_graph"
        | "search_saved_context"
        | "search_decisions"
        | "cross_session_search" => ToolResultContract::StableObject,
        _ => panic!("tool_result_contract missing classification for {name}"),
    }
}

pub fn tool_list_markdown() -> String {
    let mut markdown = format!(
        "# MCP Tools\n\nThis file is generated from rmcp-backed `atlas_mcp::tool_list()` descriptors serialized from `rmcp::model::Tool`. Do not edit by hand.\n\nMCP quick guidance:\n- protocol versions: `{}`\n- call `server/discover` for capability negotiation\n- on stdio and HTTP requests after discovery, include `params._meta` with protocol version and client capabilities\n- use explicit `arguments.repo_root` or server launch cwd for repo selection; do not rely on MCP Roots\n- HTTP transport is stateless at protocol level: no `Mcp-Session-Id`, `GET /mcp`, `DELETE /mcp`, or `Last-Event-ID` flow\n\nResult contract legend:\n- `stable-object`: JSON `structuredContent` is source of truth; `outputSchema` validates that object.\n- `text-only`: consume MCP `content`; no `outputSchema` advertised.\n\n| Tool | Title | Input schema | Result contract | Output schema | Description |\n|------|-------|--------------|-----------------|---------------|-------------|\n",
        crate::spec::supported_protocol_versions_display()
    );

    for tool in tool_descriptors() {
        let contract = tool_result_contract(&tool.name);
        let input_schema = Value::Object((*tool.input_schema).clone());
        let title = tool.title.as_deref().expect("tool title");
        let description = tool.description.as_deref().expect("tool description");
        markdown.push_str("| `");
        markdown.push_str(&tool.name);
        markdown.push_str("` | ");
        markdown.push_str(&escape_markdown_table_cell(title));
        markdown.push_str(" | ");
        markdown.push_str(&input_schema_note(&input_schema));
        markdown.push_str(" | `");
        markdown.push_str(contract.label());
        markdown.push_str("` | ");
        markdown.push_str(contract.output_schema_note());
        markdown.push_str(" | ");
        markdown.push_str(&escape_markdown_table_cell(description));
        markdown.push_str(" |\n");
    }

    markdown
}

fn input_schema_note(schema: &Value) -> String {
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or_default();
    format!("object; required fields: {required}")
}

fn escape_markdown_table_cell(text: &str) -> String {
    text.replace('\n', " ").replace('|', "\\|")
}
