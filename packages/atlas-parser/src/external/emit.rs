//! Shared node and edge emitters for external parsers.
//!
//! Query mode and rule mode must produce identical graph shapes for imports,
//! attributes, qualified names, and reference edges. These helpers are the
//! single implementation both modes call.

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::node_text;
use crate::lang::common::{contains_edge, tier_edge};

use super::ExternalQnStyle;

/// Qualified name for a symbol under the configured QN style.
pub(super) fn symbol_qn(
    style: ExternalQnStyle,
    rel_path: &str,
    kind: NodeKind,
    name: &str,
    parent_qn: &str,
    owner_name: Option<&str>,
) -> String {
    match style {
        ExternalQnStyle::Nested => format!("{parent_qn}::{}::{name}", kind.as_str()),
        ExternalQnStyle::FlatOwner => match owner_name {
            Some(owner) => format!("{rel_path}::{}::{owner}.{name}", kind.as_str()),
            None => format!("{rel_path}::{}::{name}", kind.as_str()),
        },
    }
}

/// Parent name recorded on a symbol node for the configured QN style.
pub(super) fn symbol_parent_name(
    style: ExternalQnStyle,
    rel_path: &str,
    parent_qn: &str,
    owner_qn: Option<&str>,
) -> Option<String> {
    if style == ExternalQnStyle::FlatOwner {
        owner_qn.map(str::to_owned)
    } else {
        (parent_qn != rel_path).then_some(parent_qn.to_owned())
    }
}

/// Emit an import node plus its `Imports`/`Contains` edges from file scope.
#[allow(clippy::too_many_arguments)]
pub(super) fn import_node(
    rel_path: &str,
    file_hash: &str,
    language: &str,
    name: &str,
    line_start: u32,
    line_end: u32,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let qualified_name = format!("{rel_path}::import::{name}");
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Import,
        name: name.to_owned(),
        qualified_name: qualified_name.clone(),
        file_path: rel_path.to_owned(),
        line_start,
        line_end,
        language: language.to_owned(),
        parent_name: Some(rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: file_hash.to_owned(),
        extra_json: serde_json::json!({ "imported": name }),
        repo_provenance: None,
    });
    edges.push(contains_edge(
        rel_path,
        &qualified_name,
        rel_path,
        line_start,
    ));
    edges.push(tier_edge(
        EdgeKind::Imports,
        rel_path,
        &qualified_name,
        rel_path,
        line_start,
        "explicit_import",
    ));
}

/// Emit an attribute variable node attached to `owner_qn`.
#[allow(clippy::too_many_arguments)]
pub(super) fn attribute_node(
    rel_path: &str,
    file_hash: &str,
    language: &str,
    name: &str,
    line_start: u32,
    line_end: u32,
    owner_qn: &str,
    attribute_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    *attribute_index += 1;
    let qualified_name = format!("{owner_qn}::attribute::{name}#{attribute_index}");
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Variable,
        name: name.to_owned(),
        qualified_name: qualified_name.clone(),
        file_path: rel_path.to_owned(),
        line_start,
        line_end,
        language: language.to_owned(),
        parent_name: Some(owner_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: Some("attribute".to_owned()),
        is_test: false,
        file_hash: file_hash.to_owned(),
        extra_json: serde_json::json!({ "kind": "attribute" }),
        repo_provenance: None,
    });
    edges.push(contains_edge(
        owner_qn,
        &qualified_name,
        rel_path,
        line_start,
    ));
}

/// Reference edge with syntax-only confidence.
pub(super) fn reference_edge(
    source_qn: &str,
    target_qn: &str,
    rel_path: &str,
    line: u32,
    same_file: bool,
) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::References,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: 0.75,
        confidence_tier: Some(if same_file { "same_file" } else { "text" }.to_owned()),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

/// Last named child text, used as a callee/import fallback.
pub(super) fn named_child_text<'s>(node: TsNode<'_>, source: &'s [u8]) -> Option<&'s str> {
    let mut cursor = node.walk();
    let mut last: Option<&str> = None;
    for child in node.children(&mut cursor) {
        if child.is_named() {
            last = Some(node_text(child, source));
        }
    }
    last
}
