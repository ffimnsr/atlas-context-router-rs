use std::collections::{HashMap, HashSet};
use tree_sitter::Node as TsNode;

use atlas_core::{Edge, EdgeKind, Node, NodeKind};

use crate::ast_helpers::{node_text, start_line};

use super::facts::rust_query_matches;

// ---------------------------------------------------------------------------
// Same-file reference resolution
// ---------------------------------------------------------------------------

pub(super) fn resolve_same_file_references(
    root: TsNode<'_>,
    source: &[u8],
    rel_path: &str,
    nodes: &[Node],
) -> Vec<Edge> {
    let reference_sites = extract_rust_reference_sites(root, source)
        .unwrap_or_else(|err| panic!("rust reference query failed: {err}"));
    let mut symbol_targets: HashMap<String, Vec<String>> = HashMap::new();
    let mut type_targets: HashMap<String, Vec<String>> = HashMap::new();

    for node in nodes {
        if node.kind == NodeKind::File {
            continue;
        }

        symbol_targets
            .entry(node.name.clone())
            .or_default()
            .push(node.qualified_name.clone());

        if matches!(
            node.kind,
            NodeKind::Module | NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
        ) {
            type_targets
                .entry(node.name.clone())
                .or_default()
                .push(node.qualified_name.clone());
        }
    }

    ReferenceResolver {
        source,
        rel_path,
        nodes,
        symbol_targets,
        type_targets,
        seen: HashSet::new(),
        edges: Vec::new(),
    }
    .resolve_sites(&reference_sites)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RustReferenceKind {
    UseArgument,
    Type,
}

#[derive(Clone, Copy, Debug)]
struct RustReferenceSite<'tree> {
    node: TsNode<'tree>,
    target_node: TsNode<'tree>,
    kind: RustReferenceKind,
}

fn extract_rust_reference_sites<'tree>(
    root: TsNode<'tree>,
    source: &'tree [u8],
) -> Result<Vec<RustReferenceSite<'tree>>, String> {
    let matches = rust_query_matches(root, source)?;
    let mut sites = Vec::new();

    for group in matches {
        let mut use_node = None;
        let mut use_argument = None;
        let mut type_reference = None;

        for capture in &group.captures {
            match capture.name.as_str() {
                "atlas.reference.use" => use_node = Some(capture.node),
                "atlas.reference.use_argument" => use_argument = Some(capture.node),
                "atlas.reference.type" => type_reference = Some(capture.node),
                _ => {}
            }
        }

        if let (Some(node), Some(target_node)) = (use_node, use_argument) {
            sites.push(RustReferenceSite {
                node,
                target_node,
                kind: RustReferenceKind::UseArgument,
            });
        }
        if let Some(target_node) = type_reference {
            sites.push(RustReferenceSite {
                node: target_node,
                target_node,
                kind: RustReferenceKind::Type,
            });
        }
    }

    sites.sort_by_key(|site| (site.node.start_byte(), site.node.end_byte()));
    Ok(sites)
}

struct ReferenceResolver<'a> {
    source: &'a [u8],
    rel_path: &'a str,
    nodes: &'a [Node],
    symbol_targets: HashMap<String, Vec<String>>,
    type_targets: HashMap<String, Vec<String>>,
    seen: HashSet<(String, String, u32)>,
    edges: Vec<Edge>,
}

