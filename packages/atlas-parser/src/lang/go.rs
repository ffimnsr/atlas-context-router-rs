use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use std::collections::HashMap;
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, field_text, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct GoParser;

struct GoPackage {
    name: String,
    line: u32,
}

impl LangParser for GoParser {
    fn language_name(&self) -> &'static str {
        "go"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".go")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .expect("tree-sitter-go grammar failed to load");

        let tree = parser.parse(ctx.source, ctx.old_tree);
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();

        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let package = find_package(root, ctx.source);
            let package_qn = format!("{}::package::{}", ctx.rel_path, package.name);

            nodes.push(package_node(
                ctx.rel_path,
                ctx.file_hash,
                &package.name,
                &package_qn,
                package.line,
            ));
            edges.push(contains_edge(
                ctx.rel_path,
                &package_qn,
                ctx.rel_path,
                package.line,
            ));

            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                match child.kind() {
                    "function_declaration" => {
                        visit_function(child, ctx, &package_qn, &mut nodes, &mut edges);
                    }
                    "method_declaration" => {
                        visit_method(child, ctx, &package_qn, &mut nodes, &mut edges);
                    }
                    "type_declaration" => {
                        visit_type_decl(child, ctx, &package_qn, &mut nodes, &mut edges);
                    }
                    "import_declaration" => {
                        visit_imports(child, ctx, &package_qn, &mut nodes, &mut edges);
                    }
                    _ => {}
                }
            }

            // Second pass: same-file call resolution.
            let mut call_edges = resolve_go_calls(root, ctx.source, ctx.rel_path, &nodes);
            edges.append(&mut call_edges);
        }

        let pf = ParsedFile {
            path: ctx.rel_path.to_owned(),
            language: Some("go".to_owned()),
            hash: ctx.file_hash.to_owned(),
            size: Some(ctx.source.len() as i64),
            nodes,
            edges,
        };
        (pf, tree)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn file_node(rel_path: &str, file_hash: &str, line_end: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned(),
        qualified_name: rel_path.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: 1,
        line_end,
        language: "go".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    }
}

fn package_node(rel_path: &str, file_hash: &str, package_name: &str, qn: &str, line: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::Package,
        name: package_name.to_owned(),
        qualified_name: qn.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: line,
        line_end: line,
        language: "go".to_owned(),
        parent_name: Some(rel_path.to_owned()),
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

fn find_package(root: TsNode<'_>, source: &[u8]) -> GoPackage {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_clause" {
            // package_clause: `package <identifier>`
            let mut cc = child.walk();
            for c in child.children(&mut cc) {
                if c.kind() == "package_identifier" || c.kind() == "identifier" {
                    return GoPackage {
                        name: node_text(c, source).to_owned(),
                        line: start_line(c),
                    };
                }
            }
        }
    }
    GoPackage {
        name: "main".to_owned(),
        line: 1,
    }
}

