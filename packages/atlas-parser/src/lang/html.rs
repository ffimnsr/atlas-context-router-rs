//! HTML parser backed by `tree-sitter-html`.
//! Grammar source: `tree-sitter/tree-sitter-html`.

use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct HtmlParser;

impl LangParser for HtmlParser {
    fn language_name(&self) -> &'static str {
        "html"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".html") || path.ends_with(".htm")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .expect("tree-sitter-html grammar failed to load");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count, "html"));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let document_qn = format!("{}::document", ctx.rel_path);
            nodes.push(Node {
                id: NodeId::UNSET,
                kind: NodeKind::Module,
                name: "document".to_owned(),
                qualified_name: document_qn.clone(),
                file_path: ctx.rel_path.to_owned(),
                line_start: 1,
                line_end: line_count,
                language: "html".to_owned(),
                parent_name: Some(ctx.rel_path.to_owned()),
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: ctx.file_hash.to_owned(),
                extra_json: serde_json::json!({ "kind": "document" }),
            });
            edges.push(contains_edge(ctx.rel_path, &document_qn, ctx.rel_path, 1));

            let mut import_index = 0usize;
            walk_html_children(
                root,
                ctx,
                &document_qn,
                "document",
                &mut nodes,
                &mut edges,
                &mut import_index,
            );
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("html".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn walk_html_children(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    parent_path: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    import_index: &mut usize,
) {
    let mut counts = HashMap::<String, usize>::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let tag_name = match child.kind() {
            "element" => start_tag_name(child, ctx.source),
            "self_closing_tag" => tag_name_from_tag(child, ctx.source),
            "start_tag" => tag_name_from_tag(child, ctx.source).filter(|tag| is_void_html_tag(tag)),
            "script_element" => Some("script".to_owned()),
            "style_element" => Some("style".to_owned()),
            _ => None,
        };
        let Some(tag_name) = tag_name else {
            continue;
        };

        let entry = counts.entry(tag_name.clone()).or_insert(0);
        *entry += 1;
        let child_path = format!(
            "{}.{}[{}]",
            parent_path,
            sanitize_segment(&tag_name),
            *entry
        );
        let child_qn = format!("{}::tag::{}", ctx.rel_path, child_path);
        let line = start_line(child);
        nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Module,
            name: tag_name.clone(),
            qualified_name: child_qn.clone(),
            file_path: ctx.rel_path.to_owned(),
            line_start: line,
            line_end: end_line(child),
            language: "html".to_owned(),
            parent_name: Some(parent_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: ctx.file_hash.to_owned(),
            extra_json: serde_json::json!({
                "tag": tag_name,
                "path": child_path,
            }),
        });
        edges.push(contains_edge(parent_qn, &child_qn, ctx.rel_path, line));

        emit_html_imports(child, &child_qn, ctx, nodes, edges, import_index);
        if matches!(child.kind(), "element" | "script_element" | "style_element") {
            walk_html_children(
                child,
                ctx,
                &child_qn,
                &child_path,
                nodes,
                edges,
                import_index,
            );
        }
    }
}

fn emit_html_imports(
    node: TsNode<'_>,
    container_qn: &str,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    import_index: &mut usize,
) {
    let tag = if matches!(node.kind(), "self_closing_tag" | "start_tag") {
        tag_name_from_tag(node, ctx.source)
    } else {
        start_tag_name(node, ctx.source)
    };
    let Some(tag) = tag else {
        return;
    };

    let attr_container = if matches!(node.kind(), "self_closing_tag" | "start_tag") {
        Some(node)
    } else {
        find_direct_child(node, "start_tag")
    };
    let Some(attr_container) = attr_container else {
        return;
    };

    let mut cursor = attr_container.walk();
    for child in attr_container.named_children(&mut cursor) {
        if child.kind() != "attribute" {
            continue;
        }
        let Some((name, value)) = parse_html_attribute(child, ctx.source) else {
            continue;
        };
        if !matches!(
            name.as_str(),
            "src" | "href" | "srcset" | "data-src" | "data-href"
        ) {
            continue;
        }

        *import_index += 1;
        let qn = format!("{}::import::html:{}", ctx.rel_path, *import_index);
        let line = start_line(child);
        nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Import,
            name: value.clone(),
            qualified_name: qn.clone(),
            file_path: ctx.rel_path.to_owned(),
            line_start: line,
            line_end: end_line(child),
            language: "html".to_owned(),
            parent_name: Some(container_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: ctx.file_hash.to_owned(),
            extra_json: serde_json::json!({
                "attribute": name,
                "imported": value,
                "tag": tag,
            }),
        });
        edges.push(contains_edge(container_qn, &qn, ctx.rel_path, line));
        edges.push(Edge {
            id: 0,
            kind: EdgeKind::Imports,
            source_qn: container_qn.to_owned(),
            target_qn: qn,
            file_path: ctx.rel_path.to_owned(),
            line: Some(line),
            confidence: 0.9,
            confidence_tier: Some("attribute".to_owned()),
            extra_json: serde_json::Value::Null,
        });
    }
}

fn find_direct_child<'a>(node: TsNode<'a>, kind: &str) -> Option<TsNode<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn start_tag_name(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let start = find_direct_child(node, "start_tag")?;
    tag_name_from_tag(start, source)
}

fn tag_name_from_tag(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == "tag_name")
        .map(|child| node_text(child, source).to_owned())
}

fn parse_html_attribute(node: TsNode<'_>, source: &[u8]) -> Option<(String, String)> {
    let mut cursor = node.walk();
    let mut name = None;
    let mut value = None;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "attribute_name" => name = Some(node_text(child, source).to_owned()),
            "quoted_attribute_value" | "attribute_value" => {
                value = Some(trim_quotes(node_text(child, source)).to_owned())
            }
            _ => {}
        }
    }
    if let (Some(name), Some(value)) = (name, value) {
        return Some((name, value));
    }

    let raw = node_text(node, source).trim();
    let (raw_name, raw_value) = raw.split_once('=')?;
    Some((
        raw_name.trim().to_owned(),
        trim_quotes(raw_value).to_owned(),
    ))
}

fn trim_quotes(text: &str) -> &str {
    text.trim()
        .trim_end_matches('/')
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
}

fn sanitize_segment(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn is_void_html_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn file_node(rel_path: &str, file_hash: &str, line_end: u32, language: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned(),
        qualified_name: rel_path.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: 1,
        line_end,
        language: language.to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ParsedFile {
        let (pf, _) = HtmlParser.parse(&ParseContext {
            rel_path: "site/index.html",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_elements_and_import_attributes() {
        let pf = parse(
            "<!doctype html><html><body><script src=\"app.js\"></script><img src=\"hero.png\" /></body></html>",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "site/index.html::document")
        );
        assert!(pf.nodes.iter().any(|node| node.name == "script"));
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "app.js")
        );
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Imports));
    }
}
