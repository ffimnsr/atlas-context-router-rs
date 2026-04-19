use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use std::collections::HashMap;
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, field_text, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct RustParser;

impl LangParser for RustParser {
    fn language_name(&self) -> &'static str {
        "rust"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".rs")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> ParsedFile {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("tree-sitter-rust grammar failed to load");

        let tree = parser.parse(ctx.source, None);
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();

        // Always emit a File node.
        let (file_lines, _) = ctx.source.iter().fold((1u32, false), |(ln, _), &b| {
            if b == b'\n' {
                (ln + 1, true)
            } else {
                (ln, false)
            }
        });
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, file_lines));

        if let Some(tree) = tree {
            let mut walker = Walker {
                source: ctx.source,
                rel_path: ctx.rel_path,
                file_hash: ctx.file_hash,
                nodes: &mut nodes,
                edges: &mut edges,
                // Parent qualified-name stack; starts with the file node.
                scope_stack: vec![ctx.rel_path.to_owned()],
                in_test_mod: false,
            };
            walker.walk_block(tree.root_node());

            // Second pass: same-file call resolution.
            let mut call_edges =
                resolve_same_file_calls(tree.root_node(), ctx.source, ctx.rel_path, &nodes);
            edges.append(&mut call_edges);
        }

        ParsedFile {
            path: ctx.rel_path.to_owned(),
            language: Some("rust".to_owned()),
            hash: ctx.file_hash.to_owned(),
            size: Some(ctx.source.len() as i64),
            nodes,
            edges,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal walker
// ---------------------------------------------------------------------------

struct Walker<'s, 'o> {
    source: &'s [u8],
    rel_path: &'s str,
    file_hash: &'s str,
    nodes: &'o mut Vec<Node>,
    edges: &'o mut Vec<Edge>,
    /// Current parent qualified-name (top = innermost scope).
    scope_stack: Vec<String>,
    /// True when we're inside a `#[cfg(test)]` module.
    in_test_mod: bool,
}

impl<'s, 'o> Walker<'s, 'o> {
    fn current_parent_qn(&self) -> &str {
        self.scope_stack
            .last()
            .map(|s| s.as_str())
            .unwrap_or(self.rel_path)
    }

    /// Walk all children of a block/source_file.
    fn walk_block(&mut self, node: TsNode<'_>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
    }

    fn visit(&mut self, node: TsNode<'_>) {
        match node.kind() {
            "function_item" => self.visit_fn(node, false),
            "mod_item" => self.visit_mod(node),
            "struct_item" => self.visit_named_item(node, NodeKind::Struct, "struct"),
            "enum_item" => self.visit_named_item(node, NodeKind::Enum, "enum"),
            "trait_item" => self.visit_named_item(node, NodeKind::Trait, "trait"),
            "const_item" => self.visit_named_item(node, NodeKind::Constant, "const"),
            "static_item" => self.visit_named_item(node, NodeKind::Constant, "const"),
            "impl_item" => self.visit_impl(node),
            _ => {}
        }
    }

    fn visit_fn(&mut self, node: TsNode<'_>, is_method: bool) {
        let Some(name) = field_text(node, "name", self.source) else {
            return;
        };
        let is_test = self.in_test_mod || has_test_attr(node, self.source);
        let kind = if is_test {
            NodeKind::Test
        } else if is_method {
            NodeKind::Method
        } else {
            NodeKind::Function
        };

        let type_prefix = match kind {
            NodeKind::Test => "test",
            NodeKind::Method => "method",
            _ => "fn",
        };
        let parent_qn = self.current_parent_qn().to_owned();
        let qn = format!(
            "{}::{}::{}",
            self.rel_path,
            type_prefix,
            qualified_suffix(&parent_qn, self.rel_path, name)
        );
        let params = field_text(node, "parameters", self.source).map(|s| s.to_owned());
        let ret = field_text(node, "return_type", self.source).map(|s| s.to_owned());

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(node),
            line_end: end_line(node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params,
            return_type: ret,
            modifiers: visibility_modifier(node, self.source),
            is_test: is_test || self.in_test_mod,
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(node),
        ));
    }

    fn visit_mod(&mut self, node: TsNode<'_>) {
        let Some(name) = field_text(node, "name", self.source) else {
            return;
        };
        let parent_qn = self.current_parent_qn().to_owned();
        let suffix = qualified_suffix(&parent_qn, self.rel_path, name);
        let qn = format!("{}::module::{}", self.rel_path, suffix);

        // Detect #[cfg(test)] attribute on this mod.
        let was_test_mod = self.in_test_mod;
        let is_test_mod = self.in_test_mod || has_cfg_test(node, self.source);

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Module,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(node),
            line_end: end_line(node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: None,
            return_type: None,
            modifiers: visibility_modifier(node, self.source),
            is_test: is_test_mod,
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(node),
        ));

        // Recurse into inline module body.
        if let Some(body) = node.child_by_field_name("body") {
            self.scope_stack.push(qn);
            self.in_test_mod = is_test_mod;
            self.walk_block(body);
            self.scope_stack.pop();
            self.in_test_mod = was_test_mod;
        }
    }

    fn visit_named_item(&mut self, node: TsNode<'_>, kind: NodeKind, type_prefix: &str) {
        let Some(name) = field_text(node, "name", self.source) else {
            return;
        };
        let parent_qn = self.current_parent_qn().to_owned();
        let suffix = qualified_suffix(&parent_qn, self.rel_path, name);
        let qn = format!("{}::{}::{}", self.rel_path, type_prefix, suffix);

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(node),
            line_end: end_line(node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: None,
            return_type: None,
            modifiers: visibility_modifier(node, self.source),
            is_test: self.in_test_mod,
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(node),
        ));
    }

    fn visit_impl(&mut self, node: TsNode<'_>) {
        let Some(type_name) = field_text(node, "type", self.source) else {
            return;
        };
        let trait_name = field_text(node, "trait", self.source);

        // Emit an `Implements` edge if this is `impl Trait for Type`.
        if let Some(trait_name) = trait_name {
            let type_qn = format!("{}::struct::{}", self.rel_path, type_name);
            let trait_qn = format!("{}::trait::{}", self.rel_path, trait_name);
            self.edges.push(Edge {
                id: 0,
                kind: EdgeKind::Implements,
                source_qn: type_qn,
                target_qn: trait_qn,
                file_path: self.rel_path.to_owned(),
                line: Some(start_line(node)),
                confidence: 0.9,
                confidence_tier: Some("same_file".to_owned()),
                extra_json: serde_json::Value::Null,
            });
        }

        // Walk methods in the impl body.
        if let Some(body) = node.child_by_field_name("body") {
            // Push an impl-scope name for method qualified names.
            let impl_scope = format!("{}::impl::{}", self.rel_path, type_name);
            self.scope_stack.push(impl_scope);
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if child.kind() == "function_item" {
                    self.visit_fn(child, true);
                }
            }
            self.scope_stack.pop();
        }
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
        language: "rust".to_owned(),
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

