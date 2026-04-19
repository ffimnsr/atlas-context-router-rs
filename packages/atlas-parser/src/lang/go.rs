use atlas_core::{Edge, EdgeKind, Node, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, field_text, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct GoParser;

impl LangParser for GoParser {
    fn language_name(&self) -> &'static str {
        "go"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".go")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> ParsedFile {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .expect("tree-sitter-go grammar failed to load");

        let tree = parser.parse(ctx.source, None);
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();

        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(tree) = tree {
            let root = tree.root_node();
            let package_name = find_package_name(root, ctx.source);

            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                match child.kind() {
                    "function_declaration" => {
                        visit_function(child, ctx, &package_name, &mut nodes, &mut edges);
                    }
                    "method_declaration" => {
                        visit_method(child, ctx, &package_name, &mut nodes, &mut edges);
                    }
                    "type_declaration" => {
                        visit_type_decl(child, ctx, &package_name, &mut nodes, &mut edges);
                    }
                    "import_declaration" => {
                        visit_imports(child, ctx, &mut nodes, &mut edges);
                    }
                    _ => {}
                }
            }
        }

        ParsedFile {
            path: ctx.rel_path.to_owned(),
            language: Some("go".to_owned()),
            hash: ctx.file_hash.to_owned(),
            size: Some(ctx.source.len() as i64),
            nodes,
            edges,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn file_node(rel_path: &str, file_hash: &str, line_end: u32) -> Node {
    Node {
        id: 0,
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

fn find_package_name(root: TsNode<'_>, source: &[u8]) -> String {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_clause" {
            // package_clause: `package <identifier>`
            let mut cc = child.walk();
            for c in child.children(&mut cc) {
                if c.kind() == "package_identifier" || c.kind() == "identifier" {
                    return node_text(c, source).to_owned();
                }
            }
        }
    }
    "main".to_owned()
}

fn visit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    _package: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source) else { return };
    let is_test = name.starts_with("Test") || name.starts_with("Benchmark");
    let kind = if is_test { NodeKind::Test } else { NodeKind::Function };
    let type_prefix = if is_test { "test" } else { "fn" };
    let qn = format!("{}::{}::{}", ctx.rel_path, type_prefix, name);
    let params = field_text(node, "parameters", ctx.source).map(|s| s.to_owned());
    let ret = field_text(node, "result", ctx.source).map(|s| s.to_owned());
    let file_qn = ctx.rel_path;

    nodes.push(Node {
        id: 0,
        kind,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: "go".to_owned(),
        parent_name: Some(file_qn.to_owned()),
        params,
        return_type: ret,
        modifiers: None,
        is_test,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(file_qn, &qn, ctx.rel_path, start_line(node)));
}

fn visit_method(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    _package: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source) else { return };
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
    let file_qn = ctx.rel_path;

    nodes.push(Node {
        id: 0,
        kind: NodeKind::Method,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: "go".to_owned(),
        parent_name: Some(file_qn.to_owned()),
        params,
        return_type: ret,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(file_qn, &qn, ctx.rel_path, start_line(node)));
}

/// Walk a `type_declaration` which may contain multiple `type_spec` children.
fn visit_type_decl(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    _package: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_spec" {
            visit_type_spec(child, ctx, nodes, edges);
        }
    }
}

fn visit_type_spec(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source) else { return };
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
    let file_qn = ctx.rel_path;

    nodes.push(Node {
        id: 0,
        kind,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: "go".to_owned(),
        parent_name: Some(file_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(file_qn, &qn, ctx.rel_path, start_line(node)));
}

fn visit_imports(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let file_qn = ctx.rel_path;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_spec" {
            let mut ic = child.walk();
            for n in child.children(&mut ic) {
                if n.kind() == "interpreted_string_literal" || n.kind() == "raw_string_literal" {
                    let raw = node_text(n, ctx.source);
                    let path = raw.trim_matches('"').trim_matches('`');
                    let qn = format!("{}::import::{}", ctx.rel_path, path);
                    nodes.push(Node {
                        id: 0,
                        kind: NodeKind::Import,
                        name: path.to_owned(),
                        qualified_name: qn.clone(),
                        file_path: ctx.rel_path.to_owned(),
                        line_start: start_line(n),
                        line_end: end_line(n),
                        language: "go".to_owned(),
                        parent_name: Some(file_qn.to_owned()),
                        params: None,
                        return_type: None,
                        modifiers: None,
                        is_test: false,
                        file_hash: ctx.file_hash.to_owned(),
                        extra_json: serde_json::Value::Null,
                    });
                    edges.push(Edge {
                        id: 0,
                        kind: EdgeKind::Imports,
                        source_qn: file_qn.to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ParseContext;

    fn parse(src: &str) -> ParsedFile {
        let p = GoParser;
        p.parse(&ParseContext {
            rel_path: "cmd/main.go",
            file_hash: "cafebabe",
            source: src.as_bytes(),
        })
    }

    #[test]
    fn file_node_present() {
        let pf = parse("package main\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
    }

    #[test]
    fn extracts_function() {
        let pf = parse("package main\nfunc Hello() string { return \"\" }");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Function && n.name == "Hello"));
    }

    #[test]
    fn extracts_struct() {
        let pf = parse("package main\ntype Foo struct { x int }");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Struct && n.name == "Foo"));
    }

    #[test]
    fn extracts_interface() {
        let pf = parse("package main\ntype Reader interface { Read(p []byte) (int, error) }");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Interface && n.name == "Reader"));
    }

    #[test]
    fn extracts_method() {
        let pf = parse("package main\ntype Foo struct{}\nfunc (f *Foo) Bar() {}");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Method && n.name == "Bar"));
    }

    #[test]
    fn test_function_detected() {
        let pf = parse("package main\nfunc TestFoo(t *testing.T) {}");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Test && n.name == "TestFoo"));
    }

    #[test]
    fn import_edges() {
        let pf = parse("package main\nimport \"fmt\"\nfunc main() {}");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
    }
}
