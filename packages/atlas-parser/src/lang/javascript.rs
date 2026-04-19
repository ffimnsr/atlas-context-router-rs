use atlas_core::{Edge, EdgeKind, Node, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

// ---------------------------------------------------------------------------
// Public parser types
// ---------------------------------------------------------------------------

/// JavaScript / JSX parser (`.js`, `.jsx`, `.mjs`, `.cjs`).
pub struct JsParser;

/// TypeScript / TSX parser (`.ts`, `.tsx`).
pub struct TsParser;

impl LangParser for JsParser {
    fn language_name(&self) -> &'static str {
        "javascript"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".js")
            || path.ends_with(".jsx")
            || path.ends_with(".mjs")
            || path.ends_with(".cjs")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> ParsedFile {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .expect("tree-sitter-javascript grammar failed to load");
        parse_source(&mut parser, ctx, "javascript")
    }
}

impl LangParser for TsParser {
    fn language_name(&self) -> &'static str {
        "typescript"
    }

    fn supports(&self, path: &str) -> bool {
        // Skip declaration files (.d.ts) — they carry no runtime symbols.
        if path.ends_with(".d.ts") {
            return false;
        }
        path.ends_with(".ts") || path.ends_with(".tsx")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> ParsedFile {
        let mut parser = tree_sitter::Parser::new();
        let lang = if ctx.rel_path.ends_with(".tsx") {
            tree_sitter_typescript::LANGUAGE_TSX.into()
        } else {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        };
        parser
            .set_language(&lang)
            .expect("tree-sitter-typescript grammar failed to load");
        parse_source(&mut parser, ctx, "typescript")
    }
}

// ---------------------------------------------------------------------------
// Core parsing (shared between JS and TS)
// ---------------------------------------------------------------------------

fn parse_source(
    parser: &mut tree_sitter::Parser,
    ctx: &ParseContext<'_>,
    lang: &'static str,
) -> ParsedFile {
    let tree = parser.parse(ctx.source, None);
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();

    let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
    nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count, lang));

    if let Some(tree) = tree {
        let root = tree.root_node();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            visit_toplevel(child, ctx, lang, &mut nodes, &mut edges);
        }
    }

    ParsedFile {
        path: ctx.rel_path.to_owned(),
        language: Some(lang.to_owned()),
        hash: ctx.file_hash.to_owned(),
        size: Some(ctx.source.len() as i64),
        nodes,
        edges,
    }
}

// ---------------------------------------------------------------------------
// Node constructors
// ---------------------------------------------------------------------------

fn file_node(rel_path: &str, file_hash: &str, line_end: u32, lang: &str) -> Node {
    Node {
        id: 0,
        kind: NodeKind::File,
        name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned(),
        qualified_name: rel_path.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: 1,
        line_end,
        language: lang.to_owned(),
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

// ---------------------------------------------------------------------------
// Visitors
// ---------------------------------------------------------------------------

fn visit_toplevel(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            visit_function(node, ctx, ctx.rel_path, lang, nodes, edges);
        }
        "class_declaration" | "abstract_class_declaration" => {
            visit_class(node, ctx, lang, nodes, edges);
        }
        "export_statement" => {
            visit_export(node, ctx, lang, nodes, edges);
        }
        "import_statement" => {
            visit_import(node, ctx, lang, nodes, edges);
        }
        // TypeScript-specific top-level declarations.
        "interface_declaration" => {
            visit_ts_interface(node, ctx, lang, nodes, edges);
        }
        "type_alias_declaration" => {
            visit_ts_type_alias(node, ctx, lang, nodes, edges);
        }
        "enum_declaration" => {
            visit_ts_enum(node, ctx, lang, nodes, edges);
        }
        _ => {}
    }
}

fn visit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    lang: &str,
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

    let qn = format!("{}::fn::{}", ctx.rel_path, name);
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, ctx.source).to_owned());
    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| node_text(n, ctx.source).to_owned());

    nodes.push(Node {
        id: 0,
        kind: NodeKind::Function,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: lang.to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params,
        return_type,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, start_line(node)));
}

fn visit_class(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
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
        language: lang.to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, start_line(node)));

    // Walk class body for method definitions.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "method_definition" {
                visit_method(child, ctx, &qn, lang, nodes, edges);
            }
        }
    }
}

fn visit_method(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    class_qn: &str,
    lang: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, ctx.source);

    // Skip computed property names ([Symbol.iterator]) and accessor keywords.
    if name.starts_with('[') {
        return;
    }

    let class_name = class_qn.split("::").last().unwrap_or("Unknown");
    let qn = format!("{}::method::{}.{}", ctx.rel_path, class_name, name);
    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, ctx.source).to_owned());

    nodes.push(Node {
        id: 0,
        kind: NodeKind::Method,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: lang.to_owned(),
        parent_name: Some(class_qn.to_owned()),
        params,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(class_qn, &qn, ctx.rel_path, start_line(node)));
}

