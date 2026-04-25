use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use std::collections::HashMap;
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

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("tree-sitter-python grammar failed to load");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();

        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                visit_toplevel(child, ctx, &mut nodes, &mut edges);
            }

            // Second pass: same-file call resolution.
            let mut call_edges = resolve_python_calls(root, ctx.source, ctx.rel_path, &nodes);
            edges.append(&mut call_edges);
        }

        let pf = ParsedFile {
            path: ctx.rel_path.to_owned(),
            language: Some("python".to_owned()),
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
    visit_function_with_decorators(node, ctx, parent_qn, in_class, &[], nodes, edges);
}

fn visit_function_with_decorators(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    in_class: bool,
    decorators: &[serde_json::Value],
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
        id: NodeId::UNSET,
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
        extra_json: decorator_extra_json(decorators),
    });
    edges.push(contains_edge(
        parent_qn,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

fn visit_class(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    visit_class_with_decorators(node, ctx, &[], nodes, edges);
}

fn visit_class_with_decorators(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    decorators: &[serde_json::Value],
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
        id: NodeId::UNSET,
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
        extra_json: decorator_extra_json(decorators),
    });
    edges.push(contains_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));

    // Walk class body for methods and nested class defs.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_definition" => {
                    visit_function(child, ctx, &qn, true, nodes, edges);
                }
                "decorated_definition" => {
                    let decorators = decorated_metadata(child, ctx.source);
                    if let Some(def) = child.child_by_field_name("definition") {
                        match def.kind() {
                            "function_definition" => visit_function_with_decorators(
                                def,
                                ctx,
                                &qn,
                                true,
                                &decorators,
                                nodes,
                                edges,
                            ),
                            "class_definition" => {
                                visit_class_with_decorators(def, ctx, &decorators, nodes, edges);
                            }
                            _ => {}
                        }
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
    let decorators = decorated_metadata(node, ctx.source);
    if let Some(def) = node.child_by_field_name("definition") {
        match def.kind() {
            "function_definition" => visit_function_with_decorators(
                def,
                ctx,
                ctx.rel_path,
                false,
                &decorators,
                nodes,
                edges,
            ),
            "class_definition" => visit_class_with_decorators(def, ctx, &decorators, nodes, edges),
            _ => {}
        }
    }
}

fn decorated_metadata(node: TsNode<'_>, source: &[u8]) -> Vec<serde_json::Value> {
    let mut decorators = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "decorator" {
            continue;
        }
        let raw = node_text(child, source).trim();
        let text = raw.trim_start_matches('@').trim();
        if text.is_empty() {
            continue;
        }
        let name = text
            .split('(')
            .next()
            .unwrap_or(text)
            .rsplit('.')
            .next()
            .unwrap_or(text)
            .trim();
        decorators.push(serde_json::json!({
            "name": name,
            "text": text,
        }));
    }
    decorators
}

fn decorator_extra_json(decorators: &[serde_json::Value]) -> serde_json::Value {
    if decorators.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::json!({
            "decorators": decorators,
        })
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
                let module_name = node_text(child, ctx.source);
                emit_import(
                    ctx,
                    module_name,
                    vec![import_binding_json(
                        last_python_segment(module_name),
                        module_name,
                        "module",
                    )],
                    None,
                    child,
                    nodes,
                    edges,
                );
            }
            "aliased_import" => {
                // `import foo as bar` — record the original module name.
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(n, ctx.source))
                    .unwrap_or_else(|| node_text(child, ctx.source));
                let local_name = child
                    .child_by_field_name("alias")
                    .map(|n| node_text(n, ctx.source))
                    .unwrap_or_else(|| last_python_segment(name));
                emit_import(
                    ctx,
                    name,
                    vec![import_binding_json(local_name, name, "module")],
                    None,
                    child,
                    nodes,
                    edges,
                );
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
    let relative_level = import_from_relative_level(node_text(node, ctx.source));
    let module = node
        .child_by_field_name("module_name")
        .map(|n| node_text(n, ctx.source))
        .unwrap_or("");
    let mut bindings = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                let imported = node_text(child, ctx.source);
                if imported != module {
                    bindings.push(import_binding_json(imported, imported, "from"));
                }
            }
            "aliased_import" => {
                let imported = child
                    .child_by_field_name("name")
                    .map(|n| node_text(n, ctx.source))
                    .unwrap_or_else(|| node_text(child, ctx.source));
                let local = child
                    .child_by_field_name("alias")
                    .map(|n| node_text(n, ctx.source))
                    .unwrap_or(imported);
                bindings.push(import_binding_json(local, imported, "from"));
            }
            "wildcard_import" => {
                bindings.push(import_binding_json("*", "*", "wildcard"));
            }
            _ => {}
        }
    }
    emit_import(
        ctx,
        module,
        bindings,
        Some(relative_level),
        node,
        nodes,
        edges,
    );
}

