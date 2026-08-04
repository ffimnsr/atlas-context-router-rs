use std::collections::HashMap;
use tree_sitter::Node as TsNode;

use atlas_core::{Edge, EdgeKind, Node, NodeKind};

use crate::ast_helpers::{node_text, start_line};

use super::facts::{RustNodeKey, node_key, rust_query_matches};

// ---------------------------------------------------------------------------
// Same-file call resolution
// ---------------------------------------------------------------------------

/// Walk `root` looking for call and method-call expressions.
/// Emits `Calls` edges (confidence=0.8, tier="same_file") for any call whose
/// callee name matches a function or method defined in the same file.
pub(super) fn resolve_same_file_calls(
    root: TsNode<'_>,
    source: &[u8],
    rel_path: &str,
    nodes: &[Node],
) -> Vec<Edge> {
    let callables = collect_callables(nodes);
    let call_sites = extract_rust_call_sites(root, source)
        .unwrap_or_else(|err| panic!("rust call query failed: {err}"));

    let mut edges = Vec::new();

    for site in call_sites {
        let Some(caller_qn) = caller_qn_for_line(&callables, start_line(site.node)) else {
            continue;
        };
        let called = match (site.receiver_node, site.method_node) {
            (Some(receiver_node), Some(method_node)) => Some((
                node_text(site.node, source).to_owned(),
                node_text(method_node, source).to_owned(),
                Some(node_text(receiver_node, source).to_owned()),
            )),
            _ => rust_call_target(site.target_node, source),
        };
        let Some((text, name, receiver)) = called else {
            continue;
        };
        if !should_emit_rust_call(&name) {
            continue;
        }
        if is_self_call(caller_qn, &name, receiver.as_deref()) {
            continue;
        }
        if let Some(callee_qn) = resolve_local_callee(caller_qn, &name, &callables)
            && callee_qn != caller_qn
        {
            edges.push(call_edge(
                caller_qn,
                &callee_qn,
                rel_path,
                start_line(site.node),
                &text,
                receiver.as_deref(),
                true,
            ));
        } else if !text.is_empty() {
            edges.push(call_edge(
                caller_qn,
                &text,
                rel_path,
                start_line(site.node),
                &text,
                receiver.as_deref(),
                false,
            ));
        }
    }

    edges
}

#[derive(Clone)]
struct CallableNode {
    qn: String,
    name: String,
    parent_qn: String,
    line_start: u32,
    line_end: u32,
}

#[derive(Clone, Copy, Debug)]
struct RustCallSite<'tree> {
    node: TsNode<'tree>,
    target_node: TsNode<'tree>,
    receiver_node: Option<TsNode<'tree>>,
    method_node: Option<TsNode<'tree>>,
}

fn collect_callables(nodes: &[Node]) -> Vec<CallableNode> {
    nodes
        .iter()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::Test
            )
        })
        .map(|n| CallableNode {
            qn: n.qualified_name.clone(),
            name: n.name.clone(),
            parent_qn: n.parent_name.clone().unwrap_or_else(|| n.file_path.clone()),
            line_start: n.line_start,
            line_end: n.line_end,
        })
        .collect()
}

fn extract_rust_call_sites<'tree>(
    root: TsNode<'tree>,
    source: &'tree [u8],
) -> Result<Vec<RustCallSite<'tree>>, String> {
    let matches = rust_query_matches(root, source)?;
    let mut call_sites: HashMap<RustNodeKey, RustCallSite<'tree>> = HashMap::new();

    for group in matches {
        let mut call_node = None;
        let mut target_node = None;
        let mut receiver_node = None;
        let mut method_node = None;

        for capture in &group.captures {
            match capture.name.as_str() {
                "atlas.call" => call_node = Some(capture.node),
                "atlas.call.target" => target_node = Some(capture.node),
                "atlas.call.receiver" => receiver_node = Some(capture.node),
                "atlas.call.method" => method_node = Some(capture.node),
                _ => {}
            }
        }

        if let Some(node) = call_node {
            let site = call_sites.entry(node_key(node)).or_insert(RustCallSite {
                node,
                target_node: node,
                receiver_node: None,
                method_node: None,
            });
            if let Some(target_node) = target_node {
                site.target_node = target_node;
            }
            site.receiver_node = site.receiver_node.or(receiver_node);
            site.method_node = site.method_node.or(method_node);
        }
    }

    let mut call_sites = call_sites
        .into_values()
        .filter(|site| site.target_node != site.node || site.receiver_node.is_some())
        .collect::<Vec<_>>();
    call_sites.sort_by_key(|site| (site.node.start_byte(), site.node.end_byte()));
    Ok(call_sites)
}

