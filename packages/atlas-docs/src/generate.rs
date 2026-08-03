//! Deterministic Markdown documentation generation.
//!
//! [`generate_docs`] renders the whole graph into five files: `index.md`,
//! `files.md`, `symbols.md`, `modules.md`, and `components.md`.  Every list
//! is sorted and the timestamp comes from [`DocsData::generated_at`], so
//! output is stable across runs for the same graph.

use std::collections::BTreeMap;

use atlas_core::{Edge, EdgeKind, FileRecord, Node, NodeKind};
use atlas_reasoning::{ComponentLabelMatch, InferredModule};

use crate::model::DocsView;

/// Number of list entries rendered inline before a `+ N more` summary line.
const MAX_INLINE_LIST: usize = 20;
/// Number of top module dependency rows rendered in `index.md`.
const MAX_TOP_DEPENDENCIES: usize = 20;

/// Render the five Markdown documents, keyed by file name.
pub fn generate_docs(view: &DocsView<'_>) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("index.md".to_owned(), render_index(view)),
        ("files.md".to_owned(), render_files(view)),
        ("symbols.md".to_owned(), render_symbols(view)),
        ("modules.md".to_owned(), render_modules(view)),
        ("components.md".to_owned(), render_components(view)),
    ])
}

fn data<'a>(view: &DocsView<'a>) -> &'a crate::model::DocsData {
    view.data()
}

fn preamble(title: &str, view: &DocsView<'_>) -> Vec<String> {
    let d = data(view);
    vec![
        format!("# {title}"),
        String::new(),
        format!("- Repository: `{}`", d.repo_root),
        format!("- Generated: `{}`", d.generated_at),
        String::new(),
    ]
}

fn render_index(view: &DocsView<'_>) -> String {
    let d = data(view);
    let mut out = preamble("Repository Documentation", view);
    out.push(format!("- Files: {}", d.stats.file_count));
    out.push(format!("- Symbols: {}", d.stats.node_count));
    out.push(format!("- Edges: {}", d.stats.edge_count));
    out.push(format!("- Languages: {}", d.stats.languages.join(", ")));
    out.push(String::new());
    out.push("## Contents".to_owned());
    out.push(String::new());
    out.push("- [Files](files.md)".to_owned());
    out.push("- [Symbols](symbols.md)".to_owned());
    out.push("- [Modules](modules.md)".to_owned());
    out.push("- [Components](components.md)".to_owned());
    out.push(String::new());

    out.push("## Symbol counts by kind".to_owned());
    out.push(String::new());
    out.push("| kind | count |".to_owned());
    out.push("| --- | --- |".to_owned());
    for (kind, count) in &d.stats.nodes_by_kind {
        out.push(format!("| {kind} | {count} |"));
    }
    out.push(String::new());

    out.push(format!("## Modules ({})", d.modules.len()));
    out.push(String::new());
    if d.modules.is_empty() {
        out.push("No modules inferred.".to_owned());
    } else {
        out.push("| module | confidence | explicit | files | symbols |".to_owned());
        out.push("| --- | --- | --- | --- | --- |".to_owned());
        for module in &d.modules {
            out.push(format!(
                "| {} | {:.2} | {} | {} | {} |",
                module.display_name,
                module.confidence,
                yes_no(module.explicit),
                module.file_count,
                module.node_count,
            ));
        }
    }
    out.push(String::new());

    let (file_labels, symbol_labels) = component_label_counts(view);
    out.push(format!("## Components ({})", file_labels.len()));
    out.push(String::new());
    if file_labels.is_empty() {
        out.push("No component labels assigned.".to_owned());
    } else {
        out.push("| component | files | symbols |".to_owned());
        out.push("| --- | --- | --- |".to_owned());
        for (label, file_count) in &file_labels {
            out.push(format!(
                "| {label} | {file_count} | {} |",
                symbol_labels.get(label).copied().unwrap_or(0),
            ));
        }
    }
    out.push(String::new());

    let dependencies = module_dependency_rows(view);
    out.push(format!(
        "## Top module dependencies ({})",
        dependencies.len()
    ));
    out.push(String::new());
    if dependencies.is_empty() {
        out.push("No module-to-module dependencies.".to_owned());
    } else {
        out.push("| source | target | edges |".to_owned());
        out.push("| --- | --- | --- |".to_owned());
        for (source, target, count) in dependencies.into_iter().take(MAX_TOP_DEPENDENCIES) {
            out.push(format!("| {source} | {target} | {count} |"));
        }
    }
    out.push(String::new());
    out.join("\n")
}

