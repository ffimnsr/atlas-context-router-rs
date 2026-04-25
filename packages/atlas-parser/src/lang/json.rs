use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct JsonParser;

impl LangParser for JsonParser {
    fn language_name(&self) -> &'static str {
        "json"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".json")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_json::LANGUAGE.into())
            .expect("tree-sitter-json grammar failed to load");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                match child.kind() {
                    "object" => walk_object(
                        child,
                        ctx.rel_path,
                        ctx.rel_path,
                        "",
                        ctx.source,
                        ctx.file_hash,
                        &mut nodes,
                        &mut edges,
                    ),
                    "array" => walk_array(
                        child,
                        ctx.rel_path,
                        ctx.rel_path,
                        "",
                        ctx.source,
                        ctx.file_hash,
                        &mut nodes,
                        &mut edges,
                    ),
                    _ => {}
                }
            }
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("json".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn file_node(rel_path: &str, file_hash: &str, line_end: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned(),
        qualified_name: rel_path.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: 1,
        line_end,
        language: "json".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    }
}

fn contains_edge(parent_qn: &str, child_qn: &str, file_path: &str, line: u32) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Contains,
        source_qn: parent_qn.to_owned(),
        target_qn: child_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(line),
        confidence: 1.0,
        confidence_tier: Some("definite".to_owned()),
        extra_json: serde_json::Value::Null,
    }
}

/// Extract the content of a JSON string node (strips the surrounding `"` quotes).
fn json_string_content<'s>(node: TsNode<'_>, source: &'s [u8]) -> &'s str {
    let raw = node_text(node, source);
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        &raw[1..raw.len() - 1]
    } else {
        raw
    }
}

fn join_path(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        segment.to_owned()
    } else {
        format!("{parent}.{segment}")
    }
}

/// Atlas NodeKind for a given tree-sitter-json value node kind.
fn value_node_kind(ts_kind: &str) -> NodeKind {
    match ts_kind {
        "object" => NodeKind::Module,
        _ => NodeKind::Variable,
    }
}

/// `extra_json` `value_kind` string following the existing convention.
///
/// Non-object array items always get `"array_item"` regardless of their
/// actual type; objects inside arrays keep `"object"` so the `Module` kind is
/// correctly communicated.
fn value_kind_str(ts_kind: &str, is_array_item: bool) -> &'static str {
    match ts_kind {
        "object" => "object",
        _ if is_array_item => "array_item",
        "array" => "array",
        "string" => "string",
        "number" => "number",
        "true" | "false" => "boolean",
        "null" => "null",
        _ => "array_item",
    }
}

