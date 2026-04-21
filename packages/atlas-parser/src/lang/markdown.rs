//! Markdown parser backed by `tree-sitter-md`.
//! Grammar source: `tree-sitter-grammars/tree-sitter-markdown`.

use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use regex::Regex;
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct MarkdownParser;

impl LangParser for MarkdownParser {
    fn language_name(&self) -> &'static str {
        "markdown"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".md") || path.ends_with(".markdown")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .expect("tree-sitter-md grammar failed to load");

        let tree = parser.parse(ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count, "markdown"));

        if let Some(ref tree) = tree {
            let document_qn = format!("{}::document", ctx.rel_path);
            nodes.push(Node {
                id: NodeId::UNSET,
                kind: NodeKind::Module,
                name: "document".to_owned(),
                qualified_name: document_qn.clone(),
                file_path: ctx.rel_path.to_owned(),
                line_start: 1,
                line_end: line_count,
                language: "markdown".to_owned(),
                parent_name: Some(ctx.rel_path.to_owned()),
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: ctx.file_hash.to_owned(),
                extra_json: serde_json::json!({ "kind": "document" }),
            });
            edges.push(contains_edge(ctx.rel_path, &document_qn, ctx.rel_path, 1));

            let mut link_index = 0usize;
            let mut code_index = 0usize;
            let root = tree.root_node();
            let mut duplicate_counts = HashMap::new();
            let mut heading_stack: Vec<(u8, String, String)> = Vec::new();
            walk_markdown_blocks(
                root,
                ctx,
                &document_qn,
                &mut heading_stack,
                &mut duplicate_counts,
                &mut nodes,
                &mut edges,
                &mut link_index,
                &mut code_index,
            );
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("markdown".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_markdown_blocks(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    default_parent_qn: &str,
    heading_stack: &mut Vec<(u8, String, String)>,
    duplicate_counts: &mut HashMap<String, usize>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    link_index: &mut usize,
    code_index: &mut usize,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "section" => {
                let Some(heading) = first_heading(child) else {
                    walk_markdown_blocks(
                        child,
                        ctx,
                        default_parent_qn,
                        heading_stack,
                        duplicate_counts,
                        nodes,
                        edges,
                        link_index,
                        code_index,
                    );
                    continue;
                };
                let level = heading_level(heading, ctx.source);
                let title = heading_title(heading, ctx.source);
                while heading_stack.last().is_some_and(|(depth, _, _)| *depth >= level) {
                    heading_stack.pop();
                }
                let parent_qn = heading_stack
                    .last()
                    .map(|(_, _, qn)| qn.clone())
                    .unwrap_or_else(|| default_parent_qn.to_owned());
                let parent_path = heading_stack
                    .last()
                    .map(|(_, path, _)| path.clone())
                    .unwrap_or_else(|| "document".to_owned());
                let slug = slugify(&title);
                let key = format!("{}>{}", parent_path, slug);
                let duplicate = duplicate_counts.entry(key).or_insert(0);
                *duplicate += 1;
                let path_segment = if *duplicate == 1 {
                    slug.clone()
                } else {
                    format!("{}-{}", slug, *duplicate)
                };
                let heading_path = format!("{}.{}", parent_path, path_segment);
                let heading_qn = format!("{}::heading::{}", ctx.rel_path, heading_path);
                nodes.push(Node {
                    id: NodeId::UNSET,
                    kind: NodeKind::Module,
                    name: title.clone(),
                    qualified_name: heading_qn.clone(),
                    file_path: ctx.rel_path.to_owned(),
                    line_start: start_line(heading),
                    line_end: end_line(child),
                    language: "markdown".to_owned(),
                    parent_name: Some(parent_qn.clone()),
                    params: None,
                    return_type: None,
                    modifiers: None,
                    is_test: false,
                    file_hash: ctx.file_hash.to_owned(),
                    extra_json: serde_json::json!({ "level": level, "path": heading_path }),
                });
                edges.push(contains_edge(&parent_qn, &heading_qn, ctx.rel_path, start_line(heading)));
                heading_stack.push((level, heading_path, heading_qn.clone()));
                scan_section_body(
                    child,
                    ctx,
                    &heading_qn,
                    nodes,
                    edges,
                    link_index,
                    code_index,
                );
                walk_markdown_blocks(
                    child,
                    ctx,
                    &heading_qn,
                    heading_stack,
                    duplicate_counts,
                    nodes,
                    edges,
                    link_index,
                    code_index,
                );
                heading_stack.pop();
            }
            "fenced_code_block" => emit_code_block(child, ctx, default_parent_qn, nodes, edges, code_index),
            "paragraph" => scan_links(child, ctx, default_parent_qn, nodes, edges, link_index),
            _ => walk_markdown_blocks(
                child,
                ctx,
                default_parent_qn,
                heading_stack,
                duplicate_counts,
                nodes,
                edges,
                link_index,
                code_index,
            ),
        }
    }
}

