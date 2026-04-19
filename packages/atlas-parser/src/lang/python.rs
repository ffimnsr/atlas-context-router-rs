use atlas_core::{Edge, EdgeKind, Node, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct PythonParser;

impl LangParser for PythonParser {
    fn language_name(&self) -> &'static str {
        "python"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".py")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> ParsedFile {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("tree-sitter-python grammar failed to load");

        let tree = parser.parse(ctx.source, None);
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();

        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(tree) = tree {
            let root = tree.root_node();
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                visit_toplevel(child, ctx, &mut nodes, &mut edges);
            }
        }

        ParsedFile {
            path: ctx.rel_path.to_owned(),
            language: Some("python".to_owned()),
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
        language: "python".to_owned(),
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

fn visit_toplevel(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "function_definition" => {
            visit_function(node, ctx, ctx.rel_path, false, nodes, edges);
        }
        "class_definition" => {
            visit_class(node, ctx, nodes, edges);
        }
        "decorated_definition" => {
            visit_decorated(node, ctx, nodes, edges);
        }
        "import_statement" => {
            visit_import(node, ctx, nodes, edges);
        }
        "import_from_statement" => {
            visit_import_from(node, ctx, nodes, edges);
        }
        _ => {}
    }
}

/// Parse a `function_definition`.
///
/// `parent_qn` is either `ctx.rel_path` (top-level) or a class qualified name.
/// `in_class` determines whether to emit a Method vs Function/Test kind.
fn visit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    in_class: bool,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, ctx.source))
        .unwrap_or("");
    if name.is_empty() {
        return;
    }

    let is_test = name.starts_with("test_");
    let (kind, type_prefix) = if in_class {
        if is_test {
            (NodeKind::Test, "test")
        } else {
            (NodeKind::Method, "method")
        }
    } else if is_test {
        (NodeKind::Test, "test")
    } else {
        (NodeKind::Function, "fn")
    };

    let qn = if in_class {
        // parent_qn = "<rel>::class::<ClassName>" — extract the last "::" segment.
        let class_name = parent_qn.split("::").last().unwrap_or("Unknown");
        format!("{}::{}::{}.{}", ctx.rel_path, type_prefix, class_name, name)
    } else {
        format!("{}::{}::{}", ctx.rel_path, type_prefix, name)
    };

    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, ctx.source).to_owned());
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| node_text(n, ctx.source).to_owned());

    nodes.push(Node {
        id: 0,
        kind,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: "python".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params,
        return_type,
        modifiers: None,
        is_test,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, start_line(node)));
}

fn visit_class(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, ctx.source))
        .unwrap_or("");
    if name.is_empty() {
        return;
    }

    let qn = format!("{}::class::{}", ctx.rel_path, name);

    nodes.push(Node {
        id: 0,
        kind: NodeKind::Class,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: "python".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: name.starts_with("Test"),
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, start_line(node)));

    // Walk class body for methods and nested class defs.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    visit_function(child, ctx, &qn, true, nodes, edges);
                }
                "decorated_definition" => {
                    if let Some(def) = child.child_by_field_name("definition")
                        && def.kind() == "function_definition"
                    {
                        visit_function(def, ctx, &qn, true, nodes, edges);
                    }
                }
                _ => {}
            }
        }
    }
}

fn visit_decorated(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    if let Some(def) = node.child_by_field_name("definition") {
        match def.kind() {
            "function_definition" => visit_function(def, ctx, ctx.rel_path, false, nodes, edges),
            "class_definition" => visit_class(def, ctx, nodes, edges),
            _ => {}
        }
    }
}

fn visit_import(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    // `import os, sys` or `import os.path`
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                emit_import(ctx, node_text(child, ctx.source), child, nodes, edges);
            }
            "aliased_import" => {
                // `import foo as bar` — record the original module name.
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(n, ctx.source))
                    .unwrap_or_else(|| node_text(child, ctx.source));
                emit_import(ctx, name, child, nodes, edges);
            }
            _ => {}
        }
    }
}

fn visit_import_from(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    // `from os.path import join` — record the source module.
    let module = node
        .child_by_field_name("module_name")
        .map(|n| node_text(n, ctx.source))
        .unwrap_or(".");
    emit_import(ctx, module, node, nodes, edges);
}

fn emit_import(
    ctx: &ParseContext<'_>,
    module_name: &str,
    anchor: TsNode<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let qn = format!("{}::import::{}", ctx.rel_path, module_name);
    nodes.push(Node {
        id: 0,
        kind: NodeKind::Import,
        name: module_name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(anchor),
        line_end: end_line(anchor),
        language: "python".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
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
        source_qn: ctx.rel_path.to_owned(),
        target_qn: qn,
        file_path: ctx.rel_path.to_owned(),
        line: Some(start_line(anchor)),
        confidence: 1.0,
        confidence_tier: Some("definite".to_owned()),
        extra_json: serde_json::Value::Null,
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ParseContext;

    fn parse(src: &str) -> ParsedFile {
        PythonParser.parse(&ParseContext {
            rel_path: "src/example.py",
            file_hash: "deadbeef",
            source: src.as_bytes(),
        })
    }

    #[test]
    fn file_node_present() {
        let pf = parse("x = 1\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
        assert_eq!(pf.language.as_deref(), Some("python"));
    }

    #[test]
    fn extracts_function() {
        let pf = parse("def hello(x: int) -> str:\n    return str(x)\n");
        let f = pf.nodes.iter().find(|n| n.kind == NodeKind::Function && n.name == "hello");
        assert!(f.is_some(), "function node not found");
        let f = f.unwrap();
        assert!(f.params.is_some());
        assert!(f.return_type.is_some());
    }

    #[test]
    fn extracts_class() {
        let pf = parse("class Foo:\n    pass\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Class && n.name == "Foo"));
    }

    #[test]
    fn extracts_method() {
        let pf = parse("class Foo:\n    def bar(self):\n        pass\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Method && n.name == "bar"));
    }

    #[test]
    fn test_function_detected() {
        let pf = parse("def test_addition():\n    assert 1 + 1 == 2\n");
        assert!(
            pf.nodes.iter().any(|n| n.kind == NodeKind::Test && n.name == "test_addition"),
            "test node not found"
        );
    }

    #[test]
    fn test_method_detected() {
        let pf = parse("class TestFoo:\n    def test_bar(self):\n        pass\n");
        assert!(
            pf.nodes.iter().any(|n| n.kind == NodeKind::Test && n.name == "test_bar"),
            "test method node not found"
        );
    }

    #[test]
    fn import_statement_edges() {
        let pf = parse("import os\n");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Import && n.name == "os"));
    }

    #[test]
    fn import_from_statement_edges() {
        let pf = parse("from os.path import join\n");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
    }

    #[test]
    fn decorated_function_extracted() {
        let pf = parse("@staticmethod\ndef compute():\n    pass\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Function && n.name == "compute"));
    }

    #[test]
    fn contains_edges_present() {
        let pf = parse("class Foo:\n    def bar(self): pass\n");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Contains));
    }

    #[test]
    fn line_spans_accurate() {
        let pf = parse("def one():\n    pass\n\ndef two():\n    pass\n");
        let one = pf.nodes.iter().find(|n| n.name == "one").unwrap();
        let two = pf.nodes.iter().find(|n| n.name == "two").unwrap();
        assert!(one.line_start < two.line_start, "line ordering wrong");
    }
}
