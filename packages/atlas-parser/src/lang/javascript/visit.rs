use super::helpers::*;
use super::*;

impl JsTsSyntaxFacts {
    fn extract(config: JsTsQueryConfig, root: TsNode<'_>, source: &[u8]) -> Result<Self, String> {
        let query = compile_static_query(config.language, config.language_name, config.query_text)?;
        let matches = run_query(&query, root, source);
        let mut facts = Self::default();

        for group in matches {
            for capture in group.captures {
                let key = node_key(capture.node);
                match capture.name.as_str() {
                    "atlas.export" => {
                        facts.exports.insert(key);
                    }
                    "atlas.import" => {
                        facts.imports.insert(key);
                    }
                    "atlas.definition.function" => {
                        facts.functions.insert(key);
                    }
                    "atlas.definition.class" => {
                        facts.classes.insert(key);
                    }
                    "atlas.definition.method" => {
                        facts.methods.insert(key);
                    }
                    "atlas.definition.interface" => {
                        facts.interfaces.insert(key);
                    }
                    "atlas.definition.enum" => {
                        facts.enums.insert(key);
                    }
                    "atlas.definition.variable" => match capture.node.kind() {
                        "type_alias_declaration" => {
                            facts.type_aliases.insert(key);
                        }
                        _ => {
                            facts.variables.insert(key);
                        }
                    },
                    "atlas.call" => {
                        facts.calls.insert(key);
                    }
                    _ => {}
                }
            }
        }

        Ok(facts)
    }
}

pub(super) fn parse_source(
    parser: &mut tree_sitter::Parser,
    ctx: &ParseContext<'_>,
    lang: &'static str,
    query_config: JsTsQueryConfig,
) -> (ParsedFile, Option<tree_sitter::Tree>) {
    let tree = crate::parse_runtime::parse_tree(parser, ctx.source, ctx.old_tree);
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();

    let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
    nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count, lang));

    if let Some(ref tree) = tree {
        let root = tree.root_node();
        let facts = JsTsSyntaxFacts::extract(query_config, root, ctx.source)
            .unwrap_or_else(|err| panic!("{lang} query extraction failed: {err}"));
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            visit_toplevel(child, ctx, lang, &facts, &mut nodes, &mut edges);
        }

        // Second pass: same-file call resolution.
        let mut call_edges = resolve_js_calls(root, ctx.source, ctx.rel_path, &facts, &nodes);
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

// ---------------------------------------------------------------------------
// Visitors
// ---------------------------------------------------------------------------

fn visit_toplevel(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
    facts: &JsTsSyntaxFacts,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let key = node_key(node);

    if facts.functions.contains(&key) {
        visit_function(node, ctx, ctx.rel_path, lang, nodes, edges);
    } else if facts.classes.contains(&key) {
        visit_class(node, ctx, lang, facts, nodes, edges);
    } else if facts.exports.contains(&key) {
        visit_export(node, ctx, lang, facts, nodes, edges);
    } else if facts.imports.contains(&key) {
        visit_import(node, ctx, lang, nodes, edges);
    } else if has_captured_variable_declarator(node, facts) {
        visit_variable_declaration(node, ctx, lang, facts, nodes, edges);
    } else if facts.interfaces.contains(&key) {
        visit_ts_interface(node, ctx, lang, nodes, edges);
    } else if facts.type_aliases.contains(&key) {
        visit_ts_type_alias(node, ctx, lang, nodes, edges);
    } else if facts.enums.contains(&key) {
        visit_ts_enum(node, ctx, lang, nodes, edges);
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
        repo_provenance: None,
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
    facts: &JsTsSyntaxFacts,
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
        repo_provenance: None,
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
            if facts.methods.contains(&node_key(child)) {
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
        repo_provenance: None,
    });
    edges.push(contains_edge(class_qn, &qn, ctx.rel_path, start_line(node)));
}

fn visit_export(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    lang: &str,
    facts: &JsTsSyntaxFacts,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    if let Some(decl) = node.child_by_field_name("declaration") {
        match decl.kind() {
            "function_declaration" | "generator_function_declaration" => {
                visit_function(decl, ctx, ctx.rel_path, lang, nodes, edges);
            }
            "class_declaration" | "abstract_class_declaration" => {
                visit_class(decl, ctx, lang, facts, nodes, edges);
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
                visit_variable_declaration(decl, ctx, lang, facts, nodes, edges);
            }
            _ => {}
        }
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "lexical_declaration" | "variable_declaration") {
                visit_variable_declaration(child, ctx, lang, facts, nodes, edges);
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
        repo_provenance: None,
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
        repo_provenance: None,
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
        repo_provenance: None,
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
        repo_provenance: None,
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
        repo_provenance: None,
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
    facts: &JsTsSyntaxFacts,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" && facts.variables.contains(&node_key(child)) {
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
        repo_provenance: None,
    });
    edges.push(contains_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}