/// For nested scopes the method QN includes a disambiguating suffix from the
/// parent beyond the file root.  E.g. for an impl inside `mod foo`, the method
/// QN becomes `file::method::foo::Type.name`.
fn qualified_suffix(parent_qn: &str, rel_path: &str, name: &str) -> String {
    // Trim the leading `<rel_path>::<kind>::` prefix of the parent, if any.
    let parent_tail = parent_qn
        .strip_prefix(rel_path)
        .and_then(|s| s.strip_prefix("::"))
        .and_then(|s| s.split_once("::").map(|x| x.1))
        .unwrap_or("");
    if parent_tail.is_empty() {
        name.to_owned()
    } else {
        format!("{}::{}", parent_tail, name)
    }
}

fn visibility_modifier(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            return Some(node_text(child, source).to_owned());
        }
    }
    None
}

/// Returns true if the node has a preceding `#[test]` attribute sibling.
fn has_test_attr(node: TsNode<'_>, source: &[u8]) -> bool {
    let mut sib = node.prev_named_sibling();
    while let Some(s) = sib {
        if s.kind() != "attribute_item" {
            break;
        }
        let text = node_text(s, source);
        if text.contains("test") && !text.contains("cfg") {
            return true;
        }
        sib = s.prev_named_sibling();
    }
    false
}

/// Returns true if the node has a preceding `#[cfg(test)]` attribute sibling.
fn has_cfg_test(node: TsNode<'_>, source: &[u8]) -> bool {
    let mut sib = node.prev_named_sibling();
    while let Some(s) = sib {
        if s.kind() != "attribute_item" {
            break;
        }
        let text = node_text(s, source);
        if text.contains("cfg") && text.contains("test") {
            return true;
        }
        sib = s.prev_named_sibling();
    }
    false
}

// ---------------------------------------------------------------------------
// Same-file call resolution
// ---------------------------------------------------------------------------

/// Walk `root` looking for call and method-call expressions.
/// Emits `Calls` edges (confidence=0.8, tier="same_file") for any call whose
/// callee name matches a function or method defined in the same file.
fn resolve_same_file_calls(
    root: TsNode<'_>,
    source: &[u8],
    rel_path: &str,
    nodes: &[Node],
) -> Vec<Edge> {
    // Build callable name → qualified_name map.
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
    walk_for_rust_calls(root, source, rel_path, &callables, &mut scope, &mut edges);
    edges
}

