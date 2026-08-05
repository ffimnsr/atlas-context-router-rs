//! Scala parser backed by `tree-sitter-scala`.
//! Grammar source: `tree-sitter/tree-sitter-scala`.

use std::collections::{HashMap, HashSet};

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, field_text, node_text, start_line};
use crate::query_helpers::{compile_static_query, run_query};
use crate::traits::{LangParser, ParseContext};

// SQ1 migration checklist:
// - manual extraction below owns declaration walks, scope and qualified-name rules, import/reference detection,
//   call-edge heuristics, and source metadata attachment
// - keep public parser API, graph schema, and output contracts unchanged during query migration
// - move tree-sitter syntax matching only into shared @atlas.* query captures

const SCALA_QUERY: &str = include_str!("../../queries/scala.scm");

pub struct ScalaParser;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ScalaNodeKey {
    start_byte: usize,
    end_byte: usize,
}

#[derive(Debug, Default)]
struct ScalaSyntaxFacts {
    packages: HashSet<ScalaNodeKey>,
    imports: HashSet<ScalaNodeKey>,
    objects: HashSet<ScalaNodeKey>,
    classes: HashSet<ScalaNodeKey>,
    traits: HashSet<ScalaNodeKey>,
    functions: HashSet<ScalaNodeKey>,
    _vals: HashSet<ScalaNodeKey>,
    _vars: HashSet<ScalaNodeKey>,
    calls: HashSet<ScalaNodeKey>,
}

impl ScalaSyntaxFacts {
    fn extract(root: TsNode<'_>, _source: &[u8]) -> Result<Self, String> {
        let language: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
        let query = compile_static_query(language, "scala", SCALA_QUERY)?;
        let matches = run_query(&query, root, _source);
        let mut facts = Self::default();

        for group in matches {
            for capture in group.captures {
                let key = node_key(capture.node);
                match capture.name.as_str() {
                    "atlas.definition.module" => {
                        if capture.node.kind() == "package_clause" {
                            facts.packages.insert(key);
                        } else {
                            facts.objects.insert(key);
                        }
                    }
                    "atlas.import" => {
                        facts.imports.insert(key);
                    }
                    "atlas.definition.class" => {
                        facts.classes.insert(key);
                    }
                    "atlas.definition.trait" => {
                        facts.traits.insert(key);
                    }
                    "atlas.definition.function" => {
                        facts.functions.insert(key);
                    }
                    "atlas.definition.variable" => match capture.node.kind() {
                        "val_definition" => {
                            facts._vals.insert(key);
                        }
                        "var_definition" => {
                            facts._vars.insert(key);
                        }
                        _ => {}
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

impl LangParser for ScalaParser {
    fn language_name(&self) -> &'static str {
        "scala"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".scala") || path.ends_with(".sc")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_scala::LANGUAGE.into())
            .expect("tree-sitter-scala grammar failed to load");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let facts = ScalaSyntaxFacts::extract(root, ctx.source)
                .unwrap_or_else(|err| panic!("scala query extraction failed: {err}"));
            let package_qn = emit_top_package(root, ctx, &facts, &mut nodes, &mut edges);
            let default_parent = package_qn.as_deref().unwrap_or(ctx.rel_path).to_owned();
            let mut import_index = 0usize;

            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                if facts.packages.contains(&node_key(child)) {
                    if let Some(body) = child.child_by_field_name("body") {
                        let mut body_cursor = body.walk();
                        for body_child in body.named_children(&mut body_cursor) {
                            visit_scala_node(
                                body_child,
                                ctx,
                                &facts,
                                package_qn.as_deref().unwrap_or(ctx.rel_path),
                                None,
                                &mut import_index,
                                &mut nodes,
                                &mut edges,
                            );
                        }
                    }
                    continue;
                }

                visit_scala_node(
                    child,
                    ctx,
                    &facts,
                    &default_parent,
                    None,
                    &mut import_index,
                    &mut nodes,
                    &mut edges,
                );
            }

            let callable_map = callable_qn_map(&nodes);
            let mut call_edges = Vec::new();
            walk_calls(root, ctx, &facts, &callable_map, None, &mut call_edges);
            edges.extend(call_edges);
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("scala".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn emit_top_package(
    root: TsNode<'_>,
    ctx: &ParseContext<'_>,
    facts: &ScalaSyntaxFacts,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) -> Option<String> {
    let mut cursor = root.walk();
    let package = root
        .named_children(&mut cursor)
        .find(|child| facts.packages.contains(&node_key(*child)))?;
    let name = package_name(package, ctx.source)?;
    let qn = format!("{}::package::{}", ctx.rel_path, name);
    let line = start_line(package);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Package,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(package),
        language: "scala".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, line));
    Some(qn)
}

#[allow(clippy::too_many_arguments)]
fn visit_scala_node(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    facts: &ScalaSyntaxFacts,
    parent_qn: &str,
    current_owner: Option<&str>,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let key = node_key(node);
    if facts.imports.contains(&key) {
        emit_import(node, ctx, parent_qn, import_index, nodes, edges);
        return;
    }
    if facts.objects.contains(&key) {
        emit_object(node, ctx, facts, parent_qn, import_index, nodes, edges);
        return;
    }
    if facts.classes.contains(&key) {
        emit_class(node, ctx, facts, parent_qn, import_index, nodes, edges);
        return;
    }
    if facts.traits.contains(&key) {
        emit_trait(node, ctx, facts, parent_qn, import_index, nodes, edges);
        return;
    }
    if facts.functions.contains(&key) {
        emit_function(node, ctx, parent_qn, current_owner, nodes, edges);
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_scala_node(
            child,
            ctx,
            facts,
            parent_qn,
            current_owner,
            import_index,
            nodes,
            edges,
        );
    }
}

fn emit_object(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    facts: &ScalaSyntaxFacts,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source).map(str::to_owned) else {
        return;
    };
    let qn = format!("{}::object::{}", ctx.rel_path, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Module,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "scala".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "kind": "object" }),
        repo_provenance: None,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            visit_scala_node(
                child,
                ctx,
                facts,
                &qn,
                Some(name.as_str()),
                import_index,
                nodes,
                edges,
            );
        }
    }
}