fn caller_qn_for_line(callables: &[CallableNode], line: u32) -> Option<&str> {
    callables
        .iter()
        .filter(|callable| callable.line_start <= line && line <= callable.line_end)
        .min_by_key(|callable| {
            (
                callable.line_end.saturating_sub(callable.line_start),
                callable.line_start,
            )
        })
        .map(|callable| callable.qn.as_str())
}

fn resolve_local_callee(caller_qn: &str, name: &str, callables: &[CallableNode]) -> Option<String> {
    let mut candidates = callables.iter().filter(|callable| callable.name == name);
    let first = candidates.next()?;
    let second = candidates.next();
    if second.is_none() {
        return Some(first.qn.clone());
    }

    let caller_parent_chain = scope_chain_for_qn(caller_qn);
    for parent in &caller_parent_chain {
        if let Some(matched) = callables
            .iter()
            .find(|callable| callable.name == name && callable.parent_qn == *parent)
        {
            return Some(matched.qn.clone());
        }
    }

    callables
        .iter()
        .find(|callable| callable.name == name && callable.parent_qn == caller_parent_chain[0])
        .map(|callable| callable.qn.clone())
}

fn scope_chain_for_qn(qn: &str) -> Vec<String> {
    let Some((prefix, tail)) = qn
        .split_once("::fn::")
        .or_else(|| qn.split_once("::method::"))
    else {
        return vec![qn.to_owned()];
    };
    let mut scopes = Vec::new();
    let mut parts: Vec<&str> = tail.split("::").collect();
    if parts.len() > 1 {
        parts.pop();
    }
    while !parts.is_empty() {
        scopes.push(format!("{prefix}::module::{}", parts.join("::")));
        parts.pop();
    }
    scopes.push(prefix.to_owned());
    scopes
}

fn rust_call_target(node: TsNode<'_>, source: &[u8]) -> Option<(String, String, Option<String>)> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source).to_owned();
            Some((name.clone(), name, None))
        }
        "generic_function" => node
            .child_by_field_name("function")
            .and_then(|function| rust_call_target(function, source)),
        "field_expression" => {
            let receiver = node.child_by_field_name("value")?;
            let method = node.child_by_field_name("field")?;
            Some((
                node_text(node, source).to_owned(),
                node_text(method, source).to_owned(),
                Some(node_text(receiver, source).to_owned()),
            ))
        }
        "scoped_identifier" => {
            let text = node_text(node, source).to_owned();
            let (receiver_text, callee_name) = text.rsplit_once("::")?;
            let receiver_text = receiver_text.to_owned();
            let callee_name = callee_name.to_owned();
            Some((text, callee_name, Some(receiver_text)))
        }
        _ => None,
    }
}

fn should_emit_rust_call(callee_name: &str) -> bool {
    callee_name
        .chars()
        .next()
        .is_some_and(|ch| !ch.is_uppercase())
}

fn is_self_call(caller_qn: &str, callee_name: &str, receiver: Option<&str>) -> bool {
    if receiver.is_some() {
        return false;
    }
    caller_simple_name(caller_qn) == callee_name
}

fn caller_simple_name(caller_qn: &str) -> &str {
    caller_qn
        .rsplit("::")
        .next()
        .unwrap_or(caller_qn)
        .rsplit('.')
        .next()
        .unwrap_or(caller_qn)
}

fn call_edge(
    caller_qn: &str,
    callee_qn: &str,
    rel_path: &str,
    line: u32,
    text: &str,
    receiver: Option<&str>,
    same_file: bool,
) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: caller_qn.to_owned(),
        target_qn: callee_qn.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: if same_file { 0.8 } else { 0.3 },
        confidence_tier: Some(if same_file { "same_file" } else { "text" }.to_owned()),
        extra_json: serde_json::json!({
            "callee_text": text,
            "callee_name": caller_simple_name(callee_qn),
            "receiver_text": receiver,
        }),
        repo_provenance: None,
    }
}