fn walk_for_rust_calls<'a>(
    node: TsNode<'a>,
    source: &[u8],
    rel_path: &str,
    callables: &HashMap<String, String>,
    scope: &mut Vec<String>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "function_item" => {
            let caller_qn = node
                .child_by_field_name("name")
                .and_then(|n| callables.get(node_text(n, source)));
            let pushed = if let Some(qn) = caller_qn {
                scope.push(qn.clone());
                true
            } else {
                false
            };
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_for_rust_calls(child, source, rel_path, callables, scope, edges);
            }
            if pushed {
                scope.pop();
            }
            return; // already recursed
        }
        "call_expression" => {
            if let Some(caller_qn) = scope.last() {
                let called = node
                    .child_by_field_name("function")
                    .and_then(|f| call_name_from_node(f, source));
                if let Some(name) = called
                    && let Some(callee_qn) = callables.get(name)
                    && callee_qn != caller_qn
                {
                    edges.push(call_edge(caller_qn, callee_qn, rel_path, start_line(node)));
                }
            }
        }
        "method_call_expression" => {
            if let Some(caller_qn) = scope.last() {
                // `method` field is the method name identifier.
                let called = node
                    .child_by_field_name("method")
                    .map(|m| node_text(m, source));
                if let Some(name) = called
                    && let Some(callee_qn) = callables.get(name)
                    && callee_qn != caller_qn
                {
                    edges.push(call_edge(caller_qn, callee_qn, rel_path, start_line(node)));
                }
            }
        }
        _ => {}
    }
    // Default recursive walk.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_rust_calls(child, source, rel_path, callables, scope, edges);
    }
}

/// Extract a bare function name from the `function` child of a call_expression.
fn call_name_from_node<'a>(node: TsNode<'a>, source: &'a [u8]) -> Option<&'a str> {
    match node.kind() {
        "identifier" => Some(node_text(node, source)),
        "scoped_identifier" => node
            .child_by_field_name("name")
            .map(|n| node_text(n, source)),
        _ => None,
    }
}

fn call_edge(caller_qn: &str, callee_qn: &str, rel_path: &str, line: u32) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: caller_qn.to_owned(),
        target_qn: callee_qn.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: 0.8,
        confidence_tier: Some("same_file".to_owned()),
        extra_json: serde_json::Value::Null,
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
        let p = RustParser;
        p.parse(&ParseContext {
            rel_path: "src/lib.rs",
            file_hash: "deadbeef",
            source: src.as_bytes(),
        })
    }

    #[test]
    fn extracts_file_node() {
        let pf = parse("fn foo() {}");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
    }

    #[test]
    fn extracts_free_function() {
        let pf = parse("pub fn greet(name: &str) -> String { todo!() }");
        let func = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function)
            .expect("function");
        assert_eq!(func.name, "greet");
        assert!(func.qualified_name.contains("fn::greet"));
    }

    #[test]
    fn extracts_struct() {
        let pf = parse("pub struct Foo { x: i32 }");
        let s = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Struct)
            .expect("struct");
        assert_eq!(s.name, "Foo");
    }

    #[test]
    fn extracts_enum() {
        let pf = parse("enum Color { Red, Green, Blue }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Enum && n.name == "Color")
        );
    }

    #[test]
    fn extracts_trait() {
        let pf = parse("pub trait Drawable { fn draw(&self); }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Trait && n.name == "Drawable")
        );
    }

    #[test]
    fn extracts_method_and_impl_edge() {
        let src = "struct Foo; impl Foo { pub fn bar(&self) {} }";
        let pf = parse(src);
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Method && n.name == "bar")
        );
    }

    #[test]
    fn implements_edge_for_trait_impl() {
        let src = "trait Greet {} struct Hi; impl Greet for Hi {}";
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Implements));
    }

    #[test]
    fn test_fn_detected() {
        let src = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
"#;
        let pf = parse(src);
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "it_works")
        );
    }

    #[test]
    fn nested_module() {
        let src = "mod outer { mod inner { fn deep() {} } }";
        let pf = parse(src);
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Module && n.name == "outer")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Module && n.name == "inner")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "deep")
        );
    }

    #[test]
    fn contains_edges_present() {
        let src = "mod foo { fn bar() {} }";
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Contains));
    }

    #[test]
    fn same_file_call_resolved() {
        let src = r#"
fn helper() {}
fn caller() { helper(); }
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected a Calls edge from caller to helper; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn method_call_resolved() {
        let src = r#"
fn helper() {}
struct S;
impl S {
    fn do_work(&self) { helper(); }
}
"#;
        let pf = parse(src);
        assert!(
            pf.edges
                .iter()
                .any(|e| e.kind == EdgeKind::Calls && e.target_qn.contains("helper")),
            "expected Calls edge to helper from method"
        );
    }

    #[test]
    fn no_self_calls_edge() {
        // A recursive call should not produce a self-loop.
        let src = r#"fn recurse(n: u32) -> u32 { if n == 0 { 0 } else { recurse(n-1) } }"#;
        let pf = parse(src);
        assert!(
            !pf.edges
                .iter()
                .any(|e| e.kind == EdgeKind::Calls && e.source_qn == e.target_qn),
            "recursive call must not produce a self-loop edge"
        );
    }
}
