//! `atlas trace`: directional call-chain tracing from the CLI.

use std::fmt::Write as _;
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use atlas_core::{Edge, EdgeKind, GraphToolRequirement};
use atlas_review::normalize_qn_kind_tokens;
use atlas_store_sqlite::{
    Store, TraceDirection, TraceOptions, TracePath, TracePathResult, TraceResult,
};
use serde_json::json;

use crate::cli::{Cli, Command};

use super::{
    check_graph_readiness, db_path, derive_graph_readiness, derive_graph_readiness_open_failed,
    print_json, readiness_overrides, resolve_repo,
};

pub fn run_trace(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let (from, to, direction, edge_kinds, depth, max_nodes, allow_stale, allow_partial) =
        match &cli.command {
            Command::Trace {
                from,
                to,
                direction,
                edge_kinds,
                depth,
                max_nodes,
                allow_stale,
                allow_partial,
            } => (
                from.clone(),
                to.clone(),
                direction.clone(),
                edge_kinds.clone(),
                *depth,
                *max_nodes,
                *allow_stale,
                *allow_partial,
            ),
            _ => unreachable!(),
        };

    let direction = TraceDirection::from_str(&direction).map_err(anyhow::Error::msg)?;
    let mut kinds = Vec::new();
    if edge_kinds.is_empty() {
        kinds.push(EdgeKind::Calls);
    } else {
        for raw in edge_kinds {
            kinds.push(EdgeKind::from_str(&raw).map_err(anyhow::Error::msg)?);
        }
    }

    let store = match Store::open(&db_path) {
        Ok(store) => store,
        Err(error) => {
            let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &error.to_string());
            check_graph_readiness(
                &readiness,
                GraphToolRequirement::Traversal,
                readiness_overrides(false, false),
                "trace",
                cli,
            )?;
            return Err(error).with_context(|| format!("cannot open database at {db_path}"));
        }
    };

    let readiness = derive_graph_readiness(&store, &repo, &db_path);
    if let Some(warning) = check_graph_readiness(
        &readiness,
        GraphToolRequirement::Traversal,
        readiness_overrides(allow_stale, allow_partial),
        "trace",
        cli,
    )? {
        eprintln!("Warning: {warning}");
    }

    let from = resolve_qname(&store, &from)?;
    let opts = TraceOptions {
        direction,
        edge_kinds: &kinds,
        max_depth: depth,
        max_nodes: max_nodes.max(1),
    };

    if let Some(to) = to {
        let to = resolve_qname(&store, &to)?;
        let result = store
            .trace_call_path(&from, &to, &opts)
            .context("call path trace failed")?
            .with_context(|| format!("symbol not found: {from} or {to}"))?;
        if cli.json {
            print_json("trace", path_json(&result, direction, &kinds, depth))?;
        } else {
            println!("{}", render_path_text(&result, direction, &kinds));
        }
        return Ok(());
    }

    let result = store
        .trace_calls(&from, &opts)
        .context("call trace failed")?
        .with_context(|| format!("symbol not found: {from}"))?;
    if cli.json {
        print_json("trace", trace_json(&result, direction, &kinds, depth))?;
    } else {
        println!("{}", render_trace_text(&result, direction, &kinds));
    }
    Ok(())
}