fn emit_class(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    facts: &ScalaSyntaxFacts,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source).map(str::to_owned) else {
        return;
    };
    let is_case_class = node_text(node, ctx.source)
        .trim_start()
        .starts_with("case class");
    let qn_prefix = if is_case_class { "case_class" } else { "class" };
    let qn = format!("{}::{}::{}", ctx.rel_path, qn_prefix, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Class,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "scala".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: node
            .child_by_field_name("class_parameters")
            .map(|n| node_text(n, ctx.source).to_owned()),
        return_type: None,
        modifiers: scala_modifiers(node, ctx.source),
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "case_class": is_case_class }),
        repo_provenance: None,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            visit_scala_node(
                child,
                ctx,
                facts,
                &qn,
                Some(name.as_str()),
                import_index,
                nodes,
                edges,
            );
        }
    }
}

fn emit_trait(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    facts: &ScalaSyntaxFacts,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source).map(str::to_owned) else {
        return;
    };
    let qn = format!("{}::trait::{}", ctx.rel_path, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Trait,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "scala".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: node
            .child_by_field_name("class_parameters")
            .map(|n| node_text(n, ctx.source).to_owned()),
        return_type: None,
        modifiers: scala_modifiers(node, ctx.source),
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            visit_scala_node(
                child,
                ctx,
                facts,
                &qn,
                Some(name.as_str()),
                import_index,
                nodes,
                edges,
            );
        }
    }
}

fn emit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    current_owner: Option<&str>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name) = field_text(node, "name", ctx.source).map(str::to_owned) else {
        return;
    };
    let (kind, qn) = if let Some(owner) = current_owner {
        (
            NodeKind::Method,
            format!("{}::method::{}.{}", ctx.rel_path, owner, name),
        )
    } else {
        (
            NodeKind::Function,
            format!("{}::fn::{}", ctx.rel_path, name),
        )
    };
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "scala".to_owned(),
        parent_name: Some(parent_qn.to_owned()),
        params: collect_scala_parameters(node, ctx.source),
        return_type: node
            .child_by_field_name("return_type")
            .map(|n| node_text(n, ctx.source).to_owned()),
        modifiers: scala_modifiers(node, ctx.source),
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    });
    edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
}