impl<'a> ReferenceResolver<'a> {
    fn resolve_sites(mut self, sites: &[RustReferenceSite<'_>]) -> Vec<Edge> {
        for site in sites {
            match site.kind {
                RustReferenceKind::UseArgument => {
                    let source_qn =
                        reference_source_qn(self.nodes, self.rel_path, start_line(site.node));
                    for name in use_reference_names(site.target_node, self.source) {
                        let target_qn =
                            unique_target_qn(&self.symbol_targets, &name).map(str::to_owned);
                        self.maybe_push_reference_edge(
                            source_qn,
                            target_qn.as_deref(),
                            start_line(site.node),
                        );
                    }
                }
                RustReferenceKind::Type => {
                    if is_definition_name(site.target_node) {
                        continue;
                    }
                    let source_qn =
                        reference_source_qn(self.nodes, self.rel_path, start_line(site.node));
                    let name = type_reference_name(site.target_node, self.source);
                    let target_qn = unique_target_qn(&self.type_targets, &name).map(str::to_owned);
                    self.maybe_push_reference_edge(
                        source_qn,
                        target_qn.as_deref(),
                        start_line(site.node),
                    );
                }
            }
        }
        self.edges
    }

    fn maybe_push_reference_edge(&mut self, source_qn: &str, target_qn: Option<&str>, line: u32) {
        let Some(target_qn) = target_qn else {
            return;
        };
        if source_qn == target_qn {
            return;
        }

        let key = (source_qn.to_owned(), target_qn.to_owned(), line);
        if !self.seen.insert(key.clone()) {
            return;
        }

        self.edges.push(reference_edge(
            &key.0,
            &key.1,
            self.rel_path,
            line,
            Some("same_file".to_owned()),
        ));
    }
}

fn unique_target_qn<'a>(targets: &'a HashMap<String, Vec<String>>, name: &str) -> Option<&'a str> {
    match targets.get(name) {
        Some(entries) if entries.len() == 1 => entries.first().map(|entry| entry.as_str()),
        _ => None,
    }
}

fn reference_source_qn<'a>(nodes: &'a [Node], rel_path: &'a str, line: u32) -> &'a str {
    nodes
        .iter()
        .filter(|node| {
            node.kind != NodeKind::File && node.line_start <= line && line <= node.line_end
        })
        .min_by_key(|node| {
            (
                node.line_end.saturating_sub(node.line_start),
                node.line_start,
            )
        })
        .map(|node| node.qualified_name.as_str())
        .unwrap_or(rel_path)
}

fn use_reference_names(node: TsNode<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    if node.kind() == "use_declaration" {
        if let Some(argument) = node.child_by_field_name("argument") {
            collect_use_reference_names(argument, source, &mut names);
        }
    } else {
        collect_use_reference_names(node, source, &mut names);
    }
    names.sort();
    names.dedup();
    names
}

fn collect_use_reference_names(node: TsNode<'_>, source: &[u8], names: &mut Vec<String>) {
    match node.kind() {
        "identifier" => push_reference_name(node_text(node, source), names),
        "scoped_identifier" => {
            push_reference_name(last_path_segment(node_text(node, source)), names)
        }
        "use_as_clause" => {
            if let Some(path) = node.child_by_field_name("path") {
                collect_use_reference_names(path, source, names);
            }
            return;
        }
        "scoped_use_list" => {
            if let Some(path) = node.child_by_field_name("path") {
                collect_use_reference_names(path, source, names);
            }
            if let Some(list) = node.child_by_field_name("list") {
                collect_use_reference_names(list, source, names);
            }
            return;
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_use_reference_names(child, source, names);
    }
}

fn push_reference_name(name: &str, names: &mut Vec<String>) {
    if matches!(name, "crate" | "self" | "super" | "Self") || name.is_empty() {
        return;
    }
    names.push(name.to_owned());
}

fn type_reference_name(node: TsNode<'_>, source: &[u8]) -> String {
    last_path_segment(node_text(node, source)).to_owned()
}

pub(super) fn last_path_segment(path: &str) -> &str {
    path.split("::")
        .last()
        .unwrap_or(path)
        .split('<')
        .next()
        .unwrap_or(path)
        .trim()
}

fn is_definition_name(node: TsNode<'_>) -> bool {
    node.parent()
        .and_then(|parent| parent.child_by_field_name("name"))
        .is_some_and(|name| name == node)
}

fn reference_edge(
    source_qn: &str,
    target_qn: &str,
    rel_path: &str,
    line: u32,
    confidence_tier: Option<String>,
) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::References,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: 0.75,
        confidence_tier,
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}
