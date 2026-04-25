//! CSS parser backed by `tree-sitter-css`.
//! Grammar source: `tree-sitter/tree-sitter-css`.

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct CssParser;

impl LangParser for CssParser {
    fn language_name(&self) -> &'static str {
        "css"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".css")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_css::LANGUAGE.into())
            .expect("tree-sitter-css grammar failed to load");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count, "css"));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let mut rule_index = 0usize;
            let mut import_index = 0usize;
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                match child.kind() {
                    "import_statement" => {
                        emit_css_import(child, ctx, &mut nodes, &mut edges, &mut import_index)
                    }
                    "rule_set" => {
                        rule_index += 1;
                        emit_rule_set(child, ctx, &mut nodes, &mut edges, rule_index)
                    }
                    _ => {}
                }
            }
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("css".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn emit_css_import(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    import_index: &mut usize,
) {
    *import_index += 1;
    let raw = node_text(node, ctx.source).trim().to_owned();
    let imported = extract_css_import_target(&raw).unwrap_or_else(|| raw.clone());
    let qn = format!("{}::import::css:{}", ctx.rel_path, *import_index);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Import,
        name: imported.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "css".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "source": imported, "statement": raw }),
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, line));
    edges.push(imports_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        line,
        "definite",
    ));
}

fn emit_rule_set(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    rule_index: usize,
) {
    let selectors = find_direct_child(node, "selectors")
        .map(|child| compact_css_text(node_text(child, ctx.source)))
        .unwrap_or_else(|| format!("rule-{rule_index}"));
    let rule_qn = format!("{}::rule::{}", ctx.rel_path, rule_index);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Module,
        name: selectors.clone(),
        qualified_name: rule_qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "css".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "selector": selectors }),
    });
    edges.push(contains_edge(ctx.rel_path, &rule_qn, ctx.rel_path, line));

    if let Some(selectors_node) = find_direct_child(node, "selectors") {
        let mut selector_index = 0usize;
        let mut cursor = selectors_node.walk();
        for child in selectors_node.named_children(&mut cursor) {
            if !matches!(child.kind(), "class_selector" | "id_selector" | "tag_name") {
                continue;
            }
            selector_index += 1;
            let selector = compact_css_text(node_text(child, ctx.source));
            let qn = format!(
                "{}::selector::{}:{}",
                ctx.rel_path, rule_index, selector_index
            );
            nodes.push(Node {
                id: NodeId::UNSET,
                kind: if child.kind() == "class_selector" {
                    NodeKind::Class
                } else {
                    NodeKind::Variable
                },
                name: selector.clone(),
                qualified_name: qn.clone(),
                file_path: ctx.rel_path.to_owned(),
                line_start: start_line(child),
                line_end: end_line(child),
                language: "css".to_owned(),
                parent_name: Some(rule_qn.clone()),
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: ctx.file_hash.to_owned(),
                extra_json: serde_json::json!({ "selector_kind": child.kind() }),
            });
            edges.push(contains_edge(
                &rule_qn,
                &qn,
                ctx.rel_path,
                start_line(child),
            ));
        }
    }

    if let Some(block) = find_direct_child(node, "block") {
        let mut declaration_index = 0usize;
        let mut cursor = block.walk();
        for child in block.named_children(&mut cursor) {
            if child.kind() != "declaration" {
                continue;
            }
            let Some(property) = find_direct_child(child, "property_name") else {
                continue;
            };
            declaration_index += 1;
            let name = compact_css_text(node_text(property, ctx.source));
            let qn = format!(
                "{}::decl::{}:{}",
                ctx.rel_path, rule_index, declaration_index
            );
            nodes.push(Node {
                id: NodeId::UNSET,
                kind: NodeKind::Variable,
                name,
                qualified_name: qn.clone(),
                file_path: ctx.rel_path.to_owned(),
                line_start: start_line(property),
                line_end: end_line(child),
                language: "css".to_owned(),
                parent_name: Some(rule_qn.clone()),
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: ctx.file_hash.to_owned(),
                extra_json: serde_json::json!({ "kind": "declaration" }),
            });
            edges.push(contains_edge(
                &rule_qn,
                &qn,
                ctx.rel_path,
                start_line(child),
            ));
        }
    }
}

fn extract_css_import_target(statement: &str) -> Option<String> {
    let trimmed = statement.trim();
    if let Some(start) = trimmed.find("url(") {
        let rest = &trimmed[start + 4..];
        let end = rest.find(')')?;
        return Some(
            rest[..end]
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_owned(),
        );
    }
    let quote = trimmed.find('"').or_else(|| trimmed.find('\''))?;
    let quote_char = trimmed.as_bytes()[quote] as char;
    let tail = &trimmed[quote + 1..];
    let end = tail.find(quote_char)?;
    Some(tail[..end].to_owned())
}

fn compact_css_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_direct_child<'a>(node: TsNode<'a>, kind: &str) -> Option<TsNode<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
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

fn imports_edge(source_qn: &str, target_qn: &str, file_path: &str, line: u32, tier: &str) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Imports,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(line),
        confidence: 1.0,
        confidence_tier: Some(tier.to_owned()),
        extra_json: serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ParsedFile {
        let (pf, _) = CssParser.parse(&ParseContext {
            rel_path: "assets/app.css",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_rules_selectors_and_imports() {
        let pf = parse("@import url('base.css');\n.button, #app { color: red; }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "base.css")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "assets/app.css::rule::1")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Class && node.name == ".button")
        );
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Imports));
    }
}
