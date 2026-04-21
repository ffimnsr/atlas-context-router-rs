use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use std::collections::HashMap;
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

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
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

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
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
) -> (ParsedFile, Option<tree_sitter::Tree>) {
    let tree = parser.parse(ctx.source, ctx.old_tree);
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();

    let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
    nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count, lang));

    if let Some(ref tree) = tree {
        let root = tree.root_node();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            visit_toplevel(child, ctx, lang, &mut nodes, &mut edges);
        }

        // Second pass: same-file call resolution.
        let mut call_edges = resolve_js_calls(root, ctx.source, ctx.rel_path, &nodes);
        edges.append(&mut call_edges);
    }

    let pf = ParsedFile {
        path: ctx.rel_path.to_owned(),
        language: Some(lang.to_owned()),
        hash: ctx.file_hash.to_owned(),
        size: Some(ctx.source.len() as i64),
        nodes,
        edges,
    };
    (pf, tree)
}

// ---------------------------------------------------------------------------
// Node constructors
// ---------------------------------------------------------------------------

fn file_node(rel_path: &str, file_hash: &str, line_end: u32, lang: &str) -> Node {
    Node {
        id: NodeId::UNSET,
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
        "lexical_declaration" | "variable_declaration" => {
            visit_variable_declaration(node, ctx, lang, nodes, edges);
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
        id: NodeId::UNSET,
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
        id: NodeId::UNSET,
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
    edges.push(contains_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));

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
        id: NodeId::UNSET,
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
            "lexical_declaration" | "variable_declaration" => {
                visit_variable_declaration(decl, ctx, lang, nodes, edges);
            }
            _ => {}
        }
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "lexical_declaration" | "variable_declaration") {
                visit_variable_declaration(child, ctx, lang, nodes, edges);
            }
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
    let bindings = parse_js_import_bindings(node_text(node, ctx.source));
    nodes.push(Node {
        id: NodeId::UNSET,
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
        extra_json: serde_json::json!({
            "source": source,
            "bindings": bindings,
        }),
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
        id: NodeId::UNSET,
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
    edges.push(contains_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
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
        id: NodeId::UNSET,
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
    edges.push(contains_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
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
        id: NodeId::UNSET,
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
    edges.push(contains_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

fn visit_variable_declaration(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            visit_variable_declarator(child, ctx, lang, nodes, edges);
        }
    }
}

fn visit_variable_declarator(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, ctx.source);
    if name.is_empty() || !is_identifier_like(name) {
        return;
    }
    let Some(value) = node.child_by_field_name("value") else {
        return;
    };
    let Some(function_node) = function_value_node(value, ctx.source) else {
        return;
    };

    let qn = format!("{}::fn::{}", ctx.rel_path, name);
    let params = function_node
        .child_by_field_name("parameters")
        .map(|n| node_text(n, ctx.source).to_owned());
    let return_type = function_node
        .child_by_field_name("return_type")
        .map(|n| node_text(n, ctx.source).to_owned());

    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: name.to_owned(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: start_line(node),
        line_end: end_line(node),
        language: lang.to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params,
        return_type,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    });
    edges.push(contains_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

fn function_value_node<'a>(node: TsNode<'a>, source: &[u8]) -> Option<TsNode<'a>> {
    match node.kind() {
        "function" | "function_expression" | "arrow_function" => Some(node),
        "call_expression" => {
            let function = node.child_by_field_name("function")?;
            if !matches!(
                node_text(function, source),
                "memo" | "forwardRef" | "React.memo"
            ) {
                return None;
            }
            first_function_argument(node)
        }
        _ => None,
    }
}

fn first_function_argument(node: TsNode<'_>) -> Option<TsNode<'_>> {
    let args = node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    args.children(&mut cursor).find(|child| {
        matches!(
            child.kind(),
            "function" | "function_expression" | "arrow_function"
        )
    })
}

fn is_identifier_like(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c == '_' || c == '$' || c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
}

// ---------------------------------------------------------------------------
// Same-file call resolution (JavaScript / TypeScript)
// ---------------------------------------------------------------------------

fn import_binding_json(local: &str, imported: Option<&str>, kind: &str) -> serde_json::Value {
    serde_json::json!({
        "local": local,
        "imported": imported,
        "kind": kind,
    })
}

fn parse_js_import_bindings(statement: &str) -> Vec<serde_json::Value> {
    let trimmed = statement.trim().trim_end_matches(';').trim();
    let Some(rest) = trimmed.strip_prefix("import") else {
        return Vec::new();
    };
    let rest = rest.trim();
    if rest.starts_with('"') || rest.starts_with('\'') || rest.starts_with('`') {
        return Vec::new();
    }
    let clause = rest
        .split_once(" from ")
        .map(|(head, _)| head)
        .unwrap_or(rest);
    let mut bindings = Vec::new();
    let mut remaining = clause.trim();

    if let Some((default_part, tail)) = remaining.split_once(',') {
        let default_local = default_part.trim();
        if !default_local.is_empty() {
            bindings.push(import_binding_json(
                default_local,
                Some("default"),
                "default",
            ));
        }
        remaining = tail.trim();
    }

    if remaining.starts_with("* as ") {
        let local = remaining.trim_start_matches("* as ").trim();
        if !local.is_empty() {
            bindings.push(import_binding_json(local, None, "namespace"));
        }
        return bindings;
    }

    if remaining.starts_with('{') && remaining.ends_with('}') {
        let inner = &remaining[1..remaining.len().saturating_sub(1)];
        for part in inner.split(',') {
            let entry = part.trim();
            if entry.is_empty() {
                continue;
            }
            let (imported, local) = entry
                .split_once(" as ")
                .map(|(imported, local)| (imported.trim(), local.trim()))
                .unwrap_or((entry, entry));
            bindings.push(import_binding_json(local, Some(imported), "named"));
        }
        return bindings;
    }

    if !remaining.is_empty() {
        bindings.push(import_binding_json(remaining, Some("default"), "default"));
    }

    bindings
}

fn resolve_js_calls(root: TsNode<'_>, source: &[u8], rel_path: &str, nodes: &[Node]) -> Vec<Edge> {
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
    walk_js_calls(root, source, rel_path, &callables, &mut scope, &mut edges);
    edges
}

fn walk_js_calls<'a>(
    node: TsNode<'a>,
    source: &[u8],
    rel_path: &str,
    callables: &HashMap<String, String>,
    scope: &mut Vec<String>,
    edges: &mut Vec<Edge>,
) {
    let kind = node.kind();
    let is_function_scope = matches!(
        kind,
        "function_declaration"
            | "function"
            | "function_expression"
            | "arrow_function"
            | "method_definition"
            | "function_signature" // TS
            | "method_signature" // TS
    );

    if is_function_scope {
        // Try to find the function name.
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
            walk_js_calls(child, source, rel_path, callables, scope, edges);
        }
        if pushed {
            scope.pop();
        }
        return;
    }

    if kind == "variable_declarator"
        && let (Some(name_node), Some(value_node)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        )
        && matches!(
            value_node.kind(),
            "function" | "function_expression" | "arrow_function" | "call_expression"
        )
    {
        let name = node_text(name_node, source);
        let pushed = if let Some(qn) = callables.get(name) {
            scope.push(qn.clone());
            true
        } else {
            false
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk_js_calls(child, source, rel_path, callables, scope, edges);
        }
        if pushed {
            scope.pop();
        }
        return;
    }

    if kind == "call_expression"
        && let Some(caller_qn) = scope.last().cloned()
    {
        let called = node
            .child_by_field_name("function")
            .and_then(|f| js_call_target(f, source));
        if let Some((text, name, receiver)) = called
            && !is_self_call(&caller_qn, &name, receiver.as_deref())
        {
            if let Some(callee_qn) = callables.get(&name)
                && *callee_qn != caller_qn
            {
                edges.push(js_call_edge(
                    &caller_qn,
                    callee_qn,
                    rel_path,
                    start_line(node),
                    &text,
                    receiver.as_deref(),
                    true,
                ));
            } else {
                edges.push(js_call_edge(
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

    if matches!(kind, "jsx_opening_element" | "jsx_self_closing_element")
        && let Some(caller_qn) = scope.last().cloned()
        && let Some((text, name, receiver)) = jsx_call_target(node, source)
    {
        if let Some(callee_qn) = callables.get(&name)
            && *callee_qn != caller_qn
        {
            edges.push(js_call_edge(
                &caller_qn,
                callee_qn,
                rel_path,
                start_line(node),
                &text,
                receiver.as_deref(),
                true,
            ));
        } else {
            edges.push(js_call_edge(
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

    // Default recursive walk.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_js_calls(child, source, rel_path, callables, scope, edges);
    }
}

fn js_call_target(node: TsNode<'_>, source: &[u8]) -> Option<(String, String, Option<String>)> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source).to_owned();
            Some((name.clone(), name, None))
        }
        "member_expression" => {
            let property = node.child_by_field_name("property")?;
            let object = node.child_by_field_name("object")?;
            let callee_name = node_text(property, source).to_owned();
            let receiver_text = node_text(object, source).to_owned();
            Some((
                node_text(node, source).to_owned(),
                callee_name,
                Some(receiver_text),
            ))
        }
        _ => None,
    }
}

fn jsx_call_target(node: TsNode<'_>, source: &[u8]) -> Option<(String, String, Option<String>)> {
    let name_node = node.child_by_field_name("name").or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|child| {
            matches!(
                child.kind(),
                "identifier" | "nested_identifier" | "member_expression" | "jsx_identifier"
            )
        })
    })?;
    let text = node_text(name_node, source).to_owned();
    if text.is_empty() || text.as_bytes().first().is_some_and(u8::is_ascii_lowercase) {
        return None;
    }
    if let Some((receiver, name)) = text.rsplit_once('.') {
        return Some((text.clone(), name.to_owned(), Some(receiver.to_owned())));
    }
    Some((text.clone(), text, None))
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

fn js_call_edge(
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

    fn parse_js(src: &str) -> ParsedFile {
        let (pf, _) = JsParser.parse(&ParseContext {
            rel_path: "src/app.js",
            file_hash: "cafebabe",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    fn parse_ts(src: &str) -> ParsedFile {
        let (pf, _) = TsParser.parse(&ParseContext {
            rel_path: "src/app.ts",
            file_hash: "cafebabe",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    fn parse_tsx(src: &str) -> ParsedFile {
        let (pf, _) = TsParser.parse(&ParseContext {
            rel_path: "src/app.tsx",
            file_hash: "cafebabe",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
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
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "greet")
        );
    }

    #[test]
    fn js_extracts_class() {
        let pf = parse_js("class Greeter { }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Class && n.name == "Greeter")
        );
    }

    #[test]
    fn js_extracts_method() {
        let pf = parse_js("class Greeter { greet(name) { return name; } }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Method && n.name == "greet")
        );
    }

    #[test]
    fn js_export_function() {
        let pf = parse_js("export function add(a, b) { return a + b; }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "add")
        );
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
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Interface && n.name == "Greeter"),
            "interface node not found"
        );
    }

    #[test]
    fn ts_extracts_enum() {
        let pf = parse_ts("enum Color { Red, Green, Blue }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Enum && n.name == "Color")
        );
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
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Class && n.name == "Service")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Method && n.name == "fetch")
        );
    }

    #[test]
    fn ts_export_class() {
        let pf = parse_ts("export class Router { route() {} }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Class && n.name == "Router")
        );
    }

    #[test]
    fn d_ts_not_supported() {
        assert!(!TsParser.supports("index.d.ts"));
    }

    #[test]
    fn js_same_file_call_resolved() {
        let src = "function helper() {}\nfunction caller() { helper(); }\n";
        let pf = parse_js(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected Calls edge from caller to helper; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn ts_same_file_call_resolved() {
        let src = "function helper(): void {}\nfunction caller(): void { helper(); }\n";
        let pf = parse_ts(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected Calls edge from caller to helper in TS"
        );
    }

    #[test]
    fn js_unresolved_call_keeps_text_target() {
        let src = "function caller() { utils.helper(); }\n";
        let pf = parse_js(src);
        let edge = pf
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Calls)
            .expect("call edge");
        assert_eq!(edge.target_qn, "utils.helper");
        assert_eq!(edge.confidence_tier.as_deref(), Some("text"));
    }

    #[test]
    fn tsx_const_function_component_is_callable_scope() {
        let src = "const App = () => <Widget />;\n";
        let pf = parse_tsx(src);
        assert!(pf.nodes.iter().any(|n| {
            n.kind == NodeKind::Function
                && n.name == "App"
                && n.qualified_name == "src/app.tsx::fn::App"
        }));
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "src/app.tsx::fn::App"
                    && e.target_qn == "Widget"
                    && e.confidence_tier.as_deref() == Some("text")
            }),
            "expected JSX component call from App to Widget; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn tsx_memo_named_function_component_calls_imported_jsx_component() {
        let src = "export const HistoryTab = memo(function HistoryTab() { return <SideFilterControl />; });\n";
        let pf = parse_tsx(src);
        assert!(
            pf.nodes.iter().any(|n| {
                n.kind == NodeKind::Function
                    && n.name == "HistoryTab"
                    && n.qualified_name == "src/app.tsx::fn::HistoryTab"
            }),
            "expected HistoryTab function node; nodes: {:?}",
            pf.nodes
        );
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "src/app.tsx::fn::HistoryTab"
                    && e.target_qn == "SideFilterControl"
                    && e.confidence_tier.as_deref() == Some("text")
            }),
            "expected JSX component call from HistoryTab to SideFilterControl; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn tsx_ignores_lowercase_jsx_elements() {
        let src = "const App = () => <div><Widget /></div>;\n";
        let pf = parse_tsx(src);
        assert!(
            !pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.target_qn == "div"
                    && e.confidence_tier.as_deref() == Some("text")
            }),
            "lowercase intrinsic JSX element should not produce call edge: {:?}",
            pf.edges
        );
    }
}
