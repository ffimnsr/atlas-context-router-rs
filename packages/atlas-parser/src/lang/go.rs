use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use std::collections::{HashMap, HashSet};
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

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
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
                    "const_declaration" => {
                        visit_value_decl(
                            child,
                            ctx,
                            &package_qn,
                            NodeKind::Constant,
                            "const",
                            &mut nodes,
                            &mut edges,
                        );
                    }
                    "var_declaration" => {
                        visit_value_decl(
                            child,
                            ctx,
                            &package_qn,
                            NodeKind::Variable,
                            "var",
                            &mut nodes,
                            &mut edges,
                        );
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
    let (receiver_name, receiver_type) = method_receiver(node, ctx.source);

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
        extra_json: serde_json::json!({
            "receiver_name": receiver_name,
            "receiver_type": receiver_type,
        }),
    });
    edges.push(contains_edge(
        package_qn,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

fn method_receiver(node: TsNode<'_>, source: &[u8]) -> (Option<String>, String) {
    let Some(receiver) = node.child_by_field_name("receiver") else {
        return (None, String::new());
    };
    let receiver_text = node_text(receiver, source)
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')');
    let mut parts = receiver_text.split_whitespace();
    let receiver_name = parts
        .next()
        .map(|part| part.trim_start_matches('*').to_owned());
    let mut receiver_type = parts
        .next()
        .map(normalize_receiver_type)
        .unwrap_or_default();

    if receiver_type.is_empty() {
        receiver_type = receiver
            .child_by_field_name("type")
            .or_else(|| {
                find_descendant_kind(
                    receiver,
                    &[
                        "type_identifier",
                        "qualified_type",
                        "pointer_type",
                        "generic_type",
                    ],
                )
            })
            .map(|type_node| normalize_receiver_type(node_text(type_node, source)))
            .unwrap_or_default();
    }
    (receiver_name, receiver_type)
}

fn normalize_receiver_type(raw: &str) -> String {
    let no_pointer = raw.trim_start_matches('*');
    no_pointer
        .split(['[', '{'])
        .next()
        .unwrap_or(no_pointer)
        .trim()
        .to_owned()
}

fn find_descendant_kind<'a>(node: TsNode<'a>, kinds: &[&str]) -> Option<TsNode<'a>> {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if kinds.contains(&current.kind()) {
            return Some(current);
        }
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            stack.push(child);
        }
    }
    None
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

fn visit_value_decl(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    package_qn: &str,
    kind: NodeKind,
    qn_prefix: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "const_spec" && child.kind() != "var_spec" {
            continue;
        }
        for ident in spec_identifiers(child, ctx.source) {
            let qn = format!("{}::{}::{}", ctx.rel_path, qn_prefix, ident);
            nodes.push(Node {
                id: NodeId::UNSET,
                kind,
                name: ident.clone(),
                qualified_name: qn.clone(),
                file_path: ctx.rel_path.to_owned(),
                line_start: start_line(child),
                line_end: end_line(child),
                language: "go".to_owned(),
                parent_name: Some(package_qn.to_owned()),
                params: None,
                return_type: child
                    .child_by_field_name("type")
                    .map(|type_node| node_text(type_node, ctx.source).to_owned()),
                modifiers: None,
                is_test: false,
                file_hash: ctx.file_hash.to_owned(),
                extra_json: serde_json::Value::Null,
            });
            edges.push(contains_edge(
                package_qn,
                &qn,
                ctx.rel_path,
                start_line(child),
            ));
        }
    }
}