fn visit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    package_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source) else {
        return;
    };
    let is_test = name.starts_with("Test") || name.starts_with("Benchmark");
    let kind = if is_test {
        NodeKind::Test
    } else {
        NodeKind::Function
    };
    let type_prefix = if is_test { "test" } else { "fn" };
    let qn = format!("{}::{}::{}", ctx.rel_path, type_prefix, name);
    let params = field_text(node, "parameters", ctx.source).map(|s| s.to_owned());
    let ret = field_text(node, "result", ctx.source).map(|s| s.to_owned());
    nodes.push(Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: "go".to_owned(),
        parent_name: Some(package_qn.to_owned()),
        params,
        return_type: ret,
        modifiers: None,
        is_test,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(
        package_qn,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

fn visit_method(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    package_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source) else {
        return;
    };
    // Receiver type is inside the `receiver` field: `(t *Type)` → extract Type.
    let receiver_type = node
        .child_by_field_name("receiver")
        .and_then(|r| {
            let mut c = r.walk();
            r.children(&mut c)
                .find(|n| n.kind() == "type_identifier" || n.kind() == "pointer_type")
                .map(|n| node_text(n, ctx.source).trim_start_matches('*').to_owned())
        })
        .unwrap_or_default();

    let qn = format!("{}::method::{}.{}", ctx.rel_path, receiver_type, name);
    let params = field_text(node, "parameters", ctx.source).map(|s| s.to_owned());
    let ret = field_text(node, "result", ctx.source).map(|s| s.to_owned());
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Method,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: "go".to_owned(),
        parent_name: Some(package_qn.to_owned()),
        params,
        return_type: ret,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(
        package_qn,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

/// Walk a `type_declaration` which may contain multiple `type_spec` children.
fn visit_type_decl(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    package_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_spec" {
            visit_type_spec(child, ctx, package_qn, nodes, edges);
        }
    }
}

fn visit_type_spec(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    package_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source) else {
        return;
    };
    // Determine if it's a struct or interface by looking at the `type` field.
    let (kind, type_prefix) = if let Some(type_node) = node.child_by_field_name("type") {
        match type_node.kind() {
            "struct_type" => (NodeKind::Struct, "struct"),
            "interface_type" => (NodeKind::Interface, "interface"),
            _ => (NodeKind::Class, "type"),
        }
    } else {
        (NodeKind::Class, "type")
    };
    let qn = format!("{}::{}::{}", ctx.rel_path, type_prefix, name);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: "go".to_owned(),
        parent_name: Some(package_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(
        package_qn,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

fn visit_imports(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    package_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_spec" {
            let mut ic = child.walk();
            for n in child.children(&mut ic) {
                if n.kind() == "interpreted_string_literal" || n.kind() == "raw_string_literal" {
                    let raw = node_text(n, ctx.source);
                    let path = raw.trim_matches('"').trim_matches('`');
                    let qn = format!("{}::import::{}", ctx.rel_path, path);
                    let alias = child
                        .child_by_field_name("name")
                        .map(|name| node_text(name, ctx.source).to_owned())
                        .or_else(|| {
                            let mut cc = child.walk();
                            child
                                .children(&mut cc)
                                .find(|part| part.kind() == "identifier")
                                .map(|part| node_text(part, ctx.source).to_owned())
                        })
                        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_owned());
                    nodes.push(Node {
                        id: NodeId::UNSET,
                        kind: NodeKind::Import,
                        name: path.to_owned(),
                        qualified_name: qn.clone(),
                        file_path: ctx.rel_path.to_owned(),
                        line_start: start_line(n),
                        line_end: end_line(n),
                        language: "go".to_owned(),
                        parent_name: Some(package_qn.to_owned()),
                        params: None,
                        return_type: None,
                        modifiers: None,
                        is_test: false,
                        file_hash: ctx.file_hash.to_owned(),
                        extra_json: serde_json::json!({
                            "source": path,
                            "bindings": [
                                {
                                    "local": alias,
                                    "imported": path,
                                    "kind": "package"
                                }
                            ],
                        }),
                    });
                    edges.push(Edge {
                        id: 0,
                        kind: EdgeKind::Imports,
                        source_qn: package_qn.to_owned(),
                        target_qn: qn,
                        file_path: ctx.rel_path.to_owned(),
                        line: Some(start_line(n)),
                        confidence: 1.0,
                        confidence_tier: Some("definite".to_owned()),
                        extra_json: serde_json::Value::Null,
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
// Same-file call resolution (Go)
// ---------------------------------------------------------------------------

fn resolve_go_calls(root: TsNode<'_>, source: &[u8], rel_path: &str, nodes: &[Node]) -> Vec<Edge> {
    let mut callables: HashMap<String, String> = HashMap::new();
    for n in nodes {
        if matches!(
            n.kind,
            NodeKind::Function | NodeKind::Method | NodeKind::Test
        ) {
            callables.insert(n.name.clone(), n.qualified_name.clone());
        }
    }
    let mut edges = Vec::new();
    let mut scope: Vec<String> = Vec::new();
    walk_go_calls(root, source, rel_path, &callables, &mut scope, &mut edges);
    edges
}

fn walk_go_calls<'a>(
    node: TsNode<'a>,
    source: &[u8],
    rel_path: &str,
    callables: &HashMap<String, String>,
    scope: &mut Vec<String>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "function_declaration" | "method_declaration" => {
            let pushed = if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                if let Some(qn) = callables.get(name) {
                    scope.push(qn.clone());
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_go_calls(child, source, rel_path, callables, scope, edges);
            }
            if pushed {
                scope.pop();
            }
            return;
        }
        "call_expression" => {
            if let Some(caller_qn) = scope.last().cloned() {
                // In Go, call_expression.function can be identifier or selector_expression.
                let called = node
                    .child_by_field_name("function")
                    .and_then(|f| go_call_target(f, source));
                if let Some((text, name, receiver)) = called
                    && !is_self_call(&caller_qn, &name, receiver.as_deref())
                {
                    if let Some(callee_qn) = callables.get(&name)
                        && *callee_qn != caller_qn
                    {
                        edges.push(go_call_edge(
                            &caller_qn,
                            callee_qn,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            true,
                        ));
                    } else {
                        edges.push(go_call_edge(
                            &caller_qn,
                            &text,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            false,
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_go_calls(child, source, rel_path, callables, scope, edges);
    }
}

fn go_call_target(node: TsNode<'_>, source: &[u8]) -> Option<(String, String, Option<String>)> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source).to_owned();
            Some((name.clone(), name, None))
        }
        "selector_expression" => {
            let field = node.child_by_field_name("field")?;
            let receiver = node.child_by_field_name("operand")?;
            let callee_name = node_text(field, source).to_owned();
            let receiver_text = node_text(receiver, source).to_owned();
            Some((
                node_text(node, source).to_owned(),
                callee_name,
                Some(receiver_text),
            ))
        }
        _ => None,
    }
}

fn is_self_call(caller_qn: &str, callee_name: &str, receiver: Option<&str>) -> bool {
    if receiver.is_some() {
        return false;
    }
    caller_simple_name(caller_qn) == callee_name
}

fn caller_simple_name(caller_qn: &str) -> &str {
    caller_qn
        .rsplit("::")
        .next()
        .unwrap_or(caller_qn)
        .rsplit('.')
        .next()
        .unwrap_or(caller_qn)
}

fn go_call_edge(
    caller: &str,
    callee: &str,
    rel_path: &str,
    line: u32,
    text: &str,
    receiver: Option<&str>,
    same_file: bool,
) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: caller.to_owned(),
        target_qn: callee.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: if same_file { 0.8 } else { 0.3 },
        confidence_tier: Some(if same_file { "same_file" } else { "text" }.to_owned()),
        extra_json: serde_json::json!({
            "callee_text": text,
            "callee_name": caller_simple_name(callee),
            "receiver_text": receiver,
        }),
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ParseContext;

    fn parse(src: &str) -> ParsedFile {
        let p = GoParser;
        let (pf, _) = p.parse(&ParseContext {
            rel_path: "cmd/main.go",
            file_hash: "cafebabe",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn file_node_present() {
        let pf = parse("package main\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
    }

    #[test]
    fn package_node_present() {
        let pf = parse("package widgets\nfunc Hello() {}\n");
        let package = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Package)
            .expect("package node");
        assert_eq!(package.name, "widgets");
        assert_eq!(package.qualified_name, "cmd/main.go::package::widgets");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Contains
            && e.source_qn == "cmd/main.go"
            && e.target_qn == package.qualified_name));
        assert!(pf.nodes.iter().any(|n| n.name == "Hello"
            && n.parent_name.as_deref() == Some(package.qualified_name.as_str())));
    }

    #[test]
    fn extracts_function() {
        let pf = parse("package main\nfunc Hello() string { return \"\" }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "Hello")
        );
    }

    #[test]
    fn extracts_struct() {
        let pf = parse("package main\ntype Foo struct { x int }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Struct && n.name == "Foo")
        );
    }

    #[test]
    fn extracts_interface() {
        let pf = parse("package main\ntype Reader interface { Read(p []byte) (int, error) }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Interface && n.name == "Reader")
        );
    }

    #[test]
    fn extracts_method() {
        let pf = parse("package main\ntype Foo struct{}\nfunc (f *Foo) Bar() {}");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Method && n.name == "Bar")
        );
    }

    #[test]
    fn test_function_detected() {
        let pf = parse("package main\nfunc TestFoo(t *testing.T) {}");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "TestFoo")
        );
    }

    #[test]
    fn import_edges() {
        let pf = parse("package main\nimport \"fmt\"\nfunc main() {}");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
    }

    #[test]
    fn same_file_call_resolved() {
        let src = "package main\nfunc helper() {}\nfunc caller() { helper() }";
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected Calls edge; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn unresolved_call_keeps_text_target() {
        let src = "package main\nfunc caller() { helpers.Run() }";
        let pf = parse(src);
        let edge = pf
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Calls)
            .expect("call edge");
        assert_eq!(edge.target_qn, "helpers.Run");
        assert_eq!(edge.confidence_tier.as_deref(), Some("text"));
    }
}
