//! PHP parser backed by `tree-sitter-php`.
//! Grammar source: `tree-sitter/tree-sitter-php`.

use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use regex::Regex;
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct PhpParser;

impl LangParser for PhpParser {
    fn language_name(&self) -> &'static str {
        "php"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".php")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .expect("tree-sitter-php grammar failed to load");

        let tree = parser.parse(ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let mut import_index = 0usize;
            let mut attribute_index = 0usize;
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                visit_node(
                    child,
                    ctx,
                    ctx.rel_path,
                    None,
                    &mut import_index,
                    &mut attribute_index,
                    &mut nodes,
                    &mut edges,
                );
            }

            let call_map = callable_qn_map(&nodes);
            let mut call_edges = Vec::new();
            walk_calls(root, ctx, &call_map, None, &mut call_edges);
            edges.extend(call_edges);
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("php".to_owned()),
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
fn visit_node(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    current_type: Option<&str>,
    import_index: &mut usize,
    attribute_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "namespace_definition" => emit_namespace(
            node,
            ctx,
            parent_qn,
            import_index,
            attribute_index,
            nodes,
            edges,
        ),
        "namespace_use_declaration" => emit_use(node, ctx, parent_qn, import_index, nodes, edges),
        "class_declaration" => emit_type(
            node,
            ctx,
            parent_qn,
            "class",
            NodeKind::Class,
            import_index,
            attribute_index,
            nodes,
            edges,
        ),
        "interface_declaration" => emit_type(
            node,
            ctx,
            parent_qn,
            "interface",
            NodeKind::Interface,
            import_index,
            attribute_index,
            nodes,
            edges,
        ),
        "trait_declaration" => emit_type(
            node,
            ctx,
            parent_qn,
            "trait",
            NodeKind::Trait,
            import_index,
            attribute_index,
            nodes,
            edges,
        ),
        "function_definition" => emit_function(node, ctx, parent_qn, attribute_index, nodes, edges),
        "method_declaration" if current_type.is_some() => emit_method(
            node,
            ctx,
            parent_qn,
            current_type.expect("current_type checked"),
            attribute_index,
            nodes,
            edges,
        ),
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                visit_node(
                    child,
                    ctx,
                    parent_qn,
                    current_type,
                    import_index,
                    attribute_index,
                    nodes,
                    edges,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_namespace(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    import_index: &mut usize,
    attribute_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = node
        .child_by_field_name("name")
        .map(|name_node| node_text(name_node, ctx.source).to_owned())
        .or_else(|| {
            first_named_text(
                node,
                ctx.source,
                &["namespace_name", "qualified_name", "name"],
            )
        })
    else {
        return;
    };
    let qn = format!("{}::namespace::{}", ctx.rel_path, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Module,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "php".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_node(
            child,
            ctx,
            &qn,
            None,
            import_index,
            attribute_index,
            nodes,
            edges,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_type(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    qn_prefix: &str,
    kind: NodeKind,
    import_index: &mut usize,
    attribute_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = node
        .child_by_field_name("name")
        .map(|name_node| node_text(name_node, ctx.source).to_owned())
    else {
        return;
    };
    let qn = format!("{}::{}::{}", ctx.rel_path, qn_prefix, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "php".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
    emit_attributes(node, ctx, &qn, attribute_index, nodes, edges);

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_node(
            child,
            ctx,
            &qn,
            Some(&name),
            import_index,
            attribute_index,
            nodes,
            edges,
        );
    }
}

fn emit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    attribute_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = node
        .child_by_field_name("name")
        .map(|name_node| node_text(name_node, ctx.source).to_owned())
    else {
        return;
    };
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
        language: "php".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: node
            .child_by_field_name("parameters")
            .map(|params| node_text(params, ctx.source).to_owned()),
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
    emit_attributes(node, ctx, &qn, attribute_index, nodes, edges);
}

fn emit_method(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    owner_name: &str,
    attribute_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = node
        .child_by_field_name("name")
        .map(|name_node| node_text(name_node, ctx.source).to_owned())
    else {
        return;
    };
    let qn = format!("{}::method::{}.{}", ctx.rel_path, owner_name, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Method,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "php".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: node
            .child_by_field_name("parameters")
            .map(|params| node_text(params, ctx.source).to_owned()),
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
    emit_attributes(node, ctx, &qn, attribute_index, nodes, edges);
}

fn emit_use(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "namespace_use_clause" {
            continue;
        }
        let Some(name) = first_named_text(child, ctx.source, &["qualified_name", "name"]) else {
            continue;
        };
        *import_index += 1;
        let qn = format!("{}::import::php:{}", ctx.rel_path, *import_index);
        let line = start_line(child);
        nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Import,
            name: name.clone(),
            qualified_name: qn.clone(),
            file_path: ctx.rel_path.to_owned(),
            line_start: line,
            line_end: end_line(child),
            language: "php".to_owned(),
            parent_name: Some(parent_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: ctx.file_hash.to_owned(),
            extra_json: serde_json::json!({ "imported": name }),
        });
        edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
        edges.push(imports_edge(
            parent_qn,
            &qn,
            ctx.rel_path,
            line,
            "namespace_use",
        ));
    }
}

fn emit_attributes(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    owner_qn: &str,
    attribute_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let regex = Regex::new(r"[A-Za-z_\\][A-Za-z0-9_\\]*").expect("valid php attribute regex");
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "attribute_list" {
            continue;
        }
        for capture in regex.find_iter(node_text(child, ctx.source)) {
            let name = capture.as_str();
            if name == "Attribute" {
                continue;
            }
            *attribute_index += 1;
            let qn = format!(
                "{}::attribute::{}#{}",
                ctx.rel_path, owner_qn, *attribute_index
            );
            let line = start_line(child);
            nodes.push(Node {
                id: NodeId::UNSET,
                kind: NodeKind::Variable,
                name: name.to_owned(),
                qualified_name: qn.clone(),
                file_path: ctx.rel_path.to_owned(),
                line_start: line,
                line_end: end_line(child),
                language: "php".to_owned(),
                parent_name: Some(owner_qn.to_owned()),
                params: None,
                return_type: None,
                modifiers: Some("attribute".to_owned()),
                is_test: false,
                file_hash: ctx.file_hash.to_owned(),
                extra_json: serde_json::json!({ "kind": "attribute" }),
            });
            edges.push(contains_edge(owner_qn, &qn, ctx.rel_path, line));
        }
    }
}

fn callable_qn_map(nodes: &[Node]) -> HashMap<String, String> {
    nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Function | NodeKind::Method))
        .map(|node| (node.name.clone(), node.qualified_name.clone()))
        .collect()
}