fn visit_export(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    if let Some(decl) = node.child_by_field_name("declaration") {
        match decl.kind() {
            "function_declaration" | "generator_function_declaration" => {
                visit_function(decl, ctx, ctx.rel_path, lang, nodes, edges);
            }
            "class_declaration" | "abstract_class_declaration" => {
                visit_class(decl, ctx, lang, nodes, edges);
            }
            "interface_declaration" => {
                visit_ts_interface(decl, ctx, lang, nodes, edges);
            }
            "type_alias_declaration" => {
                visit_ts_type_alias(decl, ctx, lang, nodes, edges);
            }
            "enum_declaration" => {
                visit_ts_enum(decl, ctx, lang, nodes, edges);
            }
            _ => {}
        }
    }
}

fn visit_import(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let source = node
        .child_by_field_name("source")
        .map(|n| {
            node_text(n, ctx.source)
                .trim_matches('"')
                .trim_matches('\'')
                .trim_matches('`')
                .to_owned()
        })
        .unwrap_or_default();
    if source.is_empty() {
        return;
    }

    let qn = format!("{}::import::{}", ctx.rel_path, source);
    nodes.push(Node {
        id: 0,
        kind: NodeKind::Import,
        name: source.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: lang.to_owned(),
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
        line: Some(start_line(node)),
        confidence: 1.0,
        confidence_tier: Some("definite".to_owned()),
        extra_json: serde_json::Value::Null,
    });
}

fn visit_ts_interface(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
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

    let qn = format!("{}::interface::{}", ctx.rel_path, name);
    nodes.push(Node {
        id: 0,
        kind: NodeKind::Interface,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: lang.to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, start_line(node)));
}

fn visit_ts_type_alias(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
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

    let qn = format!("{}::type::{}", ctx.rel_path, name);
    nodes.push(Node {
        id: 0,
        kind: NodeKind::Variable,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: lang.to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, start_line(node)));
}

fn visit_ts_enum(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
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

    let qn = format!("{}::enum::{}", ctx.rel_path, name);
    nodes.push(Node {
        id: 0,
        kind: NodeKind::Enum,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: lang.to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, start_line(node)));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ParseContext;

    fn parse_js(src: &str) -> ParsedFile {
        JsParser.parse(&ParseContext {
            rel_path: "src/app.js",
            file_hash: "cafebabe",
            source: src.as_bytes(),
        })
    }

    fn parse_ts(src: &str) -> ParsedFile {
        TsParser.parse(&ParseContext {
            rel_path: "src/app.ts",
            file_hash: "cafebabe",
            source: src.as_bytes(),
        })
    }

    #[test]
    fn js_file_node_present() {
        let pf = parse_js("const x = 1;\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
        assert_eq!(pf.language.as_deref(), Some("javascript"));
    }

    #[test]
    fn js_extracts_function() {
        let pf = parse_js("function greet(name) { return name; }\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Function && n.name == "greet"));
    }

    #[test]
    fn js_extracts_class() {
        let pf = parse_js("class Greeter { }\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Class && n.name == "Greeter"));
    }

    #[test]
    fn js_extracts_method() {
        let pf = parse_js("class Greeter { greet(name) { return name; } }\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Method && n.name == "greet"));
    }

    #[test]
    fn js_export_function() {
        let pf = parse_js("export function add(a, b) { return a + b; }\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Function && n.name == "add"));
    }

    #[test]
    fn js_import_edges() {
        let pf = parse_js("import { foo } from './utils';\n");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Import));
    }

    #[test]
    fn ts_file_node_present() {
        let pf = parse_ts("const x: number = 1;\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
        assert_eq!(pf.language.as_deref(), Some("typescript"));
    }

    #[test]
    fn ts_extracts_interface() {
        let pf = parse_ts("interface Greeter { greet(name: string): void; }\n");
        assert!(
            pf.nodes.iter().any(|n| n.kind == NodeKind::Interface && n.name == "Greeter"),
            "interface node not found"
        );
    }

    #[test]
    fn ts_extracts_enum() {
        let pf = parse_ts("enum Color { Red, Green, Blue }\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Enum && n.name == "Color"));
    }

    #[test]
    fn ts_type_alias() {
        let pf = parse_ts("type UserId = string;\n");
        assert!(pf.nodes.iter().any(|n| n.name == "UserId"));
    }

    #[test]
    fn ts_class_with_methods() {
        let pf = parse_ts(
            "class Service {\n  constructor(private db: DB) {}\n  fetch(id: string): Promise<Data> { return this.db.get(id); }\n}\n",
        );
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Class && n.name == "Service"));
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Method && n.name == "fetch"));
    }

    #[test]
    fn ts_export_class() {
        let pf = parse_ts("export class Router { route() {} }\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Class && n.name == "Router"));
    }

    #[test]
    fn d_ts_not_supported() {
        assert!(!TsParser.supports("index.d.ts"));
    }
}
