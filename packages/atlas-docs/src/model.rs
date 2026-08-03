//! In-memory model backing docs generation and diagram export.
//!
//! [`DocsData`] is the raw material loaded from the graph store and insight
//! engines.  [`DocsView`] derives the lookup indexes used by both renderers so
//! rendering code stays free of ad-hoc aggregation.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use atlas_core::{Edge, EdgeKind, FileRecord, GraphStats, Node, NodeKind};
use atlas_reasoning::{
    ComponentLabelAssignment, ComponentLabelMatch, DuplicateGroup, InferredModule,
};

/// Synthetic graph files (multi-repo registry) that are persisted into the
/// store but are not real repository files; docs generation excludes them.
pub const SYNTHETIC_GRAPH_PREFIX: &str = ".atlas/synthetic/";

/// Everything a docs renderer needs, assembled once per run.
#[derive(Debug, Clone)]
pub struct DocsData {
    /// Canonical absolute repository root.
    pub repo_root: String,
    /// Whole-graph statistics from the store.
    pub stats: GraphStats,
    /// All file records (canonical repo-relative paths), sorted by path.
    pub files: Vec<FileRecord>,
    /// All nodes, sorted by `(file_path, line_start, qualified_name)`.
    pub nodes: Vec<Node>,
    /// All edges, sorted by `(source, target, kind, file_path)`.
    pub edges: Vec<Edge>,
    /// Inferred modules (deterministic ordering from the insight engine).
    pub modules: Vec<InferredModule>,
    /// Component label assignments for files and symbols.
    pub component_assignments: Vec<ComponentLabelAssignment>,
    /// Duplicate callable groups (deterministic ordering).
    pub duplicate_groups: Vec<DuplicateGroup>,
    /// RFC 3339 timestamp rendered into every generated document.
    pub generated_at: String,
}

impl DocsData {
    /// True for persisted graph rows that are not real repository files.
    pub fn is_synthetic_path(path: &str) -> bool {
        path.starts_with(SYNTHETIC_GRAPH_PREFIX)
    }
}

/// Derived, borrowed lookup indexes over [`DocsData`].
///
/// All lookups return slices that were built in sorted order, so renderers
/// never need to sort again.
pub struct DocsView<'a> {
    data: &'a DocsData,
    nodes_by_file: BTreeMap<&'a str, Vec<&'a Node>>,
    edges_by_file: BTreeMap<&'a str, Vec<&'a Edge>>,
    edges_by_source: BTreeMap<&'a str, Vec<&'a Edge>>,
    edges_by_target: BTreeMap<&'a str, Vec<&'a Edge>>,
    node_by_qname: BTreeMap<&'a str, &'a Node>,
    modules_by_file: BTreeMap<&'a str, Vec<&'a InferredModule>>,
    file_component_labels: BTreeMap<&'a str, Vec<&'a ComponentLabelMatch>>,
    symbol_component_labels: BTreeMap<&'a str, Vec<&'a ComponentLabelMatch>>,
    duplicates_by_file: BTreeMap<&'a str, Vec<&'a DuplicateGroup>>,
    tests_by_symbol: BTreeMap<&'a str, Vec<&'a Node>>,
    // Cross-file edges grouped by endpoint file (source file for outbound,
    // target file for inbound); `Contains` edges are excluded.
    outbound_edges_by_file: BTreeMap<&'a str, Vec<&'a Edge>>,
    inbound_edges_by_file: BTreeMap<&'a str, Vec<&'a Edge>>,
    source_lines: RefCell<HashMap<String, Option<Vec<String>>>>,
}