fn spec_identifiers(spec: TsNode<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = spec.walk();
    for child in spec.children(&mut cursor) {
        if child.kind() == "identifier" {
            names.push(node_text(child, source).to_owned());
        }
        if child.kind() == "identifier_list" {
            let mut inner = child.walk();
            for item in child.children(&mut inner) {
                if item.kind() == "identifier" {
                    names.push(node_text(item, source).to_owned());
                }
            }
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
// Same-file call resolution (Go)
// ---------------------------------------------------------------------------

fn resolve_go_calls(root: TsNode<'_>, source: &[u8], rel_path: &str, nodes: &[Node]) -> Vec<Edge> {
    let mut functions: HashMap<String, String> = HashMap::new();
    let mut methods: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut return_types: HashMap<String, String> = HashMap::new();
    let struct_types = collect_struct_types(root, source);
    for n in nodes {
        match n.kind {
            NodeKind::Function | NodeKind::Test => {
                functions.insert(n.name.clone(), n.qualified_name.clone());
            }
            NodeKind::Method => {
                if let Some(receiver_type) = method_receiver_from_qn(&n.qualified_name) {
                    methods
                        .entry(n.name.clone())
                        .or_default()
                        .push((receiver_type.to_owned(), n.qualified_name.clone()));
                }
            }
            _ => {}
        }
        if matches!(n.kind, NodeKind::Function | NodeKind::Method)
            && let Some(return_type) = n
                .return_type
                .as_deref()
                .and_then(extract_callable_return_type)
        {
            return_types.insert(n.qualified_name.clone(), return_type);
        }
    }
    let mut edges = Vec::new();
    let mut scope: Vec<CallableScope> = Vec::new();
    walk_go_calls(
        root,
        source,
        rel_path,
        &functions,
        &methods,
        &struct_types,
        &return_types,
        &mut scope,
        &mut edges,
    );
    edges
}

#[derive(Clone, Debug, Default)]
struct StructTypeInfo {
    fields: HashMap<String, String>,
    embedded_types: Vec<String>,
}

fn collect_struct_types(root: TsNode<'_>, source: &[u8]) -> HashMap<String, StructTypeInfo> {
    let mut struct_types = HashMap::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "type_declaration" {
            continue;
        }
        let mut type_cursor = child.walk();
        for type_spec in child.children(&mut type_cursor) {
            if type_spec.kind() != "type_spec" {
                continue;
            }
            let Some(type_name) = field_text(type_spec, "name", source) else {
                continue;
            };
            let Some(type_node) = type_spec.child_by_field_name("type") else {
                continue;
            };
            if type_node.kind() != "struct_type" {
                continue;
            }
            let info = collect_struct_type_info(type_node, source);
            if !info.fields.is_empty() || !info.embedded_types.is_empty() {
                struct_types.insert(type_name.to_owned(), info);
            }
        }
    }
    struct_types
}

fn collect_struct_type_info(struct_type: TsNode<'_>, source: &[u8]) -> StructTypeInfo {
    let mut info = StructTypeInfo::default();
    let Some(field_list) = find_descendant_kind(struct_type, &["field_declaration_list"]) else {
        return info;
    };
    let mut cursor = field_list.walk();
    for field in field_list.children(&mut cursor) {
        if field.kind() != "field_declaration" {
            continue;
        }
        let Some(type_node) = field.child_by_field_name("type") else {
            continue;
        };
        let field_type = normalize_receiver_type(node_text(type_node, source));
        if field_type.is_empty() {
            continue;
        }
        let names = field_names(field, source);
        if names.is_empty() {
            if let Some(embedded_name) = embedded_field_name(type_node, source) {
                info.fields.insert(embedded_name, field_type.clone());
                info.embedded_types.push(field_type);
            }
            continue;
        }
        for name in names {
            info.fields.insert(name, field_type.clone());
        }
    }
    info
}

fn field_names(field: TsNode<'_>, source: &[u8]) -> Vec<String> {
    let Some(type_node) = field.child_by_field_name("type") else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut cursor = field.walk();
    for child in field.children(&mut cursor) {
        if !child.is_named() || child.start_byte() >= type_node.start_byte() {
            break;
        }
        match child.kind() {
            "identifier" | "field_identifier" => names.push(node_text(child, source).to_owned()),
            "identifier_list" => {
                let mut inner = child.walk();
                for item in child.children(&mut inner) {
                    if item.kind() == "identifier" || item.kind() == "field_identifier" {
                        names.push(node_text(item, source).to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    names
}

fn embedded_field_name(type_node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let normalized = normalize_receiver_type(node_text(type_node, source));
    let name = normalized.rsplit('.').next().unwrap_or(&normalized).trim();
    (!name.is_empty()).then_some(name.to_owned())
}

#[derive(Clone, Debug)]
struct CallableScope {
    qn: String,
    receiver_type: Option<String>,
    local_types: HashMap<String, String>,
}

#[allow(clippy::too_many_arguments)]
fn walk_go_calls<'a>(
    node: TsNode<'a>,
    source: &[u8],
    rel_path: &str,
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    scope: &mut Vec<CallableScope>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "function_declaration" | "method_declaration" => {
            let pushed = if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                if let Some(qn) = functions.get(name).cloned().or_else(|| {
                    methods.get(name).and_then(|candidates| {
                        if candidates.len() == 1 {
                            Some(candidates[0].1.clone())
                        } else {
                            None
                        }
                    })
                }) {
                    let (receiver_name, receiver_type) = if node.kind() == "method_declaration" {
                        method_receiver(node, source)
                    } else {
                        (None, String::new())
                    };
                    let mut local_types = node
                        .child_by_field_name("parameters")
                        .map(|parameters| collect_parameter_types(parameters, source))
                        .unwrap_or_default();
                    if let (Some(receiver_name), false) =
                        (receiver_name.as_ref(), receiver_type.is_empty())
                    {
                        local_types.insert(receiver_name.clone(), receiver_type.clone());
                    }
                    scope.push(CallableScope {
                        qn,
                        receiver_type: (!receiver_type.is_empty()).then_some(receiver_type),
                        local_types,
                    });
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_go_calls(
                    child,
                    source,
                    rel_path,
                    functions,
                    methods,
                    struct_types,
                    return_types,
                    scope,
                    edges,
                );
            }
            if pushed {
                scope.pop();
            }
            return;
        }
        "var_declaration" | "short_var_declaration" | "assignment_statement" => {
            if let Some(caller_scope) = scope.last_mut() {
                record_local_types(
                    node,
                    source,
                    functions,
                    methods,
                    struct_types,
                    return_types,
                    caller_scope,
                );
            }
        }
        "call_expression" => {
            if let Some(caller_scope) = scope.last().cloned() {
                // In Go, call_expression.function can be identifier or selector_expression.
                let function = node.child_by_field_name("function");
                let called = function.and_then(|f| go_call_target(f, source));
                if let Some((text, name, receiver)) = called {
                    let receiver_type = function.and_then(|function_node| {
                        selector_receiver_type(
                            function_node,
                            source,
                            functions,
                            methods,
                            struct_types,
                            return_types,
                            &caller_scope,
                        )
                    });
                    if let Some(callee_qn) = resolve_callable(
                        &caller_scope,
                        &name,
                        receiver.as_deref(),
                        receiver_type.as_deref(),
                        functions,
                        methods,
                        struct_types,
                    ) {
                        if callee_qn == caller_scope.qn {
                            return;
                        }
                        edges.push(go_call_edge(
                            &caller_scope.qn,
                            callee_qn,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            None,
                            true,
                        ));
                    } else {
                        edges.push(go_call_edge(
                            &caller_scope.qn,
                            &text,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            receiver_type.as_deref(),
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
        walk_go_calls(
            child,
            source,
            rel_path,
            functions,
            methods,
            struct_types,
            return_types,
            scope,
            edges,
        );
    }
}

fn resolve_callable<'a>(
    caller_scope: &CallableScope,
    callee_name: &str,
    receiver: Option<&str>,
    receiver_type: Option<&str>,
    functions: &'a HashMap<String, String>,
    methods: &'a HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
) -> Option<&'a str> {
    if let Some(receiver_text) = receiver {
        let resolved_receiver_type = receiver_type.or_else(|| {
            receiver_binding_name(receiver_text)
                .and_then(|receiver_key| caller_scope.local_types.get(receiver_key))
                .map(String::as_str)
        });
        if let Some(receiver_type) = resolved_receiver_type
            && let Some(qn) =
                resolve_method_candidate(callee_name, receiver_type, methods, struct_types)
        {
            return Some(qn);
        }
        return None;
    }

    if let Some(receiver_type) = caller_scope.receiver_type.as_deref()
        && let Some(qn) =
            resolve_method_candidate(callee_name, receiver_type, methods, struct_types)
    {
        return Some(qn);
    }

    functions.get(callee_name).map(String::as_str)
}

fn resolve_method_candidate<'a>(
    callee_name: &str,
    receiver_type: &str,
    methods: &'a HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
) -> Option<&'a str> {
    let candidates = methods.get(callee_name)?;
    if let Some((_, qn)) = candidates.iter().find(|(ty, _)| ty == receiver_type) {
        return Some(qn.as_str());
    }

    let promoted_types = promoted_receiver_types(receiver_type, struct_types);
    if promoted_types.is_empty() {
        return None;
    }

    let mut matches = candidates
        .iter()
        .filter(|(ty, _)| promoted_types.contains(ty))
        .map(|(_, qn)| qn.as_str());
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn promoted_receiver_types(
    receiver_type: &str,
    struct_types: &HashMap<String, StructTypeInfo>,
) -> HashSet<String> {
    let mut promoted = HashSet::new();
    let mut visited = HashSet::new();
    collect_promoted_receiver_types(receiver_type, struct_types, &mut visited, &mut promoted);
    promoted
}

fn collect_promoted_receiver_types(
    receiver_type: &str,
    struct_types: &HashMap<String, StructTypeInfo>,
    visited: &mut HashSet<String>,
    promoted: &mut HashSet<String>,
) {
    if !visited.insert(receiver_type.to_owned()) {
        return;
    }
    let Some(info) = struct_types.get(receiver_type) else {
        return;
    };
    for embedded_type in &info.embedded_types {
        if promoted.insert(embedded_type.clone()) {
            collect_promoted_receiver_types(embedded_type, struct_types, visited, promoted);
        }
    }
}

fn collect_parameter_types(parameter_list: TsNode<'_>, source: &[u8]) -> HashMap<String, String> {
    let mut local_types = HashMap::new();
    let mut cursor = parameter_list.walk();
    for child in parameter_list.children(&mut cursor) {
        if child.kind() != "parameter_declaration"
            && child.kind() != "variadic_parameter_declaration"
        {
            continue;
        }
        let Some(type_node) = child.child_by_field_name("type") else {
            continue;
        };
        let type_name = normalize_receiver_type(node_text(type_node, source));
        if type_name.is_empty() {
            continue;
        }
        for name in binding_identifiers(child, source) {
            local_types.insert(name, type_name.clone());
        }
    }
    local_types
}

fn record_local_types(
    node: TsNode<'_>,
    source: &[u8],
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    caller_scope: &mut CallableScope,
) {
    match node.kind() {
        "var_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "var_spec" {
                    record_var_spec_types(
                        child,
                        source,
                        functions,
                        methods,
                        struct_types,
                        return_types,
                        caller_scope,
                    );
                }
            }
        }
        "short_var_declaration" => record_assignment_like_types(
            node,
            source,
            functions,
            methods,
            struct_types,
            return_types,
            caller_scope,
        ),
        "assignment_statement" => {
            let operator = node
                .child_by_field_name("operator")
                .map(|operator| node_text(operator, source));
            if operator == Some("=") {
                record_assignment_like_types(
                    node,
                    source,
                    functions,
                    methods,
                    struct_types,
                    return_types,
                    caller_scope,
                );
            }
        }
        _ => {}
    }
}

fn record_var_spec_types(
    spec: TsNode<'_>,
    source: &[u8],
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    caller_scope: &mut CallableScope,
) {
    let names = binding_identifiers(spec, source);
    if names.is_empty() {
        return;
    }

    if let Some(type_node) = spec.child_by_field_name("type") {
        let type_name = normalize_receiver_type(node_text(type_node, source));
        if !type_name.is_empty() {
            for name in names {
                caller_scope.local_types.insert(name, type_name.clone());
            }
            return;
        }
    }

    if let Some(values) = spec.child_by_field_name("value") {
        bind_expression_list_types(
            names,
            values,
            source,
            functions,
            methods,
            struct_types,
            return_types,
            caller_scope,
        );
    }
}

fn record_assignment_like_types(
    node: TsNode<'_>,
    source: &[u8],
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    caller_scope: &mut CallableScope,
) {
    let Some(left) = node.child_by_field_name("left") else {
        return;
    };
    let Some(right) = node.child_by_field_name("right") else {
        return;
    };
    let names = binding_identifiers(left, source);
    if names.is_empty() {
        return;
    }
    bind_expression_list_types(
        names,
        right,
        source,
        functions,
        methods,
        struct_types,
        return_types,
        caller_scope,
    );
}

#[allow(clippy::too_many_arguments)]
fn bind_expression_list_types(
    names: Vec<String>,
    values: TsNode<'_>,
    source: &[u8],
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    caller_scope: &mut CallableScope,
) {
    let expressions = expression_list_items(values);
    for (name, expr) in names.into_iter().zip(expressions) {
        if let Some(type_name) = infer_expression_type(
            expr,
            source,
            functions,
            methods,
            struct_types,
            return_types,
            caller_scope,
        ) {
            caller_scope.local_types.insert(name, type_name);
        }
    }
}

fn infer_expression_type(
    expr: TsNode<'_>,
    source: &[u8],
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    caller_scope: &CallableScope,
) -> Option<String> {
    match expr.kind() {
        "identifier" => caller_scope
            .local_types
            .get(node_text(expr, source))
            .cloned(),
        "parenthesized_expression" => {
            let mut cursor = expr.walk();
            expr.children(&mut cursor)
                .find(|child| child.is_named())
                .and_then(|child| {
                    infer_expression_type(
                        child,
                        source,
                        functions,
                        methods,
                        struct_types,
                        return_types,
                        caller_scope,
                    )
                })
        }
        "unary_expression" => expr.child_by_field_name("operand").and_then(|operand| {
            infer_expression_type(
                operand,
                source,
                functions,
                methods,
                struct_types,
                return_types,
                caller_scope,
            )
        }),
        "selector_expression" => infer_selector_expression_type(
            expr,
            source,
            functions,
            methods,
            struct_types,
            return_types,
            caller_scope,
        ),
        "composite_literal" | "type_conversion_expression" => expr
            .child_by_field_name("type")
            .map(|type_node| normalize_receiver_type(node_text(type_node, source)))
            .filter(|type_name| !type_name.is_empty()),
        "call_expression" => infer_call_expression_type(
            expr,
            source,
            functions,
            methods,
            struct_types,
            return_types,
            caller_scope,
        ),
        _ => None,
    }
}

fn infer_selector_expression_type(
    expr: TsNode<'_>,
    source: &[u8],
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    caller_scope: &CallableScope,
) -> Option<String> {
    let operand = expr.child_by_field_name("operand")?;
    let field = expr.child_by_field_name("field")?;
    let owner_type = infer_expression_type(
        operand,
        source,
        functions,
        methods,
        struct_types,
        return_types,
        caller_scope,
    )?;
    let field_name = node_text(field, source);
    struct_types
        .get(&owner_type)
        .and_then(|info| info.fields.get(field_name))
        .cloned()
}

fn infer_call_expression_type(
    expr: TsNode<'_>,
    source: &[u8],
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    caller_scope: &CallableScope,
) -> Option<String> {
    let function = expr.child_by_field_name("function")?;
    let called = go_call_target(function, source)?;
    let (_, callee_name, receiver) = called;
    let receiver_type = selector_receiver_type(
        function,
        source,
        functions,
        methods,
        struct_types,
        return_types,
        caller_scope,
    );
    resolve_callable(
        caller_scope,
        &callee_name,
        receiver.as_deref(),
        receiver_type.as_deref(),
        functions,
        methods,
        struct_types,
    )
    .and_then(|qualified_name| return_types.get(qualified_name).cloned())
}

fn selector_receiver_type(
    function: TsNode<'_>,
    source: &[u8],
    functions: &HashMap<String, String>,
    methods: &HashMap<String, Vec<(String, String)>>,
    struct_types: &HashMap<String, StructTypeInfo>,
    return_types: &HashMap<String, String>,
    caller_scope: &CallableScope,
) -> Option<String> {
    if function.kind() != "selector_expression" {
        return None;
    }
    let receiver = function.child_by_field_name("operand")?;
    infer_expression_type(
        receiver,
        source,
        functions,
        methods,
        struct_types,
        return_types,
        caller_scope,
    )
}

fn extract_callable_return_type(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = if trimmed.starts_with('(') && trimmed.ends_with(')') {
        let inner = &trimmed[1..trimmed.len() - 1];
        inner.split(',').next().map(str::trim).unwrap_or("")
    } else {
        trimmed
    };

    let candidate = candidate
        .split_whitespace()
        .last()
        .unwrap_or(candidate)
        .trim();
    let normalized = normalize_receiver_type(candidate);
    (!normalized.is_empty()).then_some(normalized)
}

fn binding_identifiers(node: TsNode<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            names.push(node_text(child, source).to_owned());
        }
    }
    names
}

fn expression_list_items(list: TsNode<'_>) -> Vec<TsNode<'_>> {
    let mut items = Vec::new();
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.is_named() {
            items.push(child);
        }
    }
    items
}

fn receiver_binding_name(receiver_text: &str) -> Option<&str> {
    let trimmed = receiver_text
        .trim()
        .trim_start_matches('*')
        .trim_start_matches('&')
        .trim_start_matches('(')
        .trim_end_matches(')');
    (!trimmed.is_empty()).then_some(trimmed)
}

fn method_receiver_from_qn(qn: &str) -> Option<&str> {
    let (_, method_part) = qn.split_once("::method::")?;
    let (receiver, _) = method_part.rsplit_once('.')?;
    (!receiver.is_empty()).then_some(receiver)
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

fn caller_simple_name(caller_qn: &str) -> &str {
    caller_qn
        .rsplit("::")
        .next()
        .unwrap_or(caller_qn)
        .rsplit('.')
        .next()
        .unwrap_or(caller_qn)
}

#[allow(clippy::too_many_arguments)]
fn go_call_edge(
    caller: &str,
    callee: &str,
    rel_path: &str,
    line: u32,
    text: &str,
    receiver: Option<&str>,
    receiver_type: Option<&str>,
    same_file: bool,
) -> Edge {
    let mut extra_json = serde_json::Map::new();
    extra_json.insert(
        "callee_text".to_owned(),
        serde_json::Value::String(text.to_owned()),
    );
    extra_json.insert(
        "callee_name".to_owned(),
        serde_json::Value::String(caller_simple_name(callee).to_owned()),
    );
    extra_json.insert(
        "receiver_text".to_owned(),
        receiver
            .map(|value| serde_json::Value::String(value.to_owned()))
            .unwrap_or(serde_json::Value::Null),
    );
    if let Some(receiver_type) = receiver_type {
        extra_json.insert(
            "receiver_type".to_owned(),
            serde_json::Value::String(receiver_type.to_owned()),
        );
    }
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: caller.to_owned(),
        target_qn: callee.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: if same_file { 0.8 } else { 0.3 },
        confidence_tier: Some(if same_file { "same_file" } else { "text" }.to_owned()),
        extra_json: serde_json::Value::Object(extra_json),
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
        let method = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Method && n.name == "Bar")
            .expect("method node");
        assert_eq!(method.qualified_name, "cmd/main.go::method::Foo.Bar");
        assert_eq!(
            method.parent_name.as_deref(),
            Some("cmd/main.go::package::main")
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

    #[test]
    fn extracts_const_and_var_nodes() {
        let src = "package main\nconst (\nA = 1\nB = 2\n)\nvar c string\n";
        let pf = parse(src);
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Constant && n.name == "A")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Constant && n.name == "B")
        );
        let var = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Variable && n.name == "c")
            .expect("variable node");
        assert_eq!(var.return_type.as_deref(), Some("string"));
        assert_eq!(
            var.parent_name.as_deref(),
            Some("cmd/main.go::package::main")
        );
    }

    #[test]
    fn resolves_method_call_on_receiver_scope() {
        let src = r#"
package main
type Foo struct{}
func (f *Foo) A() { f.B() }
func (f *Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::method::Foo.A"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_receiver_alias() {
        let src = r#"
package main
type Foo struct{}
func (f *Foo) A() { alias := f; alias.B() }
func (f *Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::method::Foo.A"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_typed_local_variable() {
        let src = r#"
package main
type Foo struct{}
func caller() { var local Foo; local.B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_function_return_local() {
        let src = r#"
package main
type Foo struct{}
func NewFoo() Foo { return Foo{} }
func caller() { local := NewFoo(); local.B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_method_return_local() {
        let src = r#"
package main
type Foo struct{}
func (f Foo) Clone() Foo { return f }
func caller(seed Foo) { local := seed.Clone(); local.B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_function_return_chain() {
        let src = r#"
package main
type Foo struct{}
func NewFoo() Foo { return Foo{} }
func caller() { NewFoo().B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_method_return_chain() {
        let src = r#"
package main
type Foo struct{}
func (f Foo) Clone() Foo { return f }
func caller(seed Foo) { seed.Clone().B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_struct_field_receiver() {
        let src = r#"
package main
type Foo struct{}
type Holder struct{ foo Foo }
func caller(holder Holder) { holder.foo.B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_returned_struct_field_chain() {
        let src = r#"
package main
type Foo struct{}
type Holder struct{ foo Foo }
func NewHolder() Holder { return Holder{} }
func caller() { NewHolder().foo.B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_method_call_on_embedded_field_receiver() {
        let src = r#"
package main
type Foo struct{}
type Holder struct{ Foo }
func caller(holder Holder) { holder.Foo.B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn resolves_promoted_method_call_on_embedded_receiver() {
        let src = r#"
package main
type Foo struct{}
type Holder struct{ Foo }
func caller(holder Holder) { holder.B() }
func (f Foo) B() {}
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "cmd/main.go::fn::caller"
                    && e.target_qn == "cmd/main.go::method::Foo.B"
            }),
            "edges: {:?}",
            pf.edges
        );
    }
}