fn resolve_qname(store: &Store, input: &str) -> Result<String> {
    let normalized = normalize_qn_kind_tokens(input);
    if store.node_by_qname(&normalized)?.is_some() {
        return Ok(normalized);
    }
    let matches = store.nodes_by_name(&normalized, 25)?;
    match matches.len() {
        0 => bail!("symbol not found: {input}"),
        1 => Ok(matches[0].qualified_name.clone()),
        _ => {
            let candidates = matches
                .iter()
                .take(5)
                .map(|node| node.qualified_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("ambiguous symbol `{input}`; use a qualified name. Candidates: {candidates}")
        }
    }
}

fn trace_json(
    result: &TraceResult,
    direction: TraceDirection,
    kinds: &[EdgeKind],
    depth: u32,
) -> serde_json::Value {
    json!({
        "root_symbol": result.root.qualified_name,
        "target_symbol": serde_json::Value::Null,
        "direction": direction.as_str(),
        "edge_kinds": kinds.iter().map(|kind| kind.as_str()).collect::<Vec<_>>(),
        "depth": depth,
        "nodes": result.nodes.iter().map(|entry| {
            json!({
                "qualified_name": entry.node.qualified_name,
                "name": entry.node.name,
                "kind": entry.node.kind.as_str(),
                "language": entry.node.language,
                "file": entry.node.file_path,
                "line_start": entry.node.line_start,
                "line_end": entry.node.line_end,
                "is_test": entry.node.is_test,
                "depth": entry.depth,
                "direction": entry.direction,
                "via": entry.via_edge.as_ref().map(edge_json),
            })
        }).collect::<Vec<_>>(),
        "edges": result.edges.iter().map(edge_json).collect::<Vec<_>>(),
        "summary": {
            "node_count": result.nodes.len(),
            "edge_count": result.edges.len(),
            "max_depth_reached": result.max_depth_reached,
            "path_found": false,
        },
        "truncated": result.truncated,
    })
}

fn path_json(
    result: &TracePathResult,
    direction: TraceDirection,
    kinds: &[EdgeKind],
    depth: u32,
) -> serde_json::Value {
    json!({
        "root_symbol": result.from.qualified_name,
        "target_symbol": result.to.qualified_name,
        "direction": direction.as_str(),
        "edge_kinds": kinds.iter().map(|kind| kind.as_str()).collect::<Vec<_>>(),
        "depth": depth,
        "path": {
            "found": result.path.is_some(),
            "nodes": result.path.as_ref().map(|path| {
                path.nodes.iter().map(|node| node.qualified_name.clone()).collect::<Vec<_>>()
            }).unwrap_or_default(),
        },
        "edges": result.path.as_ref().map(|path| {
            path.edges.iter().map(edge_json).collect::<Vec<_>>()
        }).unwrap_or_default(),
        "summary": {
            "node_count": result.path.as_ref().map(|path| path.nodes.len()).unwrap_or(0),
            "edge_count": result.path.as_ref().map(|path| path.edges.len()).unwrap_or(0),
            "max_depth_reached": result.path.as_ref().map(|path| path.edges.len()).unwrap_or(0),
            "path_found": result.path.is_some(),
        },
        "truncated": result.truncated,
    })
}

fn edge_json(edge: &Edge) -> serde_json::Value {
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

fn render_trace_text(
    result: &TraceResult,
    direction: TraceDirection,
    kinds: &[EdgeKind],
) -> String {
    let kinds = kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Trace from `{}` ({}, {}, depth {})",
        result.root.qualified_name,
        direction.as_str(),
        kinds,
        result.max_depth_reached
    );
    for entry in &result.nodes {
        let marker = match entry.direction {
            "seed" => "*",
            "inbound" => "<-",
            _ => "->",
        };
        let indent = "  ".repeat(entry.depth as usize);
        let _ = write!(
            out,
            "{indent}{marker} {}  {}  {}:{}",
            entry.node.qualified_name,
            entry.node.kind.as_str(),
            entry.node.file_path,
            entry.node.line_start
        );
        if let Some(edge) = &entry.via_edge {
            let _ = write!(
                out,
                "  {} at {}:{}",
                edge.kind.as_str(),
                edge.file_path,
                edge.line
                    .map(|line| line.to_string())
                    .unwrap_or_else(|| "?".to_owned())
            );
        }
        out.push('\n');
    }
    if result.truncated {
        out.push_str("... truncated by node budget; raise --max-nodes or narrow the scope\n");
    }
    out.trim_end().to_owned()
}

fn render_path_text(
    result: &TracePathResult,
    direction: TraceDirection,
    kinds: &[EdgeKind],
) -> String {
    let kinds = kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let mut out = String::new();
    match &result.path {
        Some(path) => {
            let _ = writeln!(
                out,
                "Path `{}` -> `{}` ({}, {}): {} hop(s)",
                result.from.qualified_name,
                result.to.qualified_name,
                direction.as_str(),
                kinds,
                path.edges.len()
            );
            render_path_nodes(&mut out, path);
        }
        None => {
            let _ = write!(
                out,
                "No path from `{}` to `{}` ({}, {}, depth {})",
                result.from.qualified_name,
                result.to.qualified_name,
                direction.as_str(),
                kinds,
                result.visited_nodes
            );
        }
    }
    if result.truncated {
        out.push_str("\n... path search truncated by node budget; raise --max-nodes");
    }
    out
}

fn render_path_nodes(out: &mut String, path: &TracePath) {
    for (index, node) in path.nodes.iter().enumerate() {
        if index == 0 {
            let _ = writeln!(
                out,
                "* {}  {}  {}:{}",
                node.qualified_name,
                node.kind.as_str(),
                node.file_path,
                node.line_start
            );
            continue;
        }
        let edge = &path.edges[index - 1];
        let marker = if edge.source_qn == path.nodes[index - 1].qualified_name {
            "->"
        } else {
            "<-"
        };
        let _ = writeln!(
            out,
            "  {marker} {}  {}  {}:{}  {} at {}:{}",
            node.qualified_name,
            node.kind.as_str(),
            node.file_path,
            node.line_start,
            edge.kind.as_str(),
            edge.file_path,
            edge.line
                .map(|line| line.to_string())
                .unwrap_or_else(|| "?".to_owned())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::Node;
    use atlas_core::kinds::NodeKind;
    use atlas_core::model::NodeId;
    use atlas_store_sqlite::TraceNode;

    fn node(name: &str, file: &str) -> Node {
        Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: name.to_owned(),
            qualified_name: format!("{file}::fn::{name}"),
            file_path: file.to_owned(),
            line_start: 1,
            line_end: 9,
            language: "rust".to_owned(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "h".to_owned(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    fn edge(from: &Node, to: &Node) -> Edge {
        Edge {
            id: 1,
            kind: EdgeKind::Calls,
            source_qn: from.qualified_name.clone(),
            target_qn: to.qualified_name.clone(),
            file_path: from.file_path.clone(),
            line: Some(4),
            confidence: 1.0,
            confidence_tier: None,
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    #[test]
    fn render_trace_marks_direction_and_site() {
        let a = node("a", "a.rs");
        let b = node("b", "b.rs");
        let result = TraceResult {
            root: a.clone(),
            nodes: vec![
                TraceNode {
                    node: a.clone(),
                    depth: 0,
                    direction: "seed",
                    via_edge: None,
                },
                TraceNode {
                    node: b.clone(),
                    depth: 1,
                    direction: "outbound",
                    via_edge: Some(edge(&a, &b)),
                },
            ],
            edges: vec![edge(&a, &b)],
            truncated: false,
            max_depth_reached: 1,
        };
        let text = render_trace_text(&result, TraceDirection::Down, &[EdgeKind::Calls]);
        assert!(text.contains("* a.rs::fn::a"));
        assert!(text.contains("  -> b.rs::fn::b"));
        assert!(text.contains("calls at a.rs:4"));
    }

    #[test]
    fn render_path_lists_hops() {
        let a = node("a", "a.rs");
        let b = node("b", "b.rs");
        let result = TracePathResult {
            from: a.clone(),
            to: b.clone(),
            path: Some(TracePath {
                nodes: vec![a.clone(), b.clone()],
                edges: vec![edge(&a, &b)],
            }),
            visited_nodes: 2,
            truncated: false,
        };
        let text = render_path_text(&result, TraceDirection::Down, &[EdgeKind::Calls]);
        assert!(text.contains("1 hop(s)"));
        assert!(text.contains("-> b.rs::fn::b"));
    }

    #[test]
    fn render_unreachable_path_reports_missing_route() {
        let a = node("a", "a.rs");
        let b = node("b", "b.rs");
        let result = TracePathResult {
            from: a,
            to: b,
            path: None,
            visited_nodes: 1,
            truncated: false,
        };
        let text = render_path_text(&result, TraceDirection::Down, &[EdgeKind::Calls]);
        assert!(text.contains("No path from"));
    }
}