fn render_files(view: &DocsView<'_>) -> String {
    let mut out = preamble("Files", view);
    for file in &data(view).files {
        render_file_section(view, file, &mut out);
        out.push(String::new());
    }
    out.join("\n")
}

fn render_file_section(view: &DocsView<'_>, file: &FileRecord, out: &mut Vec<String>) {
    let symbols = view
        .nodes_in_file(&file.path)
        .iter()
        .copied()
        .filter(|node| !matches!(node.kind, NodeKind::File))
        .collect::<Vec<_>>();
    let (inbound_count, inbound_files) = file_dependency_counts(view, &file.path, false);
    let (outbound_count, outbound_files) = file_dependency_counts(view, &file.path, true);
    let duplicates = view.duplicates_for_file(&file.path);
    let modules = view.modules_for_file(&file.path);
    let component_labels = view.component_labels_for_file(&file.path);

    out.push(format!("## `{}`", file.path));
    out.push(String::new());
    out.push(format!(
        "- Language: {}",
        file.language.as_deref().unwrap_or("unknown")
    ));
    if let Some(owner) = &file.owner_id {
        out.push(format!(
            "- Package: `{owner}`{}",
            file.owner_name
                .as_deref()
                .map(|name| format!(" ({name})"))
                .unwrap_or_default()
        ));
    }
    if !modules.is_empty() {
        out.push(format!(
            "- Modules: {}",
            modules
                .iter()
                .map(|module| module.display_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !component_labels.is_empty() {
        out.push(format!(
            "- Component labels: {}",
            render_component_labels(component_labels)
        ));
    }
    out.push(format!("- Symbols: {}", symbols.len()));
    out.push(format!(
        "- Inbound dependencies: {inbound_count} (from {inbound_files} file{})",
        plural(inbound_files)
    ));
    out.push(format!(
        "- Outbound dependencies: {outbound_count} (to {outbound_files} file{})",
        plural(outbound_files)
    ));
    if !duplicates.is_empty() {
        out.push(format!(
            "- Notable duplicates: {}",
            duplicates
                .iter()
                .map(|group| {
                    format!(
                        "`{}` ({:.2}, {} members)",
                        group.duplicate_kind, group.confidence, group.member_count
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push(String::new());
    if symbols.is_empty() {
        out.push("No symbols indexed in this file.".to_owned());
    } else {
        out.push("### Symbols".to_owned());
        out.push(String::new());
        for symbol in symbols {
            out.push(format!(
                "- `{}` ({}) lines {}-{}",
                symbol.qualified_name,
                symbol.kind.as_str(),
                symbol.line_start,
                symbol.line_end,
            ));
        }
    }
}

fn render_symbols(view: &DocsView<'_>) -> String {
    let mut out = preamble("Symbols", view);
    for node in view.symbol_nodes() {
        render_symbol_section(view, node, &mut out);
        out.push(String::new());
    }
    out.join("\n")
}

fn render_symbol_section(view: &DocsView<'_>, node: &Node, out: &mut Vec<String>) {
    let modules = view.modules_for_file(&node.file_path);
    let component_labels = view.component_labels_for_symbol(&node.qualified_name);
    let callers = dependency_edges(view, &node.qualified_name, false);
    let callees = dependency_edges(view, &node.qualified_name, true);
    let tests = view.tests_for_symbol(&node.qualified_name);

    out.push(format!("## `{}`", node.qualified_name));
    out.push(String::new());
    out.push(format!("- Kind: {}", node.kind.as_str()));
    out.push(format!("- Language: {}", node.language));
    out.push(format!(
        "- File: `{}`:{}-{}",
        node.file_path, node.line_start, node.line_end
    ));
    if let Some(signature) = symbol_signature(node) {
        out.push(format!("- Signature: `{signature}`"));
    }
    if let Some(modifiers) = node.modifiers.as_deref().filter(|m| !m.is_empty()) {
        out.push(format!("- Modifiers: `{modifiers}`"));
    }
    if node.is_test {
        out.push("- Test symbol: yes".to_owned());
    }
    if let Some(module) = modules.first() {
        out.push(format!("- Module: {}", module.display_name));
    }
    if !component_labels.is_empty() {
        out.push(format!(
            "- Component labels: {}",
            render_component_labels(component_labels)
        ));
    }
    if let Some(snippet) = view.doc_snippet(node) {
        out.push("- Doc snippet:".to_owned());
        out.push(String::new());
        out.push("  > ".to_owned() + &snippet);
        out.push(String::new());
    }

    if !callers.is_empty() {
        out.push(format!("### Callers ({})", callers.len()));
        out.push(String::new());
        push_edge_lines(&callers, true, out);
        out.push(String::new());
    }
    if !callees.is_empty() {
        out.push(format!("### Callees ({})", callees.len()));
        out.push(String::new());
        push_edge_lines(&callees, false, out);
        out.push(String::new());
    }
    if !tests.is_empty() {
        out.push(format!("### Tests ({})", tests.len()));
        out.push(String::new());
        for test in tests.iter().take(MAX_INLINE_LIST) {
            out.push(format!(
                "- `{}` ({}:{})",
                test.qualified_name, test.file_path, test.line_start
            ));
        }
        if tests.len() > MAX_INLINE_LIST {
            out.push(format!("- +{} more", tests.len() - MAX_INLINE_LIST));
        }
    }
}

fn render_modules(view: &DocsView<'_>) -> String {
    let mut out = preamble("Modules", view);
    for module in &data(view).modules {
        render_module_section(view, module, &mut out);
        out.push(String::new());
    }
    out.join("\n")
}

fn render_module_section(view: &DocsView<'_>, module: &InferredModule, out: &mut Vec<String>) {
    let mut files: Vec<&FileRecord> = data(view)
        .files
        .iter()
        .filter(|file| {
            view.modules_for_file(&file.path)
                .iter()
                .any(|candidate| candidate.module_id == module.module_id)
        })
        .collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    out.push(format!("## {}", module.display_name));
    out.push(String::new());
    out.push(format!("- Module ID: `{}`", module.module_id));
    out.push(format!("- Explicit: {}", yes_no(module.explicit)));
    out.push(format!("- Confidence: {:.2}", module.confidence));
    out.push(format!(
        "- Root paths: {}",
        module
            .root_paths
            .iter()
            .map(|path| format!("`{path}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out.push(format!(
        "- Files ({}): {}",
        files.len(),
        push_inline_list(
            &files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>()
        )
    ));
    out.push(format!(
        "- Owned symbols ({}): {}",
        module.node_count,
        push_inline_list(&module.owned_symbols)
    ));
    out.push(String::new());
    out.push(format!(
        "- Inbound dependencies ({}): {}",
        module.inbound_dependencies.len(),
        push_inline_list(&module.inbound_dependencies)
    ));
    out.push(format!(
        "- Outbound dependencies ({}): {}",
        module.outbound_dependencies.len(),
        push_inline_list(&module.outbound_dependencies)
    ));
}

fn render_components(view: &DocsView<'_>) -> String {
    let mut out = preamble("Components", view);
    let d = data(view);

    let mut files_by_label: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut symbols_by_label: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for assignment in &d.component_assignments {
        for label in &assignment.labels {
            match &assignment.qualified_name {
                Some(qname) => symbols_by_label
                    .entry(label.label.clone())
                    .or_default()
                    .push(qname.clone()),
                None => files_by_label
                    .entry(label.label.clone())
                    .or_default()
                    .push(assignment.file_path.clone()),
            }
        }
    }
    for files in files_by_label.values_mut() {
        files.sort();
        files.dedup();
    }
    for symbols in symbols_by_label.values_mut() {
        symbols.sort();
        symbols.dedup();
    }

    for (label, files) in &files_by_label {
        out.push(format!("## {label}"));
        out.push(String::new());
        out.push(format!(
            "- Files ({}): {}",
            files.len(),
            push_inline_list(files)
        ));
        let symbols = symbols_by_label.get(label).cloned().unwrap_or_default();
        out.push(format!(
            "- Symbols ({}): {}",
            symbols.len(),
            push_inline_list(&symbols)
        ));
        out.push(String::new());
    }
    out.join("\n")
}

fn symbol_signature(node: &Node) -> Option<String> {
    let has_params = node.params.as_deref().is_some_and(|p| !p.is_empty());
    let has_return = node.return_type.as_deref().is_some_and(|r| !r.is_empty());
    if !has_params && !has_return {
        return None;
    }
    let params = node.params.as_deref().unwrap_or_default();
    match (has_params, has_return) {
        (true, true) => Some(format!(
            "{}({}) -> {}",
            node.name,
            params,
            node.return_type.as_deref().unwrap_or_default()
        )),
        (true, false) => Some(format!("{}({})", node.name, params)),
        (false, true) => Some(format!(
            "{}() -> {}",
            node.name,
            node.return_type.as_deref().unwrap_or_default()
        )),
        (false, false) => None,
    }
}

/// Aggregate `(count, distinct_file_count)` for cross-file edges touching
/// `path`.  `outbound = true` counts edges sourced by symbols in the file;
/// `false` counts edges targeting symbols in the file.
fn file_dependency_counts(view: &DocsView<'_>, path: &str, outbound: bool) -> (usize, usize) {
    let mut files = std::collections::BTreeSet::new();
    let mut count = 0usize;
    let edges = if outbound {
        view.outbound_edges_for_file(path)
    } else {
        view.inbound_edges_for_file(path)
    };
    for edge in edges {
        let other = if outbound {
            edge.target_qn.as_str()
        } else {
            edge.source_qn.as_str()
        };
        let Some(other_node) = view.node_by_qname(other) else {
            continue;
        };
        if other_node.file_path == path {
            continue;
        }
        count += 1;
        files.insert(other_node.file_path.as_str());
    }
    (count, files.len())
}

/// Caller/callee edges for `qname`, excluding `Contains`.
fn dependency_edges<'a>(view: &DocsView<'a>, qname: &str, outbound: bool) -> Vec<&'a Edge> {
    let edges = if outbound {
        view.outbound_edges(qname)
    } else {
        view.inbound_edges(qname)
    };
    edges
        .into_iter()
        .filter(|edge| !matches!(edge.kind, EdgeKind::Contains))
        .collect()
}

fn push_edge_lines(edges: &[&Edge], inbound: bool, out: &mut Vec<String>) {
    for edge in edges.iter().take(MAX_INLINE_LIST) {
        let other = if inbound {
            edge.source_qn.as_str()
        } else {
            edge.target_qn.as_str()
        };
        out.push(format!(
            "- `{other}` ({}:{}) [{}]",
            edge.file_path,
            edge.line.unwrap_or(0),
            edge.kind.as_str()
        ));
    }
    if edges.len() > MAX_INLINE_LIST {
        out.push(format!("- +{} more", edges.len() - MAX_INLINE_LIST));
    }
}

/// Module-to-module dependency rows `(source, target, edge_count)` sorted
/// deterministically, built from edges between owned symbols.
fn module_dependency_rows(view: &DocsView<'_>) -> Vec<(String, String, usize)> {
    let d = data(view);
    let module_by_symbol: BTreeMap<&str, &str> = d
        .modules
        .iter()
        .flat_map(|module| {
            module
                .owned_symbols
                .iter()
                .map(move |qname| (qname.as_str(), module.display_name.as_str()))
        })
        .collect();

    let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for edge in &d.edges {
        if matches!(edge.kind, EdgeKind::Contains) {
            continue;
        }
        let (Some(source), Some(target)) = (
            module_by_symbol.get(edge.source_qn.as_str()),
            module_by_symbol.get(edge.target_qn.as_str()),
        ) else {
            continue;
        };
        if source == target {
            continue;
        }
        *counts.entry((source, target)).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|((source, target), count)| (source.to_owned(), target.to_owned(), count))
        .collect()
}

fn component_label_counts(
    view: &DocsView<'_>,
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut files: BTreeMap<String, usize> = BTreeMap::new();
    let mut symbols: BTreeMap<String, usize> = BTreeMap::new();
    for assignment in &data(view).component_assignments {
        for label in &assignment.labels {
            let counter = if assignment.qualified_name.is_some() {
                &mut symbols
            } else {
                &mut files
            };
            *counter.entry(label.label.clone()).or_default() += 1;
        }
    }
    (files, symbols)
}

fn render_component_labels(labels: &[&ComponentLabelMatch]) -> String {
    labels
        .iter()
        .map(|label| format!("{} ({:.2})", label.label, label.confidence))
        .collect::<Vec<_>>()
        .join(", ")
}

fn push_inline_list(items: &[String]) -> String {
    let head = items
        .iter()
        .take(MAX_INLINE_LIST)
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > MAX_INLINE_LIST {
        format!("{head}, +{} more", items.len() - MAX_INLINE_LIST)
    } else {
        head
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DocsData, DocsView};
    use atlas_core::{Edge, EdgeKind, GraphStats, Node, NodeId, NodeKind};
    use atlas_reasoning::{ComponentLabelAssignment, ComponentLabelMatch, DuplicateGroup};

    #[test]
    fn signature_rendering_handles_params_and_return() {
        let mut node = test_node("foo", NodeKind::Function);
        assert!(symbol_signature(&node).is_none());
        node.params = Some("a: i32".to_owned());
        node.return_type = Some("bool".to_owned());
        assert_eq!(
            symbol_signature(&node).as_deref(),
            Some("foo(a: i32) -> bool")
        );
    }

    #[test]
    fn dependency_rows_sort_deterministically() {
        let data = DocsData {
            repo_root: "/repo".to_owned(),
            stats: GraphStats {
                file_count: 2,
                node_count: 3,
                edge_count: 2,
                nodes_by_kind: vec![("function".to_owned(), 3)],
                languages: vec!["rust".to_owned()],
                last_indexed_at: None,
            },
            files: vec![
                atlas_core::FileRecord {
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
                },
                atlas_core::FileRecord {
                    path: "src/b.rs".to_owned(),
                    language: Some("rust".to_owned()),
                    hash: "hash".to_owned(),
                    size: Some(32),
                    indexed_at: "2026-01-01T00:00:00Z".to_owned(),
                    owner_id: None,
                    owner_kind: None,
                    owner_root: None,
                    owner_manifest_path: None,
                    owner_name: None,
                    repo_provenance: None,
                },
                atlas_core::FileRecord {
                    path: "src/c.rs".to_owned(),
                    language: Some("rust".to_owned()),
                    hash: "hash".to_owned(),
                    size: Some(32),
                    indexed_at: "2026-01-01T00:00:00Z".to_owned(),
                    owner_id: None,
                    owner_kind: None,
                    owner_root: None,
                    owner_manifest_path: None,
                    owner_name: None,
                    repo_provenance: None,
                },
            ],
            nodes: vec![
                test_node("a", NodeKind::Function),
                test_node("b", NodeKind::Function),
                test_node("c", NodeKind::Function),
            ],
            edges: vec![
                test_edge("a", "b"),
                test_edge("b", "c"),
                test_edge("a", "c"),
            ],
            modules: vec![module("core", vec!["a"]), module("cli", vec!["b", "c"])],
            component_assignments: vec![ComponentLabelAssignment {
                file_path: "src/a.rs".to_owned(),
                qualified_name: None,
                labels: vec![ComponentLabelMatch {
                    label: "cli".to_owned(),
                    confidence: 0.9,
                    evidence: vec![],
                }],
            }],
            duplicate_groups: vec![DuplicateGroup {
                group_id: "g1".to_owned(),
                duplicate_kind: "exact_normalized".to_owned(),
                confidence: 0.8,
                normalized_pattern_summary: "fn".to_owned(),
                duplicated_line_count: 3,
                duplicated_token_count: 12,
                member_count: 2,
                files: vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()],
                members: vec![],
                suggested_extraction_target: None,
            }],
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        let view = DocsView::new(&data);
        let rows = module_dependency_rows(&view);
        assert_eq!(rows, vec![("core".to_owned(), "cli".to_owned(), 2)]);
        let (files, symbols) = component_label_counts(&view);
        assert_eq!(files.get("cli"), Some(&1));
        assert!(symbols.is_empty());
        let docs = generate_docs(&view);
        assert_eq!(docs.len(), 5);
        assert!(docs["index.md"].contains("| core | cli | 2 |"));
        assert!(docs["files.md"].contains("## `src/a.rs`"));
        // Inbound counts include edges recorded in other files (b -> c and
        // a -> c both live in src/a.rs but target src/c.rs).
        assert!(docs["files.md"].contains("- Inbound dependencies: 2 (from 2 files)"));
        assert!(docs["files.md"].contains("- Outbound dependencies: 2 (to 2 files)"));
        assert!(
            docs["files.md"].contains("- Notable duplicates: `exact_normalized` (0.80, 2 members)")
        );
        assert!(docs["symbols.md"].contains("## `a`"));
        assert!(docs["modules.md"].contains("## core"));
        assert!(docs["components.md"].contains("## cli"));
    }

    fn test_node(name: &str, kind: NodeKind) -> Node {
        Node {
            id: NodeId(1),
            kind,
            name: name.to_owned(),
            qualified_name: name.to_owned(),
            file_path: format!("src/{name}.rs"),
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

    fn test_edge(src: &str, tgt: &str) -> Edge {
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

    fn module(name: &str, owned: Vec<&str>) -> InferredModule {
        InferredModule {
            module_id: name.to_owned(),
            display_name: name.to_owned(),
            root_paths: vec![format!("src/{name}")],
            owned_symbols: owned.into_iter().map(str::to_owned).collect(),
            node_count: 1,
            file_count: 1,
            outbound_dependencies: vec![],
            inbound_dependencies: vec![],
            confidence: 0.9,
            evidence: vec![],
            explicit: false,
        }
    }
}
