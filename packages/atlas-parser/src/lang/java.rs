//! Java parser backed by `tree-sitter-java`.
//! Grammar source: `tree-sitter/tree-sitter-java`.

use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct JavaParser;

impl LangParser for JavaParser {
    fn language_name(&self) -> &'static str {
        "java"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".java")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .expect("tree-sitter-java grammar failed to load");

        let tree = parser.parse(ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let package_qn = emit_package(root, ctx, &mut nodes, &mut edges);
            let parent_qn = package_qn.as_deref().unwrap_or(ctx.rel_path).to_owned();
            let mut import_index = 0usize;
            let mut annotation_index = 0usize;

            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                visit_java_node(
                    child,
                    ctx,
                    &parent_qn,
                    None,
                    &mut import_index,
                    &mut annotation_index,
                    &mut nodes,
                    &mut edges,
                );
            }

            let method_map = method_qn_map(&nodes);
            let mut call_edges = Vec::new();
            walk_method_calls(root, ctx, &method_map, None, &mut call_edges);
            edges.extend(call_edges);
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("java".to_owned()),
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
fn visit_java_node(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    current_type: Option<&str>,
    import_index: &mut usize,
    annotation_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "package_declaration" => {}
        "import_declaration" => emit_import(node, ctx, parent_qn, import_index, nodes, edges),
        "class_declaration" => emit_type(
            node,
            ctx,
            parent_qn,
            "class",
            NodeKind::Class,
            import_index,
            annotation_index,
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
            annotation_index,
            nodes,
            edges,
        ),
        "enum_declaration" => emit_type(
            node,
            ctx,
            parent_qn,
            "enum",
            NodeKind::Enum,
            import_index,
            annotation_index,
            nodes,
            edges,
        ),
        "annotation_type_declaration" => emit_type(
            node,
            ctx,
            parent_qn,
            "annotation_type",
            NodeKind::Interface,
            import_index,
            annotation_index,
            nodes,
            edges,
        ),
        "method_declaration" if current_type.is_some() => emit_method(
            node,
            ctx,
            parent_qn,
            current_type.expect("current_type checked"),
            annotation_index,
            nodes,
            edges,
        ),
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                visit_java_node(
                    child,
                    ctx,
                    parent_qn,
                    current_type,
                    import_index,
                    annotation_index,
                    nodes,
                    edges,
                );
            }
        }
    }
}

fn emit_package(
    root: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) -> Option<String> {
    let mut cursor = root.walk();
    let package = root
        .named_children(&mut cursor)
        .find(|child| child.kind() == "package_declaration")?;
    let name = child_name_or_text(package, ctx.source, &["scoped_identifier", "identifier"])?;
    let qn = format!("{}::package::{}", ctx.rel_path, name);
    let line = start_line(package);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Package,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(package),
        language: "java".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, line));
    Some(qn)
}

#[allow(clippy::too_many_arguments)]
fn emit_type(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    qn_prefix: &str,
    kind: NodeKind,
    import_index: &mut usize,
    annotation_index: &mut usize,
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
        language: "java".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
    emit_annotations(node, ctx, &qn, annotation_index, nodes, edges);

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            visit_java_node(
                child,
                ctx,
                &qn,
                Some(&name),
                import_index,
                annotation_index,
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
    owner_name: &str,
    annotation_index: &mut usize,
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
        language: "java".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: node
            .child_by_field_name("parameters")
            .map(|params| node_text(params, ctx.source).to_owned()),
        return_type: node
            .child_by_field_name("type")
            .map(|ret| node_text(ret, ctx.source).to_owned()),
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
    emit_annotations(node, ctx, &qn, annotation_index, nodes, edges);
}

fn emit_import(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = child_name_or_text(
        node,
        ctx.source,
        &["scoped_identifier", "identifier", "asterisk"],
    ) else {
        return;
    };
    *import_index += 1;
    let qn = format!("{}::import::java:{}", ctx.rel_path, *import_index);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Import,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "java".to_owned(),
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
        "explicit_import",
    ));
}

fn emit_annotations(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    owner_qn: &str,
    annotation_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "modifiers" {
            emit_annotations(child, ctx, owner_qn, annotation_index, nodes, edges);
            continue;
        }
        if child.kind() != "annotation" && child.kind() != "marker_annotation" {
            continue;
        }
        let Some(name) =
            child_name_or_text(child, ctx.source, &["identifier", "scoped_identifier"])
        else {
            continue;
        };
        *annotation_index += 1;
        let qn = format!(
            "{}::annotation::{}#{}",
            ctx.rel_path, owner_qn, *annotation_index
        );
        let line = start_line(child);
        nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Variable,
            name: name.clone(),
            qualified_name: qn.clone(),
            file_path: ctx.rel_path.to_owned(),
            line_start: line,
            line_end: end_line(child),
            language: "java".to_owned(),
            parent_name: Some(owner_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: Some("annotation".to_owned()),
            is_test: false,
            file_hash: ctx.file_hash.to_owned(),
            extra_json: serde_json::json!({ "kind": "annotation" }),
        });
        edges.push(contains_edge(owner_qn, &qn, ctx.rel_path, line));
    }
}

fn method_qn_map(nodes: &[Node]) -> HashMap<String, String> {
    nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Method)
        .map(|node| (node.name.clone(), node.qualified_name.clone()))
        .collect()
}

fn walk_method_calls(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    methods: &HashMap<String, String>,
    current_method: Option<String>,
    edges: &mut Vec<Edge>,
) {
    let mut next_method = current_method;
    if node.kind() == "method_declaration" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, ctx.source);
            next_method = methods.get(name).cloned();
        }
    } else if node.kind() == "method_invocation"
        && let Some(owner_qn) = next_method.as_ref()
        && let Some(name_node) = node.child_by_field_name("name")
    {
        let callee = node_text(name_node, ctx.source);
        if let Some(target_qn) = methods.get(callee) {
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
        walk_method_calls(child, ctx, methods, next_method.clone(), edges);
    }
}

fn child_name_or_text(node: TsNode<'_>, source: &[u8], kinds: &[&str]) -> Option<String> {
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
        language: "java".to_owned(),
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
        let (pf, _) = JavaParser.parse(&ParseContext {
            rel_path: "src/Main.java",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_package_types_methods_imports_and_calls() {
        let pf = parse(
            "package demo.app;\nimport java.util.List;\n@Svc\nclass Main {\n  @Trace\n  void run() { helper(); }\n  void helper() {}\n}\ninterface Api { void ping(); }\nenum Mode { ON }\n",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.java::package::demo.app")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.java::class::Main")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.java::interface::Api")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.java::enum::Mode")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "java.util.List")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Variable && node.name == "Svc")
        );
        assert!(pf.edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls && edge.target_qn == "src/Main.java::method::Main.helper"
        }));
    }
}
