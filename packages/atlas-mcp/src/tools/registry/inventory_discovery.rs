//! Base `tools/list` registry entries for the `discovery` tool family.
//!
//! Assembled by `super::base_tool_list_json()`; entry order is
//! irrelevant because descriptors are sorted by name afterwards.

use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::{Value, json};

/// Base registry entry JSON for the discovery tools.
pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
                "name": "tool_list",
                "description": "List visible exported MCP tools in compact runtime inventory form. Use this instead of hardcoding tool tables in agent instructions; pair with tool_search and tool_help for discovery.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "category": { "type": "string", "description": "Optional exact category filter: graph, content, analysis, health, memory, maintenance, or introspection." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "tool_search",
                "description": "Search visible exported MCP tools by name, title, or description without executing them. Ranks matches with explicit lexical score factors and typo-tolerant fuzzy name matching, and returns suggestions when no strong direct match exists.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Short tool-name fragment or capability phrase to search, such as 'query', 'review', 'context', or 'docs'. Exact/prefix/contains matches rank highest; fuzzy name matching tolerates small typos." },
                        "limit": { "type": "integer", "description": "Maximum matches to return (default 10, max 50)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
        }),
        json!({
                "name": "tool_help",
                "description": "Return runtime manual documentation for one visible exported MCP tool by exact name. Shorthand for man with namespace='mcp'.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Exact exported MCP tool name to document. Case-sensitive." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["name"]
                }
        }),
        json!({
                "name": "man",
                "description": "Return authoritative runtime manual documentation for one visible exported MCP tool without executing that target tool. Requires namespace='mcp' and exact case-sensitive tool_name lookup from the live registry.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "namespace": { "type": "string", "description": "Manual namespace. Must be exactly 'mcp'." },
                        "tool_name": { "type": "string", "description": "Exact exported MCP tool name to document. Case-sensitive." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["namespace", "tool_name"]
                }
        }),
    ]
}
