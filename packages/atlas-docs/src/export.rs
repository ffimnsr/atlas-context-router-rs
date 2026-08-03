//! Dependency diagram export in Mermaid or Graphviz DOT form.
//!
//! [`export_diagram`] renders a deterministic, aggregated graph for one of
//! five scopes: `repo` (module-level, falling back to file-level), `module`,
//! `component`, `file`, and `symbol` (neighborhood).  Node ids are stable
//! (`n0`, `n1`, …) and edges are aggregated by `(source, target, kind)` with
//! counts, so diagrams are reproducible and bounded by `max_nodes` /
//! `max_edges` caps that report omitted elements.

use std::collections::{BTreeMap, BTreeSet};

use atlas_core::{AtlasError, Edge, EdgeKind, Node, Result};

use crate::model::DocsView;

/// Supported diagram output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocsExportFormat {
    /// Mermaid `flowchart LR` diagram.
    Mermaid,
    /// Graphviz `digraph` source.
    Dot,
}

impl DocsExportFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mermaid => "mermaid",
            Self::Dot => "dot",
        }
    }
}

/// Which part of the repository the diagram describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocsExportScope {
    /// Whole-repo module dependency graph (file graph when no modules).
    Repo,
    /// One inferred module's internal symbol graph.
    Module,
    /// One component label's file graph.
    Component,
    /// One file's internal symbol graph.
    File,
    /// One symbol and its direct neighbors.
    Symbol,
}

impl DocsExportScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repo => "repo",
            Self::Module => "module",
            Self::Component => "component",
            Self::File => "file",
            Self::Symbol => "symbol",
        }
    }
}

/// Parameters for one diagram export.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportRequest {
    pub format: DocsExportFormat,
    pub scope: DocsExportScope,
    /// Target name for `module`, `component`, `file`, and `symbol` scopes.
    pub name: Option<String>,
    /// Cap on rendered nodes; extra nodes (and their edges) are omitted and
    /// reported.  Use `usize::MAX` for no cap.
    pub max_nodes: usize,
    /// Cap on rendered edges; extra edges are omitted and reported.
    pub max_edges: usize,
}

/// Rendered diagram plus determinism-friendly counts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportResult {
    pub content: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub omitted_nodes: usize,
    pub omitted_edges: usize,
}

struct AggEdge {
    source: usize,
    target: usize,
    kind: &'static str,
    count: usize,
}

/// Render the requested diagram.  Errors when a scoped `name` does not
/// resolve to anything in the graph.
pub fn export_diagram(view: &DocsView<'_>, request: &ExportRequest) -> Result<ExportResult> {
    let (mut labels, edges) = select_scope(view, request)?;
    let total_edges = edges.len();

    // Stable ids: node `n{i}` always refers to the i-th label in sorted order.
    labels.sort();
    labels.dedup();
    let kept = labels.len().min(request.max_nodes);
    let omitted_nodes = labels.len() - kept;
    labels.truncate(kept);
    let in_node_set: BTreeSet<&str> = labels.iter().map(String::as_str).collect();

    let mut aggregated = aggregate_edges(&edges, &in_node_set);
    let kept_edges = if aggregated.len() > request.max_edges {
        aggregated.truncate(request.max_edges);
        request.max_edges
    } else {
        aggregated.len()
    };
    // Omitted edges are raw edges that are not represented in the output:
    // boundary-crossing edges plus edges dropped by the caps.  Aggregation
    // collapse (many raw edges into one counted edge) is not an omission.
    let rendered_raw_edges: usize = aggregated.iter().map(|edge| edge.count).sum();
    let omitted_edges = total_edges.saturating_sub(rendered_raw_edges);

    let content = match request.format {
        DocsExportFormat::Mermaid => render_mermaid(
            view,
            request,
            &labels,
            &aggregated,
            omitted_nodes,
            omitted_edges,
        ),
        DocsExportFormat::Dot => render_dot(
            view,
            request,
            &labels,
            &aggregated,
            omitted_nodes,
            omitted_edges,
        ),
    };

    Ok(ExportResult {
        content,
        node_count: labels.len(),
        edge_count: kept_edges,
        omitted_nodes,
        omitted_edges,
    })
}

