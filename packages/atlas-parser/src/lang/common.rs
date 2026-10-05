//! Shared helpers for call-edge construction and callable lookup maps.
//!
//! Language modules previously carried near-identical private copies of
//! `call_edge`, `caller_simple_name`, the `*_call_target` extractors, and
//! `callable_qn_map`.  They live here so behavior stays identical across
//! languages and fixes land once.

use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeKind};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::node_text;

/// Strip module (`::`) and member (`.`) prefixes, returning the simple name.
pub(crate) fn caller_simple_name(caller_qn: &str) -> &str {
    caller_qn
        .rsplit("::")
        .next()
        .unwrap_or(caller_qn)
        .rsplit('.')
        .next()
        .unwrap_or(caller_qn)
}

/// Build a `calls` edge with same-file/text confidence and callee metadata.
pub(crate) fn call_edge(
    caller: &str,
    callee: &str,
    rel_path: &str,
    line: u32,
    text: &str,
    receiver: Option<&str>,
    same_file: bool,
) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: caller.to_owned(),
        target_qn: callee.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: if same_file { 0.8 } else { 0.3 },
        confidence_tier: Some(if same_file { "same_file" } else { "text" }.to_owned()),
        extra_json: serde_json::json!({
            "callee_text": text,
            "callee_name": caller_simple_name(callee),
            "receiver_text": receiver,
        }),
        repo_provenance: None,
    }
}

/// Build a fully-confident edge of `kind` with a caller-supplied confidence tier.
pub(crate) fn tier_edge(
    kind: EdgeKind,
    source_qn: &str,
    target_qn: &str,
    file_path: &str,
    line: u32,
    tier: &str,
) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(line),
        confidence: 1.0,
        confidence_tier: Some(tier.to_owned()),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

/// Build a `contains` edge with definite confidence.
pub(crate) fn contains_edge(parent_qn: &str, child_qn: &str, file_path: &str, line: u32) -> Edge {
    tier_edge(
        EdgeKind::Contains,
        parent_qn,
        child_qn,
        file_path,
        line,
        "definite",
    )
}

/// Extract `(callee_text, callee_name, receiver_text)` from a call target.
///
/// `member_kind` is the grammar's member-expression node kind and
/// `callee_field` / `receiver_field` its field names (for example
/// `"member_expression"`, `"property"`, `"object"` for JS/TS).
pub(crate) fn call_target(
    node: TsNode<'_>,
    source: &[u8],
    member_kind: &str,
    callee_field: &str,
    receiver_field: &str,
) -> Option<(String, String, Option<String>)> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source).to_owned();
            Some((name.clone(), name, None))
        }
        kind if kind == member_kind => {
            let callee = node.child_by_field_name(callee_field)?;
            let receiver = node.child_by_field_name(receiver_field)?;
            let callee_name = node_text(callee, source).to_owned();
            let receiver_text = node_text(receiver, source).to_owned();
            Some((
                node_text(node, source).to_owned(),
                callee_name,
                Some(receiver_text),
            ))
        }
        _ => None,
    }
}

/// Build a `name -> qualified_name` map of callable nodes.
///
/// When `include_tests` is true, `NodeKind::Test` definitions are treated as
/// callable targets too.
pub(crate) fn callable_qn_map(nodes: &[Node], include_tests: bool) -> HashMap<String, String> {
    let mut map = HashMap::with_capacity(nodes.len());
    for node in nodes {
        let callable = matches!(node.kind, NodeKind::Function | NodeKind::Method)
            || (include_tests && node.kind == NodeKind::Test);
        if callable {
            map.insert(node.name.clone(), node.qualified_name.clone());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_simple_name_strips_module_and_member_prefixes() {
        assert_eq!(caller_simple_name("src/lib.rs::method::Store::run"), "run");
        assert_eq!(caller_simple_name("pkg.module.func"), "func");
        assert_eq!(caller_simple_name("bare"), "bare");
    }

    #[test]
    fn tier_edge_sets_full_confidence_and_tier() {
        let edge = tier_edge(
            EdgeKind::Imports,
            "src/lib.rs",
            "pkg::thing",
            "src/lib.rs",
            7,
            "explicit_import",
        );
        assert_eq!(edge.kind, EdgeKind::Imports);
        assert_eq!(edge.source_qn, "src/lib.rs");
        assert_eq!(edge.target_qn, "pkg::thing");
        assert_eq!(edge.line, Some(7));
        assert_eq!(edge.confidence, 1.0);
        assert_eq!(edge.confidence_tier.as_deref(), Some("explicit_import"));
        assert_eq!(edge.extra_json, serde_json::Value::Null);
    }

    #[test]
    fn contains_edge_is_definite() {
        let edge = contains_edge("parent", "child", "src/lib.rs", 2);
        assert_eq!(edge.kind, EdgeKind::Contains);
        assert_eq!(edge.confidence, 1.0);
        assert_eq!(edge.confidence_tier.as_deref(), Some("definite"));
    }

    #[test]
    fn call_edge_encodes_same_file_confidence_and_metadata() {
        let same_file = call_edge(
            "caller",
            "pkg::callee",
            "src/lib.rs",
            9,
            "callee()",
            Some("self"),
            true,
        );
        assert_eq!(same_file.kind, EdgeKind::Calls);
        assert_eq!(same_file.confidence, 0.8);
        assert_eq!(same_file.confidence_tier.as_deref(), Some("same_file"));
        let extra = same_file.extra_json.as_object().unwrap();
        assert_eq!(
            extra.get("callee_name").and_then(|v| v.as_str()),
            Some("callee")
        );
        assert_eq!(
            extra.get("callee_text").and_then(|v| v.as_str()),
            Some("callee()")
        );
        assert_eq!(
            extra.get("receiver_text").and_then(|v| v.as_str()),
            Some("self")
        );

        let cross_file = call_edge("caller", "callee", "src/lib.rs", 9, "callee", None, false);
        assert_eq!(cross_file.confidence, 0.3);
        assert_eq!(cross_file.confidence_tier.as_deref(), Some("text"));
    }
}
