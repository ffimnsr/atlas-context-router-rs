//! C++ parser backed by `tree-sitter-cpp`.
//! Grammar source: `tree-sitter/tree-sitter-cpp`.

use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct CppParser;

impl LangParser for CppParser {
    fn language_name(&self) -> &'static str {
        "cpp"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".cpp") || path.ends_with(".cc") || path.ends_with(".cxx")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .expect("tree-sitter-cpp grammar failed to load");

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
                visit_node(
                    child,
                    ctx,
                    ctx.rel_path,
                    false,
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
                language: Some("cpp".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn visit_node(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    templated: bool,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "preproc_include" => emit_include(node, ctx, parent_qn, import_index, nodes, edges),
        "namespace_definition" => emit_namespace(node, ctx, import_index, nodes, edges),
        "class_specifier" => emit_type(
            node,
            ctx,
            parent_qn,
            "class",
            NodeKind::Class,
            templated,
            import_index,
            nodes,
            edges,
        ),
        "struct_specifier" => emit_type(
            node,
            ctx,
            parent_qn,
            "struct",
            NodeKind::Struct,
            templated,
            import_index,
            nodes,
            edges,
        ),
        "enum_specifier" => emit_type(
            node,
            ctx,
            parent_qn,
            "enum",
            NodeKind::Enum,
            templated,
            import_index,
            nodes,
            edges,
        ),
        "function_definition" => emit_function(node, ctx, parent_qn, templated, nodes, edges),
        "template_declaration" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                visit_node(child, ctx, parent_qn, true, import_index, nodes, edges);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                visit_node(child, ctx, parent_qn, templated, import_index, nodes, edges);
            }
        }
    }
}

fn emit_include(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    *import_index += 1;
    let raw = node_text(node, ctx.source).trim();
    let name = raw
        .split_once('#')
        .map(|(_, include)| include.trim().trim_start_matches("include").trim())
        .unwrap_or(raw)
        .trim_matches('<')
        .trim_matches('>')
        .trim_matches('"')
        .to_owned();
    let qn = format!("{}::import::cpp:{}", ctx.rel_path, *import_index);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Import,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "cpp".to_owned(),
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
        "preproc_include",
    ));
}

fn emit_namespace(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, ctx.source).to_owned();
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
        language: "cpp".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, line));

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_node(child, ctx, &qn, false, import_index, nodes, edges);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_type(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    qn_prefix: &str,
    kind: NodeKind,
    templated: bool,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, ctx.source).to_owned();
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
        language: "cpp".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: templated.then(|| "template".to_owned()),
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "templated": templated }),
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_node(child, ctx, &qn, false, import_index, nodes, edges);
    }
}

fn emit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    templated: bool,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(declarator) = node.child_by_field_name("declarator") else {
        return;
    };
    let Some(signature) = declarator_signature(declarator, ctx.source) else {
        return;
    };
    let inline_owner = parent_scope_owner(parent_qn);
    let (kind, qn, name) = if let Some((owner, method_name)) = method_parts(&signature) {
        (
            NodeKind::Method,
            format!("{}::method::{}.{}", ctx.rel_path, owner, method_name),
            method_name,
        )
    } else if let Some(owner) = inline_owner {
        let method_name = final_segment(&signature);
        (
            NodeKind::Method,
            format!("{}::method::{}.{}", ctx.rel_path, owner, method_name),
            method_name,
        )
    } else {
        let function_name = final_segment(&signature);
        (
            NodeKind::Function,
            format!("{}::fn::{}", ctx.rel_path, function_name),
            function_name,
        )
    };
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "cpp".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: Some(signature),
        return_type: node
            .child_by_field_name("type")
            .map(|ret| node_text(ret, ctx.source).to_owned()),
        modifiers: templated.then(|| "template".to_owned()),
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "templated": templated }),
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
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
    if node.kind() == "function_definition"
        && let Some(declarator) = node.child_by_field_name("declarator")
        && let Some(signature) = declarator_signature(declarator, ctx.source)
    {
        let lookup = method_parts(&signature)
            .map(|(_, method_name)| method_name)
            .unwrap_or_else(|| final_segment(&signature));
        next_owner = callables.get(&lookup).cloned();
    } else if node.kind() == "call_expression"
        && let Some(owner_qn) = next_owner.as_ref()
        && let Some(function_node) = node.child_by_field_name("function")
    {
        let lookup = final_segment(&node_text(function_node, ctx.source).replace(' ', ""));
        if let Some(target_qn) = callables.get(&lookup) {
            edges.push(call_edge(
                owner_qn,
                target_qn,
                ctx.rel_path,
                start_line(node),
                "same_file",
            ));
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_calls(child, ctx, callables, next_owner.clone(), edges);
    }
}

fn declarator_signature(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let raw = node_text(node, source);
    let head = raw.split('(').next()?.trim();
    Some(head.replace(' ', ""))
}

fn method_parts(signature: &str) -> Option<(String, String)> {
    let mut parts = signature.split("::").collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let method = normalize_name(parts.pop().expect("len checked"));
    let owner = normalize_name(parts.pop().expect("len checked"));
    Some((owner, method))
}

fn final_segment(signature: &str) -> String {
    normalize_name(signature.rsplit("::").next().unwrap_or(signature))
}

fn parent_scope_owner(parent_qn: &str) -> Option<String> {
    if parent_qn.contains("::class::") || parent_qn.contains("::struct::") {
        return parent_qn.rsplit("::").next().map(|part| part.to_owned());
    }
    None
}

fn normalize_name(name: &str) -> String {
    name.trim_matches('&')
        .trim_matches('*')
        .split('<')
        .next()
        .unwrap_or(name)
        .trim()
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
        language: "cpp".to_owned(),
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
        let (pf, _) = CppParser.parse(&ParseContext {
            rel_path: "src/native.cpp",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_namespaces_types_methods_templates_includes_and_calls() {
        let pf = parse(
            "#include <vector>\nnamespace demo {\ntemplate <typename T> class Box {};\nclass Runner { public: void helper() {} void run() { helper(); } };\nvoid free_fn() {}\n}\n",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/native.cpp::namespace::demo")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/native.cpp::class::Box"
                    && node.modifiers.as_deref() == Some("template"))
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/native.cpp::class::Runner")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "vector")
        );
        assert!(pf.edges.iter().any(|edge| edge.kind == EdgeKind::Calls
            && edge.target_qn == "src/native.cpp::method::Runner.helper"));
    }
}