impl<'a> DocsView<'a> {
    /// Build the derived indexes.  Indexes are built in sorted key order, so
    /// lookups stay deterministic regardless of input ordering.
    pub fn new(data: &'a DocsData) -> Self {
        let mut nodes_by_file: BTreeMap<&'a str, Vec<&'a Node>> = BTreeMap::new();
        let mut edges_by_file: BTreeMap<&'a str, Vec<&'a Edge>> = BTreeMap::new();
        let mut edges_by_source: BTreeMap<&'a str, Vec<&'a Edge>> = BTreeMap::new();
        let mut edges_by_target: BTreeMap<&'a str, Vec<&'a Edge>> = BTreeMap::new();
        let mut node_by_qname: BTreeMap<&'a str, &'a Node> = BTreeMap::new();
        let mut file_by_qname: BTreeMap<&'a str, &'a str> = BTreeMap::new();

        for node in &data.nodes {
            if DocsData::is_synthetic_path(&node.file_path) {
                continue;
            }
            nodes_by_file
                .entry(node.file_path.as_str())
                .or_default()
                .push(node);
            node_by_qname
                .entry(node.qualified_name.as_str())
                .or_insert(node);
            file_by_qname
                .entry(node.qualified_name.as_str())
                .or_insert(node.file_path.as_str());
        }
        for edge in &data.edges {
            if DocsData::is_synthetic_path(&edge.file_path) {
                continue;
            }
            edges_by_file
                .entry(edge.file_path.as_str())
                .or_default()
                .push(edge);
            edges_by_source
                .entry(edge.source_qn.as_str())
                .or_default()
                .push(edge);
            edges_by_target
                .entry(edge.target_qn.as_str())
                .or_default()
                .push(edge);
        }

        // Files owned by a module: via owned symbol qualified names first, then
        // via root-path prefix matches (same precedence as module inference).
        let mut module_ids_by_file: BTreeMap<&'a str, BTreeSet<&'a str>> = BTreeMap::new();
        for module in &data.modules {
            let mut owned_files: BTreeSet<&'a str> = BTreeSet::new();
            for qname in &module.owned_symbols {
                if let Some(file) = file_by_qname.get(qname.as_str()) {
                    owned_files.insert(*file);
                }
            }
            for root in &module.root_paths {
                for file in nodes_by_file.keys() {
                    if path_under_root(file, root) {
                        owned_files.insert(*file);
                    }
                }
            }
            for file in owned_files {
                module_ids_by_file
                    .entry(file)
                    .or_default()
                    .insert(module.module_id.as_str());
            }
        }
        let modules_by_file: BTreeMap<&'a str, Vec<&'a InferredModule>> = module_ids_by_file
            .into_iter()
            .map(|(file, ids)| {
                let mut modules: Vec<&'a InferredModule> = data
                    .modules
                    .iter()
                    .filter(|module| ids.contains(module.module_id.as_str()))
                    .collect();
                modules.sort_by(|left, right| left.display_name.cmp(&right.display_name));
                (file, modules)
            })
            .collect();

        let mut file_component_labels: BTreeMap<&'a str, Vec<&'a ComponentLabelMatch>> =
            BTreeMap::new();
        let mut symbol_component_labels: BTreeMap<&'a str, Vec<&'a ComponentLabelMatch>> =
            BTreeMap::new();
        for assignment in &data.component_assignments {
            let labels: Vec<&'a ComponentLabelMatch> = assignment.labels.iter().collect();
            let entry = match &assignment.qualified_name {
                Some(qname) => symbol_component_labels.entry(qname.as_str()).or_default(),
                None => file_component_labels
                    .entry(assignment.file_path.as_str())
                    .or_default(),
            };
            entry.extend(labels);
            entry.sort_by(|left, right| {
                right
                    .confidence
                    .partial_cmp(&left.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.label.cmp(&right.label))
            });
        }

        let mut duplicates_by_file: BTreeMap<&'a str, Vec<&'a DuplicateGroup>> = BTreeMap::new();
        for group in &data.duplicate_groups {
            for file in &group.files {
                duplicates_by_file
                    .entry(file.as_str())
                    .or_default()
                    .push(group);
            }
        }
        for groups in duplicates_by_file.values_mut() {
            groups.sort_by(|left, right| left.group_id.cmp(&right.group_id));
        }

        // Mirror `Store::test_neighbors`: every `tests` / `tested_by` edge
        // connects both endpoints, so each endpoint lists the other.
        let mut tests_by_symbol: BTreeMap<&'a str, Vec<&'a Node>> = BTreeMap::new();
        for edge in &data.edges {
            if !matches!(edge.kind, EdgeKind::Tests | EdgeKind::TestedBy) {
                continue;
            }
            let source = node_by_qname.get(edge.source_qn.as_str());
            let target = node_by_qname.get(edge.target_qn.as_str());
            if let Some(source_node) = source
                && let Some(target_node) = target
            {
                tests_by_symbol
                    .entry(edge.source_qn.as_str())
                    .or_default()
                    .push(target_node);
                tests_by_symbol
                    .entry(edge.target_qn.as_str())
                    .or_default()
                    .push(source_node);
            }
        }
        for tests in tests_by_symbol.values_mut() {
            tests.sort_by(|left, right| {
                left.file_path
                    .cmp(&right.file_path)
                    .then_with(|| left.qualified_name.cmp(&right.qualified_name))
            });
        }

        // Cross-file dependency edges, grouped by endpoint file.  Inbound
        // counts must include edges recorded in *other* files (an edge lives
        // in the file where it was found), so both directions are derived
        // from the whole edge set instead of `edges_in_file`.
        let mut outbound_edges_by_file: BTreeMap<&'a str, Vec<&'a Edge>> = BTreeMap::new();
        let mut inbound_edges_by_file: BTreeMap<&'a str, Vec<&'a Edge>> = BTreeMap::new();
        for edge in &data.edges {
            if matches!(edge.kind, EdgeKind::Contains)
                || DocsData::is_synthetic_path(&edge.file_path)
            {
                continue;
            }
            let (Some(source), Some(target)) = (
                node_by_qname.get(edge.source_qn.as_str()),
                node_by_qname.get(edge.target_qn.as_str()),
            ) else {
                continue;
            };
            if source.file_path == target.file_path {
                continue;
            }
            outbound_edges_by_file
                .entry(source.file_path.as_str())
                .or_default()
                .push(edge);
            inbound_edges_by_file
                .entry(target.file_path.as_str())
                .or_default()
                .push(edge);
        }
        for edges in outbound_edges_by_file.values_mut() {
            edges.sort_by(|left, right| {
                left.target_qn
                    .cmp(&right.target_qn)
                    .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
            });
        }
        for edges in inbound_edges_by_file.values_mut() {
            edges.sort_by(|left, right| {
                left.source_qn
                    .cmp(&right.source_qn)
                    .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
            });
        }

        Self {
            data,
            nodes_by_file,
            edges_by_file,
            edges_by_source,
            edges_by_target,
            node_by_qname,
            modules_by_file,
            file_component_labels,
            symbol_component_labels,
            duplicates_by_file,
            tests_by_symbol,
            outbound_edges_by_file,
            inbound_edges_by_file,
            source_lines: RefCell::new(HashMap::new()),
        }
    }

