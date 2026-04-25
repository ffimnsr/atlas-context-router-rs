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

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count, "bash"));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            collect_functions(root, ctx, ctx.rel_path, &mut nodes, &mut edges);

            let function_map = function_qn_map(&nodes);
            let mut call_edges = Vec::new();
            let mut extra_import_nodes = Vec::new();
            let mut extra_import_edges = Vec::new();
            let mut import_index = 0usize;
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

fn collect_functions(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut next_parent_qn = parent_qn.to_owned();
    if node.kind() == "function_definition"
        && let Some(function_qn) = emit_function(node, ctx, parent_qn, nodes, edges)
    {
        next_parent_qn = function_qn;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_functions(child, ctx, &next_parent_qn, nodes, edges);
    }
}

fn emit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, ctx.source).to_owned();
    let qn = function_qn(parent_qn, ctx.rel_path, &name);
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
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
    Some(qn)
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
    let mut functions = HashMap::new();
    for node in nodes.iter().filter(|node| node.kind == NodeKind::Function) {
        functions.insert(node.qualified_name.clone(), node.name.clone());
    }
    functions
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
        let parent_qn = next_fn.as_deref().unwrap_or(ctx.rel_path);
        next_fn = Some(function_qn(parent_qn, ctx.rel_path, name));
    }

    if node.kind() == "command" {
        let owner_qn = next_fn.clone().unwrap_or_else(|| ctx.rel_path.to_owned());
        if let Some(command_name) = command_name(node, ctx.source)
            && let Some(ref function_owner) = next_fn
            && let Some(target_qn) =
                resolve_function_target(function_owner, command_name, functions)
        {
            call_edges.push(Edge {
                id: 0,
                kind: EdgeKind::Calls,
                source_qn: function_owner.clone(),
                target_qn: target_qn.to_owned(),
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
                &owner_qn,
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
    let args = command_args(node, source);

    if command_name == "source" || command_name == "." {
        let target = args.first()?.to_owned();
        return Some((command_name.to_owned(), target));
    }

    if (command_name == "command" || command_name == "builtin") && args.len() >= 2 {
        let forwarded = &args[0];
        if forwarded == "source" || forwarded == "." {
            return Some((forwarded.clone(), args[1].clone()));
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

fn command_args(node: TsNode<'_>, source: &[u8]) -> Vec<String> {
    let mut args = Vec::new();
    let mut cursor = node.walk();
    let mut seen_command = false;
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "command_name" => seen_command = true,
            "word" | "string" | "raw_string" if seen_command => {
                args.push(
                    node_text(child, source)
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_owned(),
                );
            }
            _ => {}
        }
    }
    args
}

fn function_qn(parent_qn: &str, rel_path: &str, name: &str) -> String {
    if parent_qn == rel_path {
        return format!("{}::fn::{}", rel_path, name);
    }
    format!("{}::fn::{}", parent_qn, name)
}

fn resolve_function_target<'a>(
    current_fn: &str,
    command_name: &str,
    functions: &'a HashMap<String, String>,
) -> Option<&'a str> {
    let mut scope = Some(current_fn);
    while let Some(scope_qn) = scope {
        if let Some(qn) = resolve_nested_scope_target(scope_qn, command_name, functions) {
            return Some(qn);
        }
        scope = parent_function_qn(scope_qn);
    }

    functions.iter().find_map(|(qn, name)| {
        (name == command_name && parent_function_qn(qn).is_none()).then_some(qn.as_str())
    })
}

fn resolve_nested_scope_target<'a>(
    scope_qn: &str,
    command_name: &str,
    functions: &'a HashMap<String, String>,
) -> Option<&'a str> {
    let nested_prefix = format!("{}::fn::", scope_qn);
    let mut best_match: Option<(&str, usize)> = None;
    for (qn, name) in functions {
        if name != command_name || !qn.starts_with(&nested_prefix) {
            continue;
        }
        let depth = qn.matches("::fn::").count();
        match best_match {
            Some((_, best_depth)) if depth >= best_depth => {}
            _ => best_match = Some((qn.as_str(), depth)),
        }
    }
    best_match.map(|(qn, _)| qn)
}

fn parent_function_qn(qn: &str) -> Option<&str> {
    qn.rsplit_once("::fn::")
        .and_then(|(parent, _)| parent.contains("::fn::").then_some(parent))
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

    #[test]
    fn extracts_nested_top_level_source_and_wrapped_source() {
        let pf = parse(
            "if true; then\n  command source ./from_if.sh\nfi\nsetup() {\n  builtin source ./from_fn.sh\n}\n",
        );
        assert!(pf.nodes.iter().any(|node| node.kind == NodeKind::Import
            && node.name == "./from_if.sh"
            && node.parent_name.as_deref() == Some("scripts/deploy.sh")));
        assert!(pf.nodes.iter().any(|node| node.kind == NodeKind::Import
            && node.name == "./from_fn.sh"
            && node.parent_name.as_deref() == Some("scripts/deploy.sh::fn::setup")));
    }

    #[test]
    fn nested_functions_keep_parent_function_ownership() {
        let pf = parse("outer() {\n  inner() {\n    echo hi\n  }\n  inner\n}\n");
        assert!(pf.nodes.iter().any(|node| node.kind == NodeKind::Function
            && node.qualified_name == "scripts/deploy.sh::fn::outer::fn::inner"
            && node.parent_name.as_deref() == Some("scripts/deploy.sh::fn::outer")));
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Contains
            && edge.source_qn == "scripts/deploy.sh::fn::outer"
            && edge.target_qn == "scripts/deploy.sh::fn::outer::fn::inner"));
    }

    #[test]
    fn nested_same_name_functions_get_distinct_qnames() {
        let pf = parse(
            "first() {\n  helper() {\n    echo first\n  }\n  helper\n}\nsecond() {\n  helper() {\n    echo second\n  }\n  helper\n}\n",
        );

        assert!(pf.nodes.iter().any(|node| node.kind == NodeKind::Function
            && node.qualified_name == "scripts/deploy.sh::fn::first::fn::helper"
            && node.parent_name.as_deref() == Some("scripts/deploy.sh::fn::first")));
        assert!(pf.nodes.iter().any(|node| node.kind == NodeKind::Function
            && node.qualified_name == "scripts/deploy.sh::fn::second::fn::helper"
            && node.parent_name.as_deref() == Some("scripts/deploy.sh::fn::second")));
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Calls
            && edge.source_qn == "scripts/deploy.sh::fn::first"
            && edge.target_qn == "scripts/deploy.sh::fn::first::fn::helper"));
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Calls
            && edge.source_qn == "scripts/deploy.sh::fn::second"
            && edge.target_qn == "scripts/deploy.sh::fn::second::fn::helper"));
    }
}
