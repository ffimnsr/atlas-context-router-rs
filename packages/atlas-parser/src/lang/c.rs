//! C parser backed by `tree-sitter-c`.
//! Grammar source: `tree-sitter/tree-sitter-c`.

use std::collections::{HashMap, HashSet};

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::query_helpers::{compile_static_query, run_query};
use crate::traits::{LangParser, ParseContext};

// SQ1 migration checklist:
// - manual extraction below owns declaration walks, scope and qualified-name rules, import/reference detection,
//   call-edge heuristics, and source metadata attachment
// - keep public parser API, graph schema, and output contracts unchanged during query migration
// - move tree-sitter syntax matching only into shared @atlas.* query captures

const C_QUERY: &str = include_str!("../../queries/c.scm");

pub struct CParser;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CNodeKey {
    start_byte: usize,
    end_byte: usize,
}

#[derive(Debug, Default)]
struct CSyntaxFacts {
    includes: HashSet<CNodeKey>,
    functions: HashSet<CNodeKey>,
    structs: HashSet<CNodeKey>,
    enums: HashSet<CNodeKey>,
    typedefs: HashSet<CNodeKey>,
    calls: HashSet<CNodeKey>,
}

impl CSyntaxFacts {
    fn extract(root: TsNode<'_>, source: &[u8]) -> Result<Self, String> {
        let language: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();
        let query = compile_static_query(language, "c", C_QUERY)?;
        let matches = run_query(&query, root, source);
        let mut facts = Self::default();

        for group in matches {
            for capture in group.captures {
                let key = node_key(capture.node);
                match capture.name.as_str() {
                    "atlas.import" => {
                        facts.includes.insert(key);
                    }
                    "atlas.definition.function" => {
                        facts.functions.insert(key);
                    }
                    "atlas.definition.struct" => {
                        facts.structs.insert(key);
                    }
                    "atlas.definition.enum" => {
                        facts.enums.insert(key);
                    }
                    "atlas.definition.variable"
                        if node_text(capture.node, source).contains("typedef") =>
                    {
                        facts.typedefs.insert(key);
                    }
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

impl LangParser for CParser {
    fn language_name(&self) -> &'static str {
        "c"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".c")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .expect("tree-sitter-c grammar failed to load");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let facts = CSyntaxFacts::extract(root, ctx.source)
                .unwrap_or_else(|err| panic!("c query extraction failed: {err}"));
            let mut import_index = 0usize;
            let mut cursor = root.walk();
            for child in root.named_children(&mut cursor) {
                visit_node(
                    child,
                    ctx,
                    &facts,
                    &mut import_index,
                    &mut nodes,
                    &mut edges,
                );
            }

            let function_map = function_qn_map(&nodes);
            let mut call_edges = Vec::new();
            walk_calls(root, ctx, &facts, &function_map, None, &mut call_edges);
            edges.extend(call_edges);
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("c".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn visit_node(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    facts: &CSyntaxFacts,
    import_index: &mut usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let key = node_key(node);
    if facts.includes.contains(&key) {
        *import_index += 1;
        emit_include(node, ctx, *import_index, nodes, edges);
        return;
    }
    if facts.functions.contains(&key) {
        emit_function(node, ctx, nodes, edges);
        return;
    }
    if facts.structs.contains(&key) {
        emit_named_type(node, ctx, "struct", NodeKind::Struct, nodes, edges);
        return;
    }
    if facts.enums.contains(&key) {
        emit_named_type(node, ctx, "enum", NodeKind::Enum, nodes, edges);
        return;
    }
    if facts.typedefs.contains(&key) {
        emit_typedef(node, ctx, nodes, edges);
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_node(child, ctx, facts, import_index, nodes, edges);
    }
}

fn emit_include(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    import_index: usize,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let raw = node_text(node, ctx.source).trim();
    let name = raw
        .split_once('#')
        .map(|(_, include)| include.trim().trim_start_matches("include").trim())
        .unwrap_or(raw)
        .trim_matches('<')
        .trim_matches('>')
        .trim_matches('"')
        .to_owned();
    let qn = format!("{}::import::c:{}", ctx.rel_path, import_index);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Import,
        name: name.clone(),
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "c".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "imported": name }),
        repo_provenance: None,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, line));
    edges.push(imports_edge(
        ctx.rel_path,
        &qn,
        ctx.rel_path,
        line,
        "preproc_include",
    ));
}

fn emit_function(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(declarator) = node.child_by_field_name("declarator") else {
        return;
    };
    let Some(name) = declarator_name(declarator, ctx.source) else {
        return;
    };
    let qn = format!("{}::fn::{}", ctx.rel_path, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "c".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: Some(node_text(declarator, ctx.source).to_owned()),
        return_type: node
            .child_by_field_name("type")
            .map(|ret| node_text(ret, ctx.source).to_owned()),
        modifiers: None,
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, line));
}

fn emit_named_type(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    qn_prefix: &str,
    kind: NodeKind,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = node_text(name_node, ctx.source).to_owned();
    let qn = format!("{}::{}::{}", ctx.rel_path, qn_prefix, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "c".to_owned(),
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
}

fn emit_typedef(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let raw = node_text(node, ctx.source);
    let name = raw
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .rfind(|segment| !segment.is_empty() && *segment != "typedef")
        .map(str::to_owned);
    let Some(name) = name else {
        return;
    };
    let qn = format!("{}::typedef::{}", ctx.rel_path, name);
    let line = start_line(node);
    nodes.push(Node {
        id: NodeId::UNSET,
        kind: NodeKind::Variable,
        name,
        qualified_name: qn.clone(),
        file_path: ctx.rel_path.to_owned(),
        line_start: line,
        line_end: end_line(node),
        language: "c".to_owned(),
        parent_name: Some(ctx.rel_path.to_owned()),
        params: None,
        return_type: None,
        modifiers: Some("typedef".to_owned()),
        is_test: false,
        file_hash: ctx.file_hash.to_owned(),
        extra_json: serde_json::json!({ "kind": "typedef" }),
        repo_provenance: None,
    });
    edges.push(contains_edge(ctx.rel_path, &qn, ctx.rel_path, line));
}

fn function_qn_map(nodes: &[Node]) -> HashMap<String, String> {
    nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Function)
        .map(|node| (node.name.clone(), node.qualified_name.clone()))
        .collect()
}

fn walk_calls(
    node: TsNode<'_>,
    ctx: &ParseContext<'_>,
    facts: &CSyntaxFacts,
    functions: &HashMap<String, String>,
    current_fn: Option<String>,
    edges: &mut Vec<Edge>,
) {
    let mut next_fn = current_fn;
    let key = node_key(node);
    if facts.functions.contains(&key)
        && let Some(declarator) = node.child_by_field_name("declarator")
        && let Some(name) = declarator_name(declarator, ctx.source)
    {
        next_fn = functions.get(&name).cloned();
    } else if facts.calls.contains(&key)
        && let Some(owner_qn) = next_fn.as_ref()
        && let Some(function_node) = node.child_by_field_name("function")
        && let Some(name) = declarator_name(function_node, ctx.source)
        && let Some(target_qn) = functions.get(&name)
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
        walk_calls(child, ctx, facts, functions, next_fn.clone(), edges);
    }
}

fn declarator_name(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" | "type_identifier" => {
            Some(node_text(node, source).to_owned())
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if let Some(name) = declarator_name(child, source) {
                    return Some(name);
                }
            }
            None
        }
    }
}

fn node_key(node: TsNode<'_>) -> CNodeKey {
    CNodeKey {
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
        language: "c".to_owned(),
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
        let (pf, _) = CParser.parse(&ParseContext {
            rel_path: "src/native.c",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_functions_types_includes_and_calls() {
        let pf = parse(
            "#include \"util.h\"\ntypedef unsigned long size_t;\nstruct widget { int id; };\nenum mode { ON };\nstatic void helper(void) {}\nvoid run(void) { helper(); }\n",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Import && node.name == "util.h")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/native.c::typedef::size_t")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/native.c::struct::widget")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "src/native.c::enum::mode")
        );
        assert!(pf.edges.iter().any(
            |edge| edge.kind == EdgeKind::Calls && edge.target_qn == "src/native.c::fn::helper"
        ));
    }
}
