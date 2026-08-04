use tree_sitter::Node as TsNode;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};

use crate::ast_helpers::{end_line, field_text, node_text, start_line};
use crate::traits::ParseContext;

pub(super) struct GoPackage {
    pub(super) name: String,
    pub(super) line: u32,
}
// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn file_node(rel_path: &str, file_hash: &str, line_end: u32) -> Node {
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
        repo_provenance: None,
    }
}

pub(super) fn package_node(
    rel_path: &str,
    file_hash: &str,
    package_name: &str,
    qn: &str,
    line: u32,
) -> Node {
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
        repo_provenance: None,
    }
}

pub(super) fn contains_edge(parent_qn: &str, child_qn: &str, file_path: &str, line: u32) -> Edge {
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
        repo_provenance: None,
    }
}

pub(super) fn find_package(root: TsNode<'_>, source: &[u8]) -> GoPackage {
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

pub(super) fn visit_function(
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
        repo_provenance: None,
    });
    edges.push(contains_edge(
        package_qn,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

pub(super) fn visit_method(
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
        repo_provenance: None,
    });
    edges.push(contains_edge(
        package_qn,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

pub(super) fn method_receiver(node: TsNode<'_>, source: &[u8]) -> (Option<String>, String) {
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

pub(super) fn normalize_receiver_type(raw: &str) -> String {
    let no_pointer = raw.trim_start_matches('*');
    no_pointer
        .split(['[', '{'])
        .next()
        .unwrap_or(no_pointer)
        .trim()
        .to_owned()
}

pub(super) fn find_descendant_kind<'a>(node: TsNode<'a>, kinds: &[&str]) -> Option<TsNode<'a>> {
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
pub(super) fn visit_type_decl(
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
        repo_provenance: None,
    });
    edges.push(contains_edge(
        package_qn,
        &qn,
        ctx.rel_path,
        start_line(node),
    ));
}

pub(super) fn visit_imports(
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
                        repo_provenance: None,
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
                        repo_provenance: None,
                    });
                }
            }
        }
    }
}

pub(super) fn visit_value_decl(
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
                repo_provenance: None,
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