    pub fn data(&self) -> &'a DocsData {
        self.data
    }

    /// Nodes whose kind is not `File` (symbols), in stored sorted order.
    pub fn symbol_nodes(&self) -> impl Iterator<Item = &'a Node> {
        self.data.nodes.iter().filter(|node| {
            !DocsData::is_synthetic_path(&node.file_path) && !matches!(node.kind, NodeKind::File)
        })
    }

    pub fn nodes_in_file(&self, path: &str) -> &[&'a Node] {
        self.nodes_by_file
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn edges_in_file(&self, path: &str) -> &[&'a Edge] {
        self.edges_by_file
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn node_by_qname(&self, qname: &str) -> Option<&'a Node> {
        self.node_by_qname.get(qname).copied()
    }

    /// Edges whose source is `qname` (fan-out), sorted by target then kind.
    pub fn outbound_edges(&self, qname: &str) -> Vec<&'a Edge> {
        self.edges_by_source
            .get(qname)
            .map(|edges| {
                let mut edges = edges.clone();
                edges.sort_by(|left, right| {
                    left.target_qn
                        .cmp(&right.target_qn)
                        .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
                });
                edges
            })
            .unwrap_or_default()
    }

    /// Edges whose target is `qname` (fan-in), sorted by source then kind.
    pub fn inbound_edges(&self, qname: &str) -> Vec<&'a Edge> {
        self.edges_by_target
            .get(qname)
            .map(|edges| {
                let mut edges = edges.clone();
                edges.sort_by(|left, right| {
                    left.source_qn
                        .cmp(&right.source_qn)
                        .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
                });
                edges
            })
            .unwrap_or_default()
    }

    /// Modules that own `path` (by owned symbol file or root-path prefix).
    pub fn modules_for_file(&self, path: &str) -> &[&'a InferredModule] {
        self.modules_by_file
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Component labels assigned to a whole file.
    pub fn component_labels_for_file(&self, path: &str) -> &[&'a ComponentLabelMatch] {
        self.file_component_labels
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Component labels assigned to a symbol.
    pub fn component_labels_for_symbol(&self, qname: &str) -> &[&'a ComponentLabelMatch] {
        self.symbol_component_labels
            .get(qname)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Duplicate groups that include `path`.
    pub fn duplicates_for_file(&self, path: &str) -> &[&'a DuplicateGroup] {
        self.duplicates_by_file
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Cross-file edges sourced by symbols in `path` (fan-out), sorted by
    /// target then kind.  `Contains` and same-file edges are excluded.
    pub fn outbound_edges_for_file(&self, path: &str) -> &[&'a Edge] {
        self.outbound_edges_by_file
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Cross-file edges targeting symbols in `path` (fan-in), sorted by
    /// source then kind.  `Contains` and same-file edges are excluded.
    pub fn inbound_edges_for_file(&self, path: &str) -> &[&'a Edge] {
        self.inbound_edges_by_file
            .get(path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Test nodes associated with `qname` (via `tests` / `tested_by` edges).
    pub fn tests_for_symbol(&self, qname: &str) -> &[&'a Node] {
        self.tests_by_symbol
            .get(qname)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Best-effort doc comment extracted from the source file on disk,
    /// immediately above `node`'s declared line.  `None` when the file is
    /// unreadable or no contiguous comment block precedes the symbol.
    pub fn doc_snippet(&self, node: &Node) -> Option<String> {
        const MAX_SNIPPET_CHARS: usize = 512;
        const MAX_SNIPPET_LINES: usize = 16;

        let mut cache = self.source_lines.borrow_mut();
        let lines = cache.entry(node.file_path.clone()).or_insert_with(|| {
            std::fs::read_to_string(
                std::path::Path::new(&self.data.repo_root).join(&node.file_path),
            )
            .ok()
            .map(|content| content.lines().map(str::to_owned).collect())
        });

        let lines = lines.as_ref()?;
        let start = usize::try_from(node.line_start).ok()?;
        if start == 0 || start > lines.len() {
            return None;
        }

        let mut snippet: Vec<String> = Vec::new();
        let mut cursor = start - 1; // 0-based index of the line above the declaration
        while snippet.len() < MAX_SNIPPET_LINES && cursor > 0 {
            let raw = lines[cursor - 1].trim_start();
            if raw.is_empty() {
                break;
            }
            if !is_comment_line(raw) {
                break;
            }
            let stripped = strip_comment_markers(raw);
            if !stripped.is_empty() {
                snippet.push(stripped);
            }
            cursor -= 1;
        }
        snippet.reverse();
        let text = snippet.join(" ").trim().to_owned();
        if text.is_empty() {
            return None;
        }
        let mut chars = text.chars();
        let truncated: String = chars.by_ref().take(MAX_SNIPPET_CHARS).collect();
        if chars.next().is_some() {
            return Some(format!("{truncated}…"));
        }
        Some(truncated)
    }
}

/// True when `file` equals `root` or lives under it (path-segment boundary).
fn path_under_root(file: &str, root: &str) -> bool {
    file == root
        || file
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('\\'))
}

fn is_comment_line(line: &str) -> bool {
    let line = line.trim_start();
    let markers = ["//", "/*", "*", "#", "\"\"\"", "'''", "--"];
    markers.iter().any(|marker| line.starts_with(marker))
}

fn strip_comment_markers(line: &str) -> String {
    let mut trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("//") {
        trimmed = rest
            .trim_start_matches('/')
            .trim_start_matches('!')
            .trim_start();
    } else {
        for marker in ["/*", "*/", "*", "\"\"\"", "'''", "--", "#"] {
            if let Some(rest) = trimmed.strip_prefix(marker) {
                trimmed = rest.trim_start();
                break;
            }
        }
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::NodeId;

    #[test]
    fn path_under_root_matches_segment_boundaries() {
        assert!(path_under_root("src/lib.rs", "src"));
        assert!(path_under_root("src", "src"));
        assert!(!path_under_root("srcx/lib.rs", "src"));
        assert!(!path_under_root("src", "src/lib.rs"));
    }

    #[test]
    fn comment_markers_round_trip() {
        assert!(is_comment_line("/// docs"));
        assert!(is_comment_line("  # shell docs"));
        assert!(is_comment_line("/* block"));
        assert!(!is_comment_line("fn main() {}"));
        assert_eq!(strip_comment_markers("/// docs"), "docs");
        assert_eq!(strip_comment_markers("*  docs"), "docs");
    }

    #[test]
    fn synthetic_path_detection() {
        assert!(DocsData::is_synthetic_path(
            ".atlas/synthetic/repos/registry.atlas"
        ));
        assert!(!DocsData::is_synthetic_path("src/lib.rs"));
    }

    #[test]
    fn view_indexes_are_sorted_and_filter_synthetic() {
        let data = DocsData {
            repo_root: "/repo".to_owned(),
            stats: GraphStats {
                file_count: 2,
                node_count: 2,
                edge_count: 1,
                nodes_by_kind: vec![("function".to_owned(), 2)],
                languages: vec!["rust".to_owned()],
                last_indexed_at: None,
            },
            files: vec![],
            nodes: vec![
                node(2, "b", "b", "src/b.rs", NodeKind::Function),
                node(1, "a", "a", "src/a.rs", NodeKind::Function),
                node(
                    0,
                    "registry",
                    "registry::x",
                    ".atlas/synthetic/repos/registry.atlas",
                    NodeKind::Package,
                ),
            ],
            edges: vec![edge("a", "b", EdgeKind::Calls, "src/a.rs")],
            modules: vec![],
            component_assignments: vec![],
            duplicate_groups: vec![],
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let view = DocsView::new(&data);
        let symbols: Vec<&str> = view.symbol_nodes().map(|n| n.name.as_str()).collect();
        assert_eq!(symbols, vec!["b", "a"]);
        assert_eq!(view.nodes_in_file("src/a.rs").len(), 1);
        assert_eq!(view.outbound_edges("a").len(), 1);
        assert!(view.inbound_edges("a").is_empty());
        assert_eq!(view.inbound_edges("b").len(), 1);
        assert!(view.modules_for_file("src/a.rs").is_empty());
    }

    #[test]
    fn view_derives_cross_file_dependency_indexes() {
        let data = DocsData {
            repo_root: "/repo".to_owned(),
            stats: GraphStats {
                file_count: 2,
                node_count: 2,
                edge_count: 2,
                nodes_by_kind: vec![("function".to_owned(), 2)],
                languages: vec!["rust".to_owned()],
                last_indexed_at: None,
            },
            files: vec![],
            nodes: vec![
                node(1, "a", "a", "src/a.rs", NodeKind::Function),
                node(2, "b", "b", "src/b.rs", NodeKind::Function),
            ],
            edges: vec![
                edge("a", "b", EdgeKind::Calls, "src/a.rs"),
                edge("b", "a", EdgeKind::Imports, "src/b.rs"),
            ],
            modules: vec![],
            component_assignments: vec![],
            duplicate_groups: vec![],
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let view = DocsView::new(&data);
        // Inbound for `src/a.rs` must include the edge recorded in `src/b.rs`.
        let outbound_a = view.outbound_edges_for_file("src/a.rs");
        assert_eq!(outbound_a.len(), 1);
        assert_eq!(outbound_a[0].target_qn, "b");
        let inbound_a = view.inbound_edges_for_file("src/a.rs");
        assert_eq!(inbound_a.len(), 1);
        assert_eq!(inbound_a[0].source_qn, "b");
        let inbound_b = view.inbound_edges_for_file("src/b.rs");
        assert_eq!(inbound_b.len(), 1);
        assert_eq!(inbound_b[0].source_qn, "a");
        let outbound_b = view.outbound_edges_for_file("src/b.rs");
        assert_eq!(outbound_b.len(), 1);
        assert_eq!(outbound_b[0].target_qn, "a");
    }

    #[test]
    fn view_derives_doc_snippet_from_source_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "//! Crate docs\n\n/// Adds numbers.\n///\n/// Keeps it simple.\npub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .unwrap();
        let mut data = DocsData {
            repo_root: dir.path().to_str().unwrap().to_owned(),
            stats: GraphStats {
                file_count: 1,
                node_count: 1,
                edge_count: 0,
                nodes_by_kind: vec![("function".to_owned(), 1)],
                languages: vec!["rust".to_owned()],
                last_indexed_at: None,
            },
            files: vec![],
            nodes: vec![node(1, "add", "add", "src/lib.rs", NodeKind::Function)],
            edges: vec![],
            modules: vec![],
            component_assignments: vec![],
            duplicate_groups: vec![],
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        data.nodes[0].line_start = 6;
        let view = DocsView::new(&data);
        let snippet = view.doc_snippet(&data.nodes[0]).expect("snippet");
        assert_eq!(snippet, "Adds numbers. Keeps it simple.");
    }

    #[test]
    fn missing_source_file_yields_no_snippet() {
        let data = DocsData {
            repo_root: "/does/not/exist".to_owned(),
            stats: GraphStats {
                file_count: 1,
                node_count: 1,
                edge_count: 0,
                nodes_by_kind: vec![("function".to_owned(), 1)],
                languages: vec!["rust".to_owned()],
                last_indexed_at: None,
            },
            files: vec![],
            nodes: vec![node(1, "add", "add", "src/lib.rs", NodeKind::Function)],
            edges: vec![],
            modules: vec![],
            component_assignments: vec![],
            duplicate_groups: vec![],
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let view = DocsView::new(&data);
        assert!(view.doc_snippet(&data.nodes[0]).is_none());
    }

    fn node(id: i64, name: &str, qname: &str, file: &str, kind: NodeKind) -> Node {
        Node {
            id: NodeId(id),
            kind,
            name: name.to_owned(),
            qualified_name: qname.to_owned(),
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

    fn edge(src: &str, tgt: &str, kind: EdgeKind, file: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: src.to_owned(),
            target_qn: tgt.to_owned(),
            file_path: file.to_owned(),
            line: None,
            confidence: 1.0,
            confidence_tier: Some("high".to_owned()),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }
}