fn walk_calls(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    callables: &HashMap<String, String>,
    current_owner: Option<String>,
    edges: &mut Vec<Edge>,
) {
    let mut next_owner = current_owner;
    match node.kind() {
        "function_definition" | "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                next_owner = callables.get(node_text(name_node, ctx.source)).cloned();
            }
        }
        "function_call_expression" | "scoped_call_expression" | "member_call_expression" => {
            if let Some(owner_qn) = next_owner.as_ref()
                && let Some(callee) = call_name(node, ctx.source)
                && let Some(target_qn) = callables.get(&callee)
            {
                edges.push(call_edge(
                    owner_qn,
                    target_qn,
                    ctx.rel_path,
                    start_line(node),
                    "same_file",
                ));
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_calls(child, ctx, callables, next_owner.clone(), edges);
    }
}

fn call_name(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .or_else(|| node.child_by_field_name("function"))
        .or_else(|| {
            let mut cursor = node.walk();
            node.named_children(&mut cursor).next()
        })
        .map(|name_node| match name_node.kind() {
            "qualified_name" => node_text(name_node, source)
                .rsplit('\\')
                .next()
                .unwrap_or_else(|| node_text(name_node, source))
                .to_owned(),
            "variable_name" => {
                first_named_text(name_node, source, &["name"]).unwrap_or_else(|| {
                    node_text(name_node, source)
                        .trim_start_matches('$')
                        .to_owned()
                })
            }
            _ => node_text(name_node, source)
                .trim_start_matches('$')
                .to_owned(),
        })
}

fn first_named_text(node: TsNode<'_>, source: &[u8], kinds: &[&str]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if kinds.contains(&child.kind()) {
            return Some(node_text(child, source).to_owned());
        }
    }
    None
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
        language: "php".to_owned(),
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

fn call_edge(source_qn: &str, target_qn: &str, file_path: &str, line: u32, tier: &str) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
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
        let (pf, _) = PhpParser.parse(&ParseContext {
            rel_path: "src/index.php",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_namespace_types_methods_imports_attributes_and_calls() {
        let pf = parse(
            "<?php\nnamespace Demo\\App;\nuse Demo\\Support\\Helper;\n#[Service]\nclass Runner {\n    #[Trace]\n    public function run() { helper(); }\n    private function helper() {}\n}\ntrait UsesLog {}\ninterface RunnerContract {}\nfunction helper() {}\n",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/index.php::namespace::Demo\\App")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/index.php::class::Runner")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/index.php::trait::UsesLog")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/index.php::interface::RunnerContract")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "Demo\\Support\\Helper")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Variable && node.name == "Service")
        );
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Calls));
    }
}
