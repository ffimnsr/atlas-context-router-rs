//! Directional call-chain tracing: `trace_calls`.
//!
//! Kept out of `graph.rs` because that module is already at the repo file-size
//! ceiling and this surface owns different option semantics.

use std::str::FromStr;

use anyhow::Result;
use atlas_core::{Edge, EdgeKind, Node};
use atlas_review::normalize_qn_kind_tokens;
use atlas_store_sqlite::{Store, TraceDirection, TraceOptions};
use serde_json::{Map, Value, json};

use crate::tool_result::{
    ToolErrorCode, ToolErrorPayload, ToolSuccessEnvelope, normalized_tool_result_value,
    tool_execution_error_value,
};

use super::shared::{open_store, str_arg, string_array_arg, u64_arg};

const DEFAULT_MAX_DEPTH: u32 = 3;
const DEFAULT_MAX_NODES: usize = 200;

pub(super) fn tool_trace_calls(
    args: Option<&serde_json::Value>,
    _repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<Value> {
    let from_raw = str_arg(args, "from")?.ok_or_else(|| {
        anyhow::anyhow!("missing required argument: from (symbol name or qualified name)")
    })?;
    let from = normalize_qn_kind_tokens(from_raw);
    let to = str_arg(args, "to")?.map(normalize_qn_kind_tokens);

    let direction = match str_arg(args, "direction")? {
        Some(value) => match TraceDirection::from_str(value) {
            Ok(direction) => direction,
            Err(error) => {
                return invalid_input(
                    output_format,
                    "trace_calls",
                    error,
                    "Use up, down, or both.",
                );
            }
        },
        None => TraceDirection::Down,
    };

    let raw_kinds = string_array_arg(args, "edge_kinds")?;
    let mut edge_kinds = Vec::new();
    if raw_kinds.is_empty() {
        edge_kinds.push(EdgeKind::Calls);
    } else {
        for raw in raw_kinds {
            match EdgeKind::from_str(&raw) {
                Ok(kind) => edge_kinds.push(kind),
                Err(error) => {
                    return invalid_input(
                        output_format,
                        "trace_calls",
                        error,
                        "Use edge kinds: calls, imports, contains, defines, implements, extends, tests, references, tested_by.",
                    );
                }
            }
        }
    }

    let max_depth = u64_arg(args, "max_depth")
        .map(|value| value.min(u32::MAX as u64) as u32)
        .unwrap_or(DEFAULT_MAX_DEPTH);
    let max_nodes = u64_arg(args, "max_nodes")
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_NODES)
        .max(1);

    let store = open_store(db_path)?;

    let from = match resolve_qname(&store, &from)? {
        QnameResolution::Found(qname) => qname,
        QnameResolution::Missing => {
            return symbol_not_found(output_format, "trace_calls", &from);
        }
        QnameResolution::Ambiguous(message) => {
            return invalid_input(
                output_format,
                "trace_calls",
                message,
                "Use a qualified name from resolve_symbol.",
            );
        }
    };

    if let Some(to) = to {
        let to = match resolve_qname(&store, &to)? {
            QnameResolution::Found(qname) => qname,
            QnameResolution::Missing => {
                return symbol_not_found(output_format, "trace_calls", &to);
            }
            QnameResolution::Ambiguous(message) => {
                return invalid_input(
                    output_format,
                    "trace_calls",
                    message,
                    "Use a qualified name from resolve_symbol.",
                );
            }
        };
        let opts = TraceOptions {
            direction,
            edge_kinds: &edge_kinds,
            max_depth,
            max_nodes,
        };
        let Some(result) = store.trace_call_path(&from, &to, &opts)? else {
            return symbol_not_found(output_format, "trace_calls", &from);
        };
        return path_payload(&result, &edge_kinds, &direction, max_depth, output_format);
    }

    let opts = TraceOptions {
        direction,
        edge_kinds: &edge_kinds,
        max_depth,
        max_nodes,
    };
    let Some(result) = store.trace_calls(&from, &opts)? else {
        return symbol_not_found(output_format, "trace_calls", &from);
    };

    let nodes: Vec<Value> = result
        .nodes
        .iter()
        .map(|entry| {
            node_json(
                &entry.node,
                entry.depth,
                entry.direction,
                entry.via_edge.as_ref(),
            )
        })
        .collect();
    let edges: Vec<Value> = result.edges.iter().map(edge_json).collect();

    let mut payload = Map::new();
    payload.insert("root_symbol".to_owned(), json!(result.root.qualified_name));
    payload.insert("target_symbol".to_owned(), Value::Null);
    payload.insert("direction".to_owned(), json!(direction.as_str()));
    payload.insert("edge_kinds".to_owned(), kinds_json(&edge_kinds));
    payload.insert("depth".to_owned(), json!(max_depth));
    payload.insert("path".to_owned(), path_json(None));
    payload.insert("nodes".to_owned(), Value::Array(nodes));
    payload.insert("edges".to_owned(), Value::Array(edges));
    payload.insert(
        "summary".to_owned(),
        json!({
            "node_count": result.nodes.len(),
            "edge_count": result.edges.len(),
            "max_depth_reached": result.max_depth_reached,
            "path_found": false,
        }),
    );

    let envelope = ToolSuccessEnvelope::new("trace_calls", Value::Object(payload)).with_truncation(
        result.truncated,
        result
            .truncated
            .then_some("trace capped by node budget; raise max_nodes or narrow the scope"),
    );
    normalized_tool_result_value(&envelope, output_format)
}

