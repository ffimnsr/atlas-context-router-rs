//! Ruby parser backed by `tree-sitter-ruby`.
//! Grammar source: `tree-sitter/tree-sitter-ruby`.

use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, field_text, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct RubyParser;

impl LangParser for RubyParser {
    fn language_name(&self) -> &'static str {
        "ruby"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".rb")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .expect("tree-sitter-ruby grammar failed to load");

        let tree = parser.parse(ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let mut import_index = 0usize;
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                visit_ruby_node(
                    child,
                    ctx,
                    ctx.rel_path,
                    None,
                    &mut import_index,
                    &mut nodes,
                    &mut edges,
                );
            }

            let callable_map = callable_qn_map(&nodes);
            let mut call_edges = Vec::new();
            walk_calls(root, ctx, &callable_map, None, &mut call_edges);
            edges.extend(call_edges);
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("ruby".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn visit_ruby_node(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    current_owner: Option<&str>,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "module" => emit_module(node, ctx, parent_qn, import_index, nodes, edges),
        "class" => emit_class(node, ctx, parent_qn, import_index, nodes, edges),
        "method" => emit_method(node, ctx, parent_qn, current_owner, nodes, edges),
        "singleton_method" => {
            emit_singleton_method(node, ctx, parent_qn, current_owner, nodes, edges)
        }
        "call" => emit_import_like_call(
            node,
            ctx,
            parent_qn,
            current_owner,
            import_index,
            nodes,
            edges,
        ),
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                visit_ruby_node(
                    child,
                    ctx,
                    parent_qn,
                    current_owner,
                    import_index,
                    nodes,
                    edges,
                );
            }
        }
    }
}

fn emit_module(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source).map(str::to_owned) else {
        return;
    };
    let qn = format!("{}::module::{}", ctx.rel_path, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Module,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "ruby".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            visit_ruby_node(
                child,
                ctx,
                &qn,
                Some(name.as_str()),
                import_index,
                nodes,
                edges,
            );
        }
    }
}

fn emit_class(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source).map(str::to_owned) else {
        return;
    };
    let qn = format!("{}::class::{}", ctx.rel_path, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Class,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "ruby".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            visit_ruby_node(
                child,
                ctx,
                &qn,
                Some(name.as_str()),
                import_index,
                nodes,
                edges,
            );
        }
    }
}

fn emit_method(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    current_owner: Option<&str>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source).map(str::to_owned) else {
        return;
    };
    let owner = current_owner.unwrap_or("Object");
    let qn = format!("{}::method::{}.{}", ctx.rel_path, owner, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Method,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "ruby".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: node
            .child_by_field_name("parameters")
            .map(|n| node_text(n, ctx.source).to_owned()),
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
}

fn emit_singleton_method(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    current_owner: Option<&str>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source).map(str::to_owned) else {
        return;
    };
    let owner = singleton_owner(node, ctx.source, current_owner);
    let qn = format!("{}::singleton_method::{}.{}", ctx.rel_path, owner, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Method,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "ruby".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: node
            .child_by_field_name("parameters")
            .map(|n| node_text(n, ctx.source).to_owned()),
        return_type: None,
        modifiers: Some("singleton".to_owned()),
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "singleton": true }),
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
}

fn emit_import_like_call(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    current_owner: Option<&str>,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(method_name) = ruby_call_name(node, ctx.source) else {
        return;
    };
    let tier = match method_name.as_str() {
        "require" | "require_relative" => Some("require"),
        "include" | "extend" | "prepend" => Some("mixin"),
        _ => None,
    };
    let Some(tier) = tier else {
        return;
    };
    let Some(target) = ruby_call_argument(node, ctx.source) else {
        return;
    };

    *import_index += 1;
    let qn = format!("{}::import::ruby:{}", ctx.rel_path, *import_index);
    let line = start_line(node);
    let owner_qn = if tier == "mixin" {
        parent_qn.to_owned()
    } else {
        current_owner
            .map(|_| parent_qn.to_owned())
            .unwrap_or_else(|| ctx.rel_path.to_owned())
    };
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Import,
        name: target.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "ruby".to_owned(),
        parent_name: Some(owner_qn.clone()),
        params: None,
        return_type: None,
        modifiers: Some(method_name.clone()),
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "kind": tier, "method": method_name, "imported": target }),
    });
    edges.push(contains_edge(&owner_qn, &qn, ctx.rel_path, line));
    edges.push(imports_edge(&owner_qn, &qn, ctx.rel_path, line, tier));
}

fn callable_qn_map(nodes: &[Node]) -> HashMap<String, String> {
    nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Method)
        .map(|node| (node.name.clone(), node.qualified_name.clone()))
        .collect()
}

fn walk_calls(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    callables: &HashMap<String, String>,
    current_callable: Option<String>,
    edges: &mut Vec<Edge>,
) {
    let mut next_callable = current_callable;
    if matches!(node.kind(), "method" | "singleton_method") {
        if let Some(name) = field_text(node, "name", ctx.source) {
            next_callable = callables.get(name).cloned();
        }
    } else if node.kind() == "call"
        && let Some(owner_qn) = next_callable.as_ref()
        && let Some(callee) = ruby_call_name(node, ctx.source)
        && !matches!(
            callee.as_str(),
            "require" | "require_relative" | "include" | "extend" | "prepend"
        )
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

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_calls(child, ctx, callables, next_callable.clone(), edges);
    }
}

fn ruby_call_name(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("method")
        .map(|method| node_text(method, source).trim_start_matches(':').to_owned())
        .or_else(|| {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|child| child.kind() == "identifier")
                .map(|child| node_text(child, source).to_owned())
        })
}

fn ruby_call_argument(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let args = node.child_by_field_name("arguments")?;
    let raw = node_text(args, source).trim();
    Some(
        raw.trim_matches(|ch| matches!(ch, '(' | ')' | '"' | '\'' | ' '))
            .to_owned(),
    )
}

fn singleton_owner(node: TsNode<'_>, source: &[u8], current_owner: Option<&str>) -> String {
    let raw = node
        .child_by_field_name("object")
        .map(|object| node_text(object, source).trim().to_owned())
        .unwrap_or_else(|| current_owner.unwrap_or("Object").to_owned());
    if raw == "self" {
        return current_owner.unwrap_or("Object").to_owned();
    }
    raw.rsplit("::")
        .next()
        .unwrap_or(raw.as_str())
        .trim_start_matches('@')
        .to_owned()
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
        language: "ruby".to_owned(),
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
        let (pf, _) = RubyParser.parse(&ParseContext {
            rel_path: "lib/app.rb",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_modules_classes_methods_imports_mixins_and_calls() {
        let pf = parse(
            "require \"json\"\nrequire_relative \"helper\"\n\nmodule Demo\n  class Runner\n    include Logging\n    extend Builders\n\n    def helper\n    end\n\n    def run\n      helper()\n    end\n\n    def self.build\n      helper()\n    end\n  end\nend\n",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "lib/app.rb::module::Demo")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "lib/app.rb::class::Runner")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "lib/app.rb::method::Runner.run")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "lib/app.rb::singleton_method::Runner.build")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "json")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "Logging")
        );
        assert!(pf.edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls && edge.target_qn == "lib/app.rb::method::Runner.helper"
        }));
    }
}