type ScopeEdges<'a> = Vec<(&'a str, &'a str, &'static str)>;

/// Select node labels and raw edges for the scope.
///
/// Edges are pre-mapped to `(source label, target label, kind)` pairs so the
/// aggregation step only needs label indexes.  The returned edge list is the
/// full set of edges *touching* the scope; edges whose endpoints both land in
/// the final (possibly capped) node set are rendered, everything else counts
/// as omitted.
fn select_scope<'a>(
    view: &DocsView<'a>,
    request: &ExportRequest,
) -> Result<(Vec<String>, ScopeEdges<'a>)> {
    let data = view.data();
    let name = request.name.as_deref();
    let required = |scope: DocsExportScope| -> Result<&str> {
        name.ok_or_else(|| {
            AtlasError::Other(format!(
                "`--name` is required for the {} scope",
                scope.as_str()
            ))
        })
    };

    match request.scope {
        DocsExportScope::Repo => {
            if data.modules.is_empty() {
                let labels: Vec<String> = data
                    .files
                    .iter()
                    .filter(|file| !crate::model::DocsData::is_synthetic_path(&file.path))
                    .map(|file| file.path.clone())
                    .collect();
                let edges = touching_file_edges(view)
                    .into_iter()
                    .map(|(edge, source, target)| {
                        (
                            source.file_path.as_str(),
                            target.file_path.as_str(),
                            edge.kind.as_str(),
                        )
                    });
                Ok((labels, edges.collect()))
            } else {
                let module_by_symbol = module_by_symbol_map(view);
                let labels: Vec<String> = data
                    .modules
                    .iter()
                    .map(|module| module.display_name.clone())
                    .collect();
                let edges = data
                    .edges
                    .iter()
                    .filter(|edge| !matches!(edge.kind, EdgeKind::Contains))
                    .filter_map(|edge| {
                        let source = module_by_symbol.get(edge.source_qn.as_str())?;
                        let target = module_by_symbol.get(edge.target_qn.as_str())?;
                        (source != target).then_some((*source, *target, edge.kind.as_str()))
                    })
                    .collect();
                Ok((labels, edges))
            }
        }
        DocsExportScope::Module => {
            let target = required(DocsExportScope::Module)?;
            let module = data
                .modules
                .iter()
                .find(|module| module.module_id == target || module.display_name == target)
                .ok_or_else(|| {
                    AtlasError::Other(format!(
                        "unknown module '{target}'; available: {}",
                        available_list(
                            data.modules
                                .iter()
                                .map(|module| module.display_name.as_str())
                        )
                    ))
                })?;
            let node_set: BTreeSet<&str> =
                module.owned_symbols.iter().map(String::as_str).collect();
            let labels = module.owned_symbols.clone();
            let edges = touching_symbol_edges(view, &node_set)
                .into_iter()
                .map(|(edge, _, _)| {
                    (
                        edge.source_qn.as_str(),
                        edge.target_qn.as_str(),
                        edge.kind.as_str(),
                    )
                });
            Ok((labels, edges.collect()))
        }
        DocsExportScope::Component => {
            let target = required(DocsExportScope::Component)?;
            let labels: Vec<String> = data
                .component_assignments
                .iter()
                .filter(|assignment| {
                    assignment.qualified_name.is_none()
                        && assignment.labels.iter().any(|label| label.label == target)
                })
                .map(|assignment| assignment.file_path.clone())
                .collect();
            if labels.is_empty() {
                return Err(AtlasError::Other(format!(
                    "unknown component '{target}'; available: {}",
                    available_list(component_label_names(view))
                )));
            }
            let edges = {
                let node_set: BTreeSet<&str> = labels.iter().map(String::as_str).collect();
                touching_file_edges(view)
                    .into_iter()
                    .filter_map(|(edge, source, target)| {
                        let source_in = node_set.contains(source.file_path.as_str());
                        let target_in = node_set.contains(target.file_path.as_str());
                        (source_in || target_in).then_some((
                            source.file_path.as_str(),
                            target.file_path.as_str(),
                            edge.kind.as_str(),
                        ))
                    })
                    .collect::<Vec<_>>()
            };
            Ok((labels, edges))
        }
        DocsExportScope::File => {
            let target = required(DocsExportScope::File)?;
            if !data.files.iter().any(|file| file.path == target)
                && view.nodes_in_file(target).is_empty()
            {
                return Err(AtlasError::Other(format!(
                    "unknown file '{target}'; available: {}",
                    available_list(data.files.iter().filter_map(|file| {
                        (!crate::model::DocsData::is_synthetic_path(&file.path))
                            .then_some(file.path.as_str())
                    }))
                )));
            }
            let mut labels: Vec<String> = view
                .nodes_in_file(target)
                .iter()
                .filter(|node| !matches!(node.kind, atlas_core::NodeKind::File))
                .map(|node| node.qualified_name.clone())
                .collect();
            labels.sort();
            labels.dedup();
            let edges = {
                let node_set: BTreeSet<&str> = labels.iter().map(String::as_str).collect();
                touching_symbol_edges(view, &node_set)
                    .into_iter()
                    .map(|(edge, _, _)| {
                        (
                            edge.source_qn.as_str(),
                            edge.target_qn.as_str(),
                            edge.kind.as_str(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            Ok((labels, edges))
        }
        DocsExportScope::Symbol => {
            let target = required(DocsExportScope::Symbol)?;
            let center = view.node_by_qname(target).ok_or_else(|| {
                AtlasError::Other(format!(
                    "unknown symbol '{target}'; available: {}",
                    available_list(view.symbol_nodes().map(|node| node.qualified_name.as_str()))
                ))
            })?;
            let mut neighbors: BTreeSet<&str> = BTreeSet::new();
            neighbors.insert(center.qualified_name.as_str());
            for edge in view.outbound_edges(target) {
                neighbors.insert(edge.target_qn.as_str());
            }
            for edge in view.inbound_edges(target) {
                neighbors.insert(edge.source_qn.as_str());
            }
            let labels: Vec<String> = neighbors.into_iter().map(str::to_owned).collect();
            let edges = {
                let node_set: BTreeSet<&str> = labels.iter().map(String::as_str).collect();
                touching_symbol_edges(view, &node_set)
                    .into_iter()
                    .map(|(edge, _, _)| {
                        (
                            edge.source_qn.as_str(),
                            edge.target_qn.as_str(),
                            edge.kind.as_str(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            Ok((labels, edges))
        }
    }
}

/// Map every owned symbol qualified name to its module display name.
fn module_by_symbol_map<'a>(view: &DocsView<'a>) -> BTreeMap<&'a str, &'a str> {
    view.data()
        .modules
        .iter()
        .flat_map(|module| {
            module
                .owned_symbols
                .iter()
                .map(move |qname| (qname.as_str(), module.display_name.as_str()))
        })
        .collect()
}

/// Edges (excluding `Contains`) with at least one symbol endpoint in
/// `node_set`, paired with their endpoint nodes.
fn touching_symbol_edges<'a>(
    view: &DocsView<'a>,
    node_set: &BTreeSet<&str>,
) -> Vec<(&'a Edge, &'a Node, &'a Node)> {
    view.data()
        .edges
        .iter()
        .filter_map(|edge| {
            if matches!(edge.kind, EdgeKind::Contains) {
                return None;
            }
            let (Some(source), Some(target)) = (
                view.node_by_qname(edge.source_qn.as_str()),
                view.node_by_qname(edge.target_qn.as_str()),
            ) else {
                return None;
            };
            (node_set.contains(edge.source_qn.as_str())
                || node_set.contains(edge.target_qn.as_str()))
            .then_some((edge, source, target))
        })
        .collect()
}

/// Cross-file edges (excluding `Contains`) paired with their endpoint nodes.
fn touching_file_edges<'a>(view: &DocsView<'a>) -> Vec<(&'a Edge, &'a Node, &'a Node)> {
    view.data()
        .edges
        .iter()
        .filter_map(|edge| {
            if matches!(edge.kind, EdgeKind::Contains) {
                return None;
            }
            let (Some(source), Some(target)) = (
                view.node_by_qname(edge.source_qn.as_str()),
                view.node_by_qname(edge.target_qn.as_str()),
            ) else {
                return None;
            };
            (source.file_path != target.file_path).then_some((edge, source, target))
        })
        .collect()
}

/// Aggregate raw edges by `(source label, target label, kind)`; edges whose
/// endpoints fell outside the (possibly capped) node set are dropped.
fn aggregate_edges(
    edges: &[(&str, &str, &'static str)],
    node_set: &BTreeSet<&str>,
) -> Vec<AggEdge> {
    let mut grouped: BTreeMap<(usize, usize, &'static str), usize> = BTreeMap::new();
    let mut index_of: BTreeMap<&str, usize> = BTreeMap::new();
    for (index, label) in node_set.iter().enumerate() {
        index_of.insert(label, index);
    }
    for (source, target, kind) in edges {
        let (Some(&source), Some(&target)) = (index_of.get(*source), index_of.get(*target)) else {
            continue;
        };
        if source == target {
            continue;
        }
        *grouped.entry((source, target, *kind)).or_default() += 1;
    }
    grouped
        .into_iter()
        .map(|((source, target, kind), count)| AggEdge {
            source,
            target,
            kind,
            count,
        })
        .collect()
}

fn render_mermaid(
    view: &DocsView<'_>,
    request: &ExportRequest,
    labels: &[String],
    edges: &[AggEdge],
    omitted_nodes: usize,
    omitted_edges: usize,
) -> String {
    let mut out = vec![
        "%% Atlas dependency diagram".to_owned(),
        format!("%% scope: {}", scope_label(request)),
        format!("%% generated: {}", view.data().generated_at),
        format!("%% nodes: {}, edges: {}", labels.len(), edges.len()),
        format!("%% omitted nodes: {omitted_nodes}, omitted edges: {omitted_edges}"),
        "flowchart LR".to_owned(),
    ];
    if labels.is_empty() {
        out.push("%% no nodes matched this scope".to_owned());
        return out.join("\n") + "\n";
    }
    for (index, label) in labels.iter().enumerate() {
        out.push(format!("    n{index}[\"{}\"]", escape_label(label)));
    }
    for edge in edges {
        out.push(format!(
            "    n{} -->|\"{} x{}\"| n{}",
            edge.source, edge.kind, edge.count, edge.target
        ));
    }
    out.join("\n") + "\n"
}

fn render_dot(
    view: &DocsView<'_>,
    request: &ExportRequest,
    labels: &[String],
    edges: &[AggEdge],
    omitted_nodes: usize,
    omitted_edges: usize,
) -> String {
    let mut out = vec![
        "// Atlas dependency diagram".to_owned(),
        format!("// scope: {}", scope_label(request)),
        format!("// generated: {}", view.data().generated_at),
        format!("// nodes: {}, edges: {}", labels.len(), edges.len()),
        format!("// omitted nodes: {omitted_nodes}, omitted edges: {omitted_edges}"),
        "digraph atlas {".to_owned(),
    ];
    if labels.is_empty() {
        out.push("  // no nodes matched this scope".to_owned());
        out.push("}".to_owned());
        return out.join("\n") + "\n";
    }
    for (index, label) in labels.iter().enumerate() {
        out.push(format!("  n{index} [label=\"{}\"];", escape_label(label)));
    }
    for edge in edges {
        out.push(format!(
            "  n{} -> n{} [label=\"{} x{}\"];",
            edge.source, edge.target, edge.kind, edge.count
        ));
    }
    out.push("}".to_owned());
    out.join("\n") + "\n"
}

fn scope_label(request: &ExportRequest) -> String {
    match (&request.scope, request.name.as_deref()) {
        (scope, Some(name)) => format!("{}:{name}", scope.as_str()),
        (scope, None) => scope.as_str().to_owned(),
    }
}

fn escape_label(label: &str) -> String {
    label.replace('"', "\\\"")
}

fn available_list<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let mut names: Vec<&str> = items.collect();
    names.sort();
    names.dedup();
    if names.len() > 10 {
        format!("{}, … ({} total)", names[..10].join(", "), names.len())
    } else {
        names.join(", ")
    }
}

fn component_label_names<'a>(view: &DocsView<'a>) -> impl Iterator<Item = &'a str> + 'a {
    let mut labels: Vec<&str> = view
        .data()
        .component_assignments
        .iter()
        .filter(|assignment| assignment.qualified_name.is_none())
        .flat_map(|assignment| assignment.labels.iter().map(|label| label.label.as_str()))
        .collect();
    labels.sort();
    labels.dedup();
    labels.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DocsData;
    use atlas_core::{Edge, EdgeKind, GraphStats, Node, NodeId, NodeKind};
    use atlas_reasoning::{ComponentLabelAssignment, ComponentLabelMatch, InferredModule};

    fn fixture_data() -> &'static DocsData {
        let data = DocsData {
            repo_root: "/repo".to_owned(),
            stats: GraphStats {
                file_count: 2,
                node_count: 4,
                edge_count: 4,
                nodes_by_kind: vec![("function".to_owned(), 4)],
                languages: vec!["rust".to_owned()],
                last_indexed_at: None,
            },
            files: vec![atlas_core::FileRecord {
                path: "src/a.rs".to_owned(),
                language: Some("rust".to_owned()),
                hash: "hash".to_owned(),
                size: Some(64),
                indexed_at: "2026-01-01T00:00:00Z".to_owned(),
                owner_id: None,
                owner_kind: None,
                owner_root: None,
                owner_manifest_path: None,
                owner_name: None,
                repo_provenance: None,
            }],
            nodes: vec![
                node("a", "src/a.rs"),
                node("b", "src/a.rs"),
                node("c", "src/c.rs"),
                node("d", "src/c.rs"),
            ],
            edges: vec![
                edge("a", "b"),
                edge("a", "c"),
                edge("b", "c"),
                edge("c", "d"),
            ],
            modules: vec![
                module("core", vec!["a", "b"], "src/"),
                module("cli", vec!["c", "d"], "src/"),
            ],
            component_assignments: vec![ComponentLabelAssignment {
                file_path: "src/a.rs".to_owned(),
                qualified_name: None,
                labels: vec![ComponentLabelMatch {
                    label: "cli".to_owned(),
                    confidence: 0.9,
                    evidence: vec![],
                }],
            }],
            duplicate_groups: vec![],
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        Box::leak(Box::new(data))
    }

    fn view() -> DocsView<'static> {
        DocsView::new(fixture_data())
    }

    #[test]
    fn repo_scope_renders_mermaid_module_graph() {
        let result = export_diagram(
            &view(),
            &ExportRequest {
                format: DocsExportFormat::Mermaid,
                scope: DocsExportScope::Repo,
                name: None,
                max_nodes: usize::MAX,
                max_edges: usize::MAX,
            },
        )
        .unwrap();
        assert!(result.content.contains("n0[\"cli\"]"));
        assert!(result.content.contains("n1[\"core\"]"));
        assert!(result.content.contains("n1 -->|\"calls x2\"| n0"));
        assert_eq!(result.omitted_nodes, 0);
        assert_eq!(result.omitted_edges, 0);
    }

    #[test]
    fn file_scope_renders_internal_symbols_and_counts_external() {
        let result = export_diagram(
            &view(),
            &ExportRequest {
                format: DocsExportFormat::Mermaid,
                scope: DocsExportScope::File,
                name: Some("src/a.rs".to_owned()),
                max_nodes: usize::MAX,
                max_edges: usize::MAX,
            },
        )
        .unwrap();
        assert!(result.content.contains("n0[\"a\"]"));
        assert!(result.content.contains("n1[\"b\"]"));
        // a -> c crosses the file boundary and is omitted from the diagram.
        assert!(
            result
                .content
                .contains("%% omitted nodes: 0, omitted edges: 2")
        );
        assert_eq!(result.edge_count, 1);
        assert_eq!(result.omitted_edges, 2);
    }

    #[test]
    fn symbol_scope_renders_neighborhood() {
        let result = export_diagram(
            &view(),
            &ExportRequest {
                format: DocsExportFormat::Mermaid,
                scope: DocsExportScope::Symbol,
                name: Some("c".to_owned()),
                max_nodes: usize::MAX,
                max_edges: usize::MAX,
            },
        )
        .unwrap();
        assert!(result.content.contains("n0[\"a\"]"));
        assert!(result.content.contains("n1[\"b\"]"));
        assert!(result.content.contains("n2[\"c\"]"));
        assert!(result.content.contains("n3[\"d\"]"));
        assert_eq!(result.node_count, 4);
        assert_eq!(result.edge_count, 4);
    }

    #[test]
    fn module_scope_renders_internal_symbol_graph_and_counts_boundary() {
        let result = export_diagram(
            &view(),
            &ExportRequest {
                format: DocsExportFormat::Mermaid,
                scope: DocsExportScope::Module,
                name: Some("core".to_owned()),
                max_nodes: usize::MAX,
                max_edges: usize::MAX,
            },
        )
        .unwrap();
        assert!(result.content.contains("n0[\"a\"]"));
        assert!(result.content.contains("n1[\"b\"]"));
        assert!(result.content.contains("n0 -->|\"calls x1\"| n1"));
        // a -> c and b -> c cross the module boundary and are omitted.
        assert_eq!(result.edge_count, 1);
        assert_eq!(result.omitted_edges, 2);
        assert!(result.content.contains("omitted edges: 2"));
    }

    #[test]
    fn dot_output_is_graphviz_compatible() {
        let result = export_diagram(
            &view(),
            &ExportRequest {
                format: DocsExportFormat::Dot,
                scope: DocsExportScope::Component,
                name: Some("cli".to_owned()),
                max_nodes: usize::MAX,
                max_edges: usize::MAX,
            },
        )
        .unwrap();
        assert!(result.content.starts_with("// Atlas dependency diagram"));
        assert!(result.content.contains("digraph atlas {"));
        assert!(result.content.contains("n0 [label=\"src/a.rs\"];"));
    }

    #[test]
    fn caps_report_omitted_elements() {
        let result = export_diagram(
            &view(),
            &ExportRequest {
                format: DocsExportFormat::Mermaid,
                scope: DocsExportScope::Symbol,
                name: Some("c".to_owned()),
                max_nodes: 2,
                max_edges: 1,
            },
        )
        .unwrap();
        assert_eq!(result.node_count, 2);
        assert_eq!(result.omitted_nodes, 2);
        assert_eq!(result.edge_count, 1);
        assert_eq!(result.omitted_edges, 3);
        assert!(
            result
                .content
                .contains("%% omitted nodes: 2, omitted edges: 3")
        );
    }

    #[test]
    fn unknown_scope_names_error_actionably() {
        let error = export_diagram(
            &view(),
            &ExportRequest {
                format: DocsExportFormat::Mermaid,
                scope: DocsExportScope::Module,
                name: Some("nope".to_owned()),
                max_nodes: usize::MAX,
                max_edges: usize::MAX,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown module 'nope'"));
        assert!(error.to_string().contains("available: cli, core"));
    }

    #[test]
    fn missing_name_for_named_scope_errors() {
        let error = export_diagram(
            &view(),
            &ExportRequest {
                format: DocsExportFormat::Mermaid,
                scope: DocsExportScope::Symbol,
                name: None,
                max_nodes: usize::MAX,
                max_edges: usize::MAX,
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("is required for the symbol scope")
        );
    }

    fn node(name: &str, file: &str) -> Node {
        Node {
            id: NodeId(1),
            kind: NodeKind::Function,
            name: name.to_owned(),
            qualified_name: name.to_owned(),
            file_path: file.to_owned(),
            line_start: 1,
            line_end: 10,
            language: "rust".to_owned(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "hash".to_owned(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    fn edge(src: &str, tgt: &str) -> Edge {
        Edge {
            id: 0,
            kind: EdgeKind::Calls,
            source_qn: src.to_owned(),
            target_qn: tgt.to_owned(),
            file_path: "src/a.rs".to_owned(),
            line: Some(3),
            confidence: 1.0,
            confidence_tier: Some("high".to_owned()),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    fn module(name: &str, owned: Vec<&str>, root: &str) -> InferredModule {
        let node_count = owned.len();
        InferredModule {
            module_id: name.to_owned(),
            display_name: name.to_owned(),
            root_paths: vec![root.to_owned()],
            owned_symbols: owned.into_iter().map(str::to_owned).collect(),
            node_count,
            file_count: 1,
            outbound_dependencies: vec![],
            inbound_dependencies: vec![],
            confidence: 0.9,
            evidence: vec![],
            explicit: false,
        }
    }
}