fn scan_section_body(
    section: TsNode<'_>,
    ctx: &ParseContext<'_>,
    container_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    link_index: &mut usize,
    code_index: &mut usize,
) {
    let mut cursor = section.walk();
    for child in section.named_children(&mut cursor) {
        match child.kind() {
            "fenced_code_block" => emit_code_block(child, ctx, container_qn, nodes, edges, code_index),
            "paragraph" => scan_links(child, ctx, container_qn, nodes, edges, link_index),
            _ => {}
        }
    }
}

fn emit_code_block(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    code_index: &mut usize,
) {
    *code_index += 1;
    let info = find_named_child_text(node, "info_string", ctx.source).unwrap_or_default();
    let qn = format!("{}::code::{}", ctx.rel_path, *code_index);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Variable,
        name: if info.is_empty() { "code".to_owned() } else { format!("code:{}", info) },
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "markdown".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "info_string": info, "kind": "fenced_code_block" }),
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
}

fn scan_links(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    link_index: &mut usize,
) {
    let inline_link = Regex::new(r"\[[^\]]+\]\(([^)\s]+)[^)]*\)").expect("inline link regex");
    let ref_link = Regex::new(r"(?m)^\[[^\]]+\]:\s*(\S+)").expect("reference link regex");
    let text = node_text(node, ctx.source);
    for capture in inline_link.captures_iter(text).chain(ref_link.captures_iter(text)) {
        let Some(matched) = capture.get(1) else {
            continue;
        };
        *link_index += 1;
        let qn = format!("{}::link::{}", ctx.rel_path, *link_index);
        let line = start_line(node);
        let destination = matched.as_str().to_owned();
        nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Import,
            name: destination.clone(),
            qualified_name: qn.clone(),
            file_path: ctx.rel_path.to_owned(),
            line_start: line,
            line_end: end_line(node),
            language: "markdown".to_owned(),
            parent_name: Some(parent_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: ctx.file_hash.to_owned(),
            extra_json: serde_json::json!({ "destination": destination }),
        });
        edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
        edges.push(Edge {
            id: 0,
            kind: EdgeKind::Imports,
            source_qn: parent_qn.to_owned(),
            target_qn: qn,
            file_path: ctx.rel_path.to_owned(),
            line: Some(line),
            confidence: 0.8,
            confidence_tier: Some("markdown_link".to_owned()),
            extra_json: serde_json::Value::Null,
        });
    }
}

fn first_heading(node: TsNode<'_>) -> Option<TsNode<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| matches!(child.kind(), "atx_heading" | "setext_heading"))
}

fn heading_level(node: TsNode<'_>, source: &[u8]) -> u8 {
    match node.kind() {
        "atx_heading" => node_text(node, source)
            .chars()
            .take_while(|ch| *ch == '#')
            .count()
            .clamp(1, 6) as u8,
        "setext_heading" => {
            let text = node_text(node, source);
            if text.lines().nth(1).unwrap_or_default().trim_start().starts_with('=') {
                1
            } else {
                2
            }
        }
        _ => 1,
    }
}

fn heading_title(node: TsNode<'_>, source: &[u8]) -> String {
    let raw = node_text(node, source);
    match node.kind() {
        "atx_heading" => raw
            .trim()
            .trim_start_matches('#')
            .trim()
            .trim_end_matches('#')
            .trim()
            .to_owned(),
        "setext_heading" => raw.lines().next().unwrap_or(raw).trim().to_owned(),
        _ => raw.trim().to_owned(),
    }
}

fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in text.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_owned().if_empty_then("section")
}

trait EmptyFallback {
    fn if_empty_then(self, fallback: &str) -> String;
}

impl EmptyFallback for String {
    fn if_empty_then(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_owned()
        } else {
            self
        }
    }
}

fn find_named_child_text(node: TsNode<'_>, kind: &str, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
        .map(|child| node_text(child, source).trim().to_owned())
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
        let (pf, _) = MarkdownParser.parse(&ParseContext {
            rel_path: "README.md",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_headings_code_blocks_and_links() {
        let pf = parse(
            "# Intro\n\nSee [guide](docs/guide.md).\n\n## Usage\n\n```rust\nfn main() {}\n```\n",
        );
        assert!(pf.nodes.iter().any(|node| node.qualified_name == "README.md::document"));
        assert!(pf.nodes.iter().any(|node| node.qualified_name == "README.md::heading::document.intro"));
        assert!(pf.nodes.iter().any(|node| node.kind == NodeKind::Import && node.name == "docs/guide.md"));
        assert!(pf.nodes.iter().any(|node| node.qualified_name == "README.md::code::1"));
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Imports));
    }
}