/// Walk a JSON object node, emitting Atlas nodes for each key-value pair.
#[allow(clippy::too_many_arguments)]
fn walk_object(
    node: TsNode<'_>,
    parent_qn: &str,
    rel_path: &str,
    parent_path: &str,
    source: &[u8],
    file_hash: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for pair in node.named_children(&mut cursor) {
        if pair.kind() != "pair" || pair.is_error() {
            continue;
        }
        let Some(key_node) = pair.child_by_field_name("key") else {
            continue;
        };
        let Some(value_node) = pair.child_by_field_name("value") else {
            continue;
        };
        let key = json_string_content(key_node, source);
        let path = join_path(parent_path, key);
        let qn = format!("{rel_path}::key::{path}");
        let line_start = start_line(pair);
        let line_end = end_line(pair);
        let vkind = value_kind_str(value_node.kind(), false);

        nodes.push(Node {
            id: NodeId::UNSET,
            kind: value_node_kind(value_node.kind()),
            name: key.to_owned(),
            qualified_name: qn.clone(),
            file_path: rel_path.to_owned(),
            line_start,
            line_end,
            language: "json".to_owned(),
            parent_name: Some(parent_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: file_hash.to_owned(),
            extra_json: serde_json::json!({
                "path": path,
                "value_kind": vkind,
            }),
        });
        edges.push(contains_edge(parent_qn, &qn, rel_path, line_start));

        match value_node.kind() {
            "object" => {
                walk_object(
                    value_node, &qn, rel_path, &path, source, file_hash, nodes, edges,
                );
            }
            "array" => {
                walk_array(
                    value_node, &qn, rel_path, &path, source, file_hash, nodes, edges,
                );
            }
            _ => {}
        }
    }
}

/// Walk a JSON array node, emitting Atlas nodes for each element.
#[allow(clippy::too_many_arguments)]
fn walk_array(
    node: TsNode<'_>,
    parent_qn: &str,
    rel_path: &str,
    parent_path: &str,
    source: &[u8],
    file_hash: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    let mut index = 0usize;
    for item in node.named_children(&mut cursor) {
        if item.is_error() {
            continue;
        }
        let child_path = format!("{parent_path}[{index}]");
        let child_qn = format!("{rel_path}::key::{child_path}");
        let line_start = start_line(item);
        let line_end = end_line(item);
        let vkind = value_kind_str(item.kind(), true);

        nodes.push(Node {
            id: NodeId::UNSET,
            kind: value_node_kind(item.kind()),
            name: format!("[{index}]"),
            qualified_name: child_qn.clone(),
            file_path: rel_path.to_owned(),
            line_start,
            line_end,
            language: "json".to_owned(),
            parent_name: Some(parent_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: file_hash.to_owned(),
            extra_json: serde_json::json!({
                "path": child_path,
                "value_kind": vkind,
            }),
        });
        edges.push(contains_edge(parent_qn, &child_qn, rel_path, line_start));

        match item.kind() {
            "object" => {
                walk_object(
                    item,
                    &child_qn,
                    rel_path,
                    &child_path,
                    source,
                    file_hash,
                    nodes,
                    edges,
                );
            }
            "array" => {
                walk_array(
                    item,
                    &child_qn,
                    rel_path,
                    &child_path,
                    source,
                    file_hash,
                    nodes,
                    edges,
                );
            }
            _ => {}
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ParseContext;

    fn parse(src: &str) -> ParsedFile {
        let (pf, _) = JsonParser.parse(&ParseContext {
            rel_path: "config/app.json",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_nested_keys_and_arrays() {
        let pf = parse(
            r#"{
  "server": { "host": "localhost", "ports": [8080, 8081] },
  "enabled": true
}
"#,
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "config/app.json::key::server")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "config/app.json::key::server.host")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "config/app.json::key::server.ports[0]")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "config/app.json::key::enabled")
        );
    }

    #[test]
    fn malformed_source_returns_at_least_file_node() {
        // tree-sitter does partial/best-effort extraction on malformed input.
        let pf = parse("{\n  \"server\": [1, 2,\n");
        assert!(!pf.nodes.is_empty());
        assert_eq!(pf.nodes[0].kind, NodeKind::File);
    }

    #[test]
    fn parses_negative_number_values() {
        let pf = parse("{\n  \"threshold\": -12.5e2\n}\n");
        let threshold = pf
            .nodes
            .iter()
            .find(|node| node.qualified_name == "config/app.json::key::threshold")
            .expect("threshold node");

        assert_eq!(threshold.kind, NodeKind::Variable);
        assert_eq!(threshold.extra_json["value_kind"], "number");
        assert_eq!(threshold.line_start, 2);
        assert_eq!(threshold.line_end, 2);
    }

    #[test]
    fn parse_returns_tree_for_incremental_reuse() {
        let src = r#"{"a": 1}"#;
        let (_, tree) = JsonParser.parse(&ParseContext {
            rel_path: "test.json",
            file_hash: "hash",
            source: src.as_bytes(),
            old_tree: None,
        });
        assert!(
            tree.is_some(),
            "tree-sitter tree must be returned for incremental reuse"
        );
    }
}