fn emit_import(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    parent_qn: &str,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    for target in scala_import_targets(node_text(node, ctx.source)) {
        *import_index += 1;
        let qn = format!("{}::import::scala:{}", ctx.rel_path, *import_index);
        let line = start_line(node);
        nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Import,
            name: target.clone(),
            qualified_name: qn.clone(),
            file_path: ctx.rel_path.to_owned(),
            line_start: line,
            line_end: end_line(node),
            language: "scala".to_owned(),
            parent_name: Some(parent_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: ctx.file_hash.to_owned(),
            extra_json: serde_json::json!({ "imported": target }),
            repo_provenance: None,
        });
        edges.push(contains_edge(parent_qn, &qn, ctx.rel_path, line));
        edges.push(imports_edge(
            parent_qn,
            &qn,
            ctx.rel_path,
            line,
            "explicit_import",
        ));
    }
}

fn callable_qn_map(nodes: &[Node]) -> HashMap<String, String> {
    nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Function | NodeKind::Method))
        .map(|node| (node.name.clone(), node.qualified_name.clone()))
        .collect()
}

fn walk_calls(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    facts: &ScalaSyntaxFacts,
    callables: &HashMap<String, String>,
    current_callable: Option<String>,
    edges: &mut Vec<Edge>,
) {
    let mut next_callable = current_callable;
    let key = node_key(node);
    if facts.functions.contains(&key) {
        if let Some(name) = field_text(node, "name", ctx.source) {
            next_callable = callables.get(name).cloned();
        }
    } else if facts.calls.contains(&key)
        && let Some(owner_qn) = next_callable.as_ref()
        && let Some(callee) = scala_call_name(node, ctx.source)
        && let Some(target_qn) = callables.get(&callee)
    {
        edges.push(call_edge(
            owner_qn,
            target_qn,
            ctx.rel_path,
            start_line(node),
            "same_file",
        ));
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_calls(child, ctx, facts, callables, next_callable.clone(), edges);
    }
}

fn scala_call_name(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let function = node.child_by_field_name("function")?;
    last_identifier(function, source)
}

fn last_identifier(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    if matches!(node.kind(), "identifier" | "operator_identifier") {
        return Some(node_text(node, source).to_owned());
    }

    let mut cursor = node.walk();
    let mut last = None;
    for child in node.named_children(&mut cursor) {
        if let Some(found) = last_identifier(child, source) {
            last = Some(found);
        }
    }
    last
}

fn scala_modifiers(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let raw = node_text(node, source).trim_start();
    if raw.starts_with("case class") {
        return Some("case".to_owned());
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "modifiers" {
            return Some(
                node_text(child, source)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
    }
    None
}

fn collect_scala_parameters(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let mut params = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "parameters" || child.kind() == "type_parameters" {
            params.push(node_text(child, source).to_owned());
        }
    }
    if params.is_empty() {
        None
    } else {
        Some(params.join(" "))
    }
}

fn scala_import_targets(raw: &str) -> Vec<String> {
    let spec = raw.trim().trim_start_matches("import").trim();
    if spec.is_empty() {
        return Vec::new();
    }
    if spec.contains('{') {
        return vec![spec.to_owned()];
    }
    spec.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn package_name(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, source).replace(' ', ""))
}

fn node_key(node: TsNode<'_>) -> ScalaNodeKey {
    ScalaNodeKey {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}

fn file_node(rel_path: &str, file_hash: &str, line_end: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned(),
        qualified_name: rel_path.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: 1,
        line_end,
        language: "scala".to_owned(),
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
        repo_provenance: None,
    }
}

fn imports_edge(source_qn: &str, target_qn: &str, file_path: &str, line: u32, tier: &str) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Imports,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(line),
        confidence: 1.0,
        confidence_tier: Some(tier.to_owned()),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn call_edge(source_qn: &str, target_qn: &str, file_path: &str, line: u32, tier: &str) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(line),
        confidence: 1.0,
        confidence_tier: Some(tier.to_owned()),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ParsedFile {
        let (pf, _) = ScalaParser.parse(&ParseContext {
            rel_path: "src/Main.scala",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_package_objects_classes_traits_methods_imports_and_calls() {
        let pf = parse(
            "package demo.app\n\nimport demo.support.Helper\n\nobject Runner {\n  def helper(): Unit = ()\n  def run(): Unit = helper()\n}\n\ncase class Box(value: Int)\nclass Worker { def work(): Unit = () }\ntrait Service { def ping(): Unit }\n",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.scala::package::demo.app")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.scala::object::Runner")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.scala::case_class::Box")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.scala::class::Worker")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/Main.scala::trait::Service")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "demo.support.Helper")
        );
        assert!(pf.edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/Main.scala::method::Runner.helper"
        }));
    }
}