fn path_payload(
    result: &atlas_store_sqlite::TracePathResult,
    edge_kinds: &[EdgeKind],
    direction: &TraceDirection,
    max_depth: u32,
    output_format: crate::output::OutputFormat,
) -> Result<Value> {
    let (nodes, edges, path_found) = match &result.path {
        Some(path) => (
            path.nodes
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let via = index
                        .checked_sub(1)
                        .and_then(|edge_index| path.edges.get(edge_index));
                    let hop = if index == 0 { "seed" } else { "path" };
                    node_json(node, index as u32, hop, via)
                })
                .collect::<Vec<_>>(),
            path.edges.iter().map(edge_json).collect::<Vec<_>>(),
            true,
        ),
        None => (Vec::new(), Vec::new(), false),
    };

    let mut payload = Map::new();
    payload.insert("root_symbol".to_owned(), json!(result.from.qualified_name));
    payload.insert("target_symbol".to_owned(), json!(result.to.qualified_name));
    payload.insert("direction".to_owned(), json!(direction.as_str()));
    payload.insert("edge_kinds".to_owned(), kinds_json(edge_kinds));
    payload.insert("depth".to_owned(), json!(max_depth));
    payload.insert("path".to_owned(), path_json(result.path.as_ref()));
    payload.insert("nodes".to_owned(), Value::Array(nodes));
    payload.insert("edges".to_owned(), Value::Array(edges));
    payload.insert(
        "summary".to_owned(),
        json!({
            "node_count": result.path.as_ref().map(|path| path.nodes.len()).unwrap_or(0),
            "edge_count": result.path.as_ref().map(|path| path.edges.len()).unwrap_or(0),
            "max_depth_reached": result.path.as_ref().map(|path| path.edges.len() as u32).unwrap_or(0),
            "path_found": path_found,
        }),
    );

    let envelope = ToolSuccessEnvelope::new("trace_calls", Value::Object(payload)).with_truncation(
        result.truncated,
        result
            .truncated
            .then_some("path search capped by node budget; raise max_nodes or max_depth"),
    );
    normalized_tool_result_value(&envelope, output_format)
}

fn path_json(path: Option<&atlas_store_sqlite::TracePath>) -> Value {
    match path {
        Some(path) => json!({
            "found": true,
            "nodes": path
                .nodes
                .iter()
                .map(|node| node.qualified_name.clone())
                .collect::<Vec<_>>(),
            "edges": path
                .edges
                .iter()
                .map(|edge| json!({
                    "from": edge.source_qn,
                    "to": edge.target_qn,
                    "kind": edge.kind.as_str(),
                }))
                .collect::<Vec<_>>(),
        }),
        None => json!({ "found": false, "nodes": [], "edges": [] }),
    }
}

enum QnameResolution {
    Found(String),
    Missing,
    Ambiguous(String),
}

fn resolve_qname(store: &Store, input: &str) -> Result<QnameResolution> {
    if store.node_by_qname(input)?.is_some() {
        return Ok(QnameResolution::Found(input.to_owned()));
    }
    let matches = store.nodes_by_name(input, 25)?;
    match matches.len() {
        0 => Ok(QnameResolution::Missing),
        1 => Ok(QnameResolution::Found(matches[0].qualified_name.clone())),
        _ => {
            let candidates = matches
                .iter()
                .take(5)
                .map(|node| node.qualified_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Ok(QnameResolution::Ambiguous(format!(
                "ambiguous symbol `{input}`; use a qualified name. Candidates: {candidates}"
            )))
        }
    }
}

fn kinds_json(kinds: &[EdgeKind]) -> Value {
    Value::Array(
        kinds
            .iter()
            .map(|kind| json!(kind.as_str()))
            .collect::<Vec<_>>(),
    )
}

fn node_json(node: &Node, depth: u32, direction: &'static str, via: Option<&Edge>) -> Value {
    json!({
        "qualified_name": node.qualified_name,
        "name": node.name,
        "kind": node.kind.as_str(),
        "language": node.language,
        "file": node.file_path,
        "line_start": node.line_start,
        "line_end": node.line_end,
        "is_test": node.is_test,
        "depth": depth,
        "direction": direction,
        "via": via.map(edge_json),
    })
}

fn edge_json(edge: &Edge) -> Value {
    json!({
        "from": edge.source_qn,
        "to": edge.target_qn,
        "kind": edge.kind.as_str(),
        "file": edge.file_path,
        "line": edge.line,
        "confidence": edge.confidence,
        "confidence_tier": edge.confidence_tier,
    })
}

fn invalid_input(
    output_format: crate::output::OutputFormat,
    tool: &str,
    message: String,
    retry_guidance: &str,
) -> Result<Value> {
    let payload = ToolErrorPayload::new(ToolErrorCode::InvalidInput, message)
        .with_tool(tool)
        .with_retry_guidance(retry_guidance);
    tool_execution_error_value(output_format, &payload)
}

fn symbol_not_found(
    output_format: crate::output::OutputFormat,
    tool: &str,
    symbol: &str,
) -> Result<Value> {
    let payload = ToolErrorPayload::new(
        ToolErrorCode::SymbolNotFound,
        format!("symbol not found: {symbol}"),
    )
    .with_tool(tool)
    .with_retry_guidance("Resolve the exact qualified name with resolve_symbol, then retry.");
    tool_execution_error_value(output_format, &payload)
}
