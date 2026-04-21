//! Bash parser backed by `tree-sitter-bash`.
//! Grammar source: `tree-sitter/tree-sitter-bash`.

use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct BashParser;

impl LangParser for BashParser {
    fn language_name(&self) -> &'static str {
        "bash"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".sh") || path.ends_with(".bash")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .expect("tree-sitter-bash grammar failed to load");

        let tree = parser.parse(ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count, "bash"));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let mut import_index = 0usize;
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                match child.kind() {
                    "function_definition" => emit_function(child, ctx, &mut nodes, &mut edges),
                    "command" => emit_source_command(
                        child,
                        ctx,
                        ctx.rel_path,
                        &mut nodes,
                        &mut edges,
                        &mut import_index,
                    ),
                    _ => {}
                }
            }

            let function_map = function_qn_map(&nodes);
            let mut call_edges = Vec::new();
            let mut extra_import_nodes = Vec::new();
            let mut extra_import_edges = Vec::new();
            walk_commands(
                root,
                ctx,
                &function_map,
                None,
                &mut import_index,
                &mut extra_import_nodes,
                &mut extra_import_edges,
                &mut call_edges,
            );
            nodes.extend(extra_import_nodes);
            edges.extend(extra_import_edges);
            edges.extend(call_edges);
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("bash".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn emit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, ctx.source).to_owned();
    let qn = format!("{}::fn::{}", ctx.rel_path, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "bash".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, line));
}

fn emit_source_command(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    import_index: &mut usize,
) {
    let Some((command_name, target)) = parse_source_command(node, ctx.source) else {
        return;
    };
    *import_index += 1;
    let qn = format!("{}::import::bash:{}", ctx.rel_path, *import_index);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Import,
        name: target.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "bash".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "command": command_name, "imported": target }),
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
    edges.push(imports_edge(
        parent_qn,
        &qn,
        ctx.rel_path,
        line,
        "shell_source",
    ));
}

fn function_qn_map(nodes: &[Node]) -> HashMap<String, String> {
    nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Function)
        .map(|node| (node.name.clone(), node.qualified_name.clone()))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn walk_commands(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    functions: &HashMap<String, String>,
    current_fn: Option<String>,
    import_index: &mut usize,
    import_nodes: &mut Vec<Node>,
    import_edges: &mut Vec<Edge>,
    call_edges: &mut Vec<Edge>,
) {
    let mut next_fn = current_fn;
    if node.kind() == "function_definition"
        && let Some(name_node) = node.child_by_field_name("name")
    {
        let name = node_text(name_node, ctx.source);
        next_fn = Some(format!("{}::fn::{}", ctx.rel_path, name));
    }

    if node.kind() == "command"
        && let Some(ref owner_qn) = next_fn
    {
        if let Some(command_name) = command_name(node, ctx.source)
            && let Some(target_qn) = functions.get(command_name)
        {
            call_edges.push(Edge {
                id: 0,
                kind: EdgeKind::Calls,
                source_qn: owner_qn.clone(),
                target_qn: target_qn.clone(),
                file_path: ctx.rel_path.to_owned(),
                line: Some(start_line(node)),
                confidence: 1.0,
                confidence_tier: Some("same_file".to_owned()),
                extra_json: serde_json::Value::Null,
            });
        }
        if parse_source_command(node, ctx.source).is_some() {
            emit_source_command(
                node,
                ctx,
                owner_qn,
                import_nodes,
                import_edges,
                import_index,
            );
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_commands(
            child,
            ctx,
            functions,
            next_fn.clone(),
            import_index,
            import_nodes,
            import_edges,
            call_edges,
        );
    }
}

fn parse_source_command(node: TsNode<'_>, source: &[u8]) -> Option<(String, String)> {
    let command_name = command_name(node, source)?;
    if command_name != "source" && command_name != "." {
        return None;
    }

    let mut cursor = node.walk();
    let mut seen_command = false;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "command_name" => seen_command = true,
            "word" | "string" if seen_command => {
                return Some((
                    command_name.to_owned(),
                    node_text(child, source)
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_owned(),
                ));
            }
            _ => {}
        }
    }
    None
}

fn command_name<'a>(node: TsNode<'a>, source: &'a [u8]) -> Option<&'a str> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == "command_name")
        .map(|child| node_text(child, source).trim())
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
        let (pf, _) = BashParser.parse(&ParseContext {
            rel_path: "scripts/deploy.sh",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_functions_source_commands_and_calls() {
        let pf = parse(
            "source ./env.sh\nsetup() {\n  helper\n  source ./inner.sh\n}\nhelper() {\n  echo hi\n}\n",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "scripts/deploy.sh::fn::setup")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "./env.sh")
        );
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Calls
            && edge.target_qn == "scripts/deploy.sh::fn::helper"));
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Imports));
    }
}