fn emit_import(
    ctx: &ParseContext<'_>,
    module_name: &str,
    bindings: Vec<serde_json::Value>,
    relative_level: Option<usize>,
    anchor: TsNode<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let qn = format!("{}::import::{}", ctx.rel_path, module_name);
    nodes.push(Node {
        id: NodeId::UNSET,
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
        extra_json: serde_json::json!({
            "source": module_name,
            "bindings": bindings,
            "relative_level": relative_level.unwrap_or(0),
        }),
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
// Same-file call resolution (Python)
// ---------------------------------------------------------------------------

fn import_binding_json(local: &str, imported: &str, kind: &str) -> serde_json::Value {
    serde_json::json!({
        "local": local,
        "imported": imported,
        "kind": kind,
    })
}

fn last_python_segment(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

fn import_from_relative_level(statement: &str) -> usize {
    let trimmed = statement.trim();
    let Some(rest) = trimmed.strip_prefix("from") else {
        return 0;
    };
    rest.trim_start()
        .chars()
        .take_while(|ch| *ch == '.')
        .count()
}

fn resolve_python_calls(
    root: TsNode<'_>,
    source: &[u8],
    rel_path: &str,
    nodes: &[Node],
) -> Vec<Edge> {
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
    walk_python_calls(root, source, rel_path, &callables, &mut scope, &mut edges);
    edges
}

fn walk_python_calls<'a>(
    node: TsNode<'a>,
    source: &[u8],
    rel_path: &str,
    callables: &HashMap<String, String>,
    scope: &mut Vec<String>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "function_definition" => {
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
                walk_python_calls(child, source, rel_path, callables, scope, edges);
            }
            if pushed {
                scope.pop();
            }
            return;
        }
        "call" => {
            if let Some(caller_qn) = scope.last().cloned() {
                // `function` field holds the called expression.
                let called = node
                    .child_by_field_name("function")
                    .and_then(|f| python_call_target(f, source));
                if let Some((text, name, receiver)) = called
                    && !is_self_call(&caller_qn, &name, receiver.as_deref())
                {
                    if let Some(callee_qn) = callables.get(&name)
                        && *callee_qn != caller_qn
                    {
                        edges.push(py_call_edge(
                            &caller_qn,
                            callee_qn,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            true,
                        ));
                    } else {
                        edges.push(py_call_edge(
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
        walk_python_calls(child, source, rel_path, callables, scope, edges);
    }
}

fn python_call_target(node: TsNode<'_>, source: &[u8]) -> Option<(String, String, Option<String>)> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source).to_owned();
            Some((name.clone(), name, None))
        }
        "attribute" => {
            let callee = node.child_by_field_name("attribute")?;
            let receiver = node.child_by_field_name("object")?;
            let callee_name = node_text(callee, source).to_owned();
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

fn py_call_edge(
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ParseContext;

    fn parse(src: &str) -> ParsedFile {
        let (pf, _) = PythonParser.parse(&ParseContext {
            rel_path: "src/example.py",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
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
        let f = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.name == "hello");
        assert!(f.is_some(), "function node not found");
        let f = f.unwrap();
        assert!(f.params.is_some());
        assert!(f.return_type.is_some());
    }

    #[test]
    fn extracts_class() {
        let pf = parse("class Foo:\n    pass\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Class && n.name == "Foo")
        );
    }

    #[test]
    fn extracts_method() {
        let pf = parse("class Foo:\n    def bar(self):\n        pass\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Method && n.name == "bar")
        );
    }

    #[test]
    fn test_function_detected() {
        let pf = parse("def test_addition():\n    assert 1 + 1 == 2\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "test_addition"),
            "test node not found"
        );
    }

    #[test]
    fn test_method_detected() {
        let pf = parse("class TestFoo:\n    def test_bar(self):\n        pass\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "test_bar"),
            "test method node not found"
        );
    }

    #[test]
    fn import_statement_edges() {
        let pf = parse("import os\n");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Import && n.name == "os")
        );
    }

    #[test]
    fn import_from_statement_edges() {
        let pf = parse("from os.path import join\n");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
    }

    #[test]
    fn decorated_function_extracted() {
        let pf = parse("@staticmethod\ndef compute():\n    pass\n");
        let node = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.name == "compute")
            .expect("decorated function node");
        let decorators = node
            .extra_json
            .get("decorators")
            .and_then(|value| value.as_array())
            .expect("decorators metadata");
        assert_eq!(decorators.len(), 1);
        assert_eq!(decorators[0]["name"], "staticmethod");
    }

    #[test]
    fn decorated_class_extracted() {
        let pf = parse("@dataclass\nclass Example:\n    pass\n");
        let node = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Class && n.name == "Example")
            .expect("decorated class node");
        let decorators = node
            .extra_json
            .get("decorators")
            .and_then(|value| value.as_array())
            .expect("decorators metadata");
        assert_eq!(decorators[0]["name"], "dataclass");
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

    #[test]
    fn same_file_call_resolved() {
        let src = "def helper():\n    pass\n\ndef caller():\n    helper()\n";
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected Calls edge from caller to helper; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn unresolved_call_keeps_text_target() {
        let src = "def caller():\n    imported.helper()\n";
        let pf = parse(src);
        let edge = pf
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Calls)
            .expect("call edge");
        assert_eq!(edge.target_qn, "imported.helper");
        assert_eq!(edge.confidence_tier.as_deref(), Some("text"));
    }
}
