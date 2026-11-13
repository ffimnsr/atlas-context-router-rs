//! ICM-G — shared overview graph/path helpers.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use atlas_core::model::{Edge, FileRecord, Node};
use atlas_core::{AtlasError, Result};
use atlas_repo::{CanonicalRepoPath, head_ref};
use atlas_session::{
    OverviewGraphProvenance, OverviewPacketFreshness, OverviewSubject, OverviewSubjectKind,
};
use atlas_store_sqlite::{GraphBuildState, Store};
use camino::Utf8Path;

/// Canonicalize a repo-relative path through the shared identity helper.
pub fn canonical_path(repo_root: &str, path: &str) -> Result<String> {
    CanonicalRepoPath::from_cli_argument(Utf8Path::new(repo_root), Utf8Path::new(path))
        .map(|path| path.as_str().to_owned())
        .map_err(|error| AtlasError::Other(format!("invalid overview path: {error}")))
}

pub fn current_commit_sha(repo_root: &str) -> String {
    head_ref(Utf8Path::new(repo_root)).unwrap_or_else(|| "no-head".to_owned())
}

pub fn graph_provenance(
    store: &Store,
    db_path: &str,
    repo_root: &str,
) -> Result<OverviewGraphProvenance> {
    let status = store.get_build_status(repo_root)?;
    let stats = store.graph_stats()?;
    let last_indexed_ref = store.last_indexed_ref(repo_root)?;
    Ok(OverviewGraphProvenance {
        graph_db_path: db_path.to_owned(),
        graph_state: match status.map(|status| status.state) {
            Some(GraphBuildState::Building) => "building".to_owned(),
            Some(GraphBuildState::Built) => "built".to_owned(),
            Some(GraphBuildState::Degraded) => "degraded".to_owned(),
            Some(GraphBuildState::BuildFailed) => "build_failed".to_owned(),
            None => "unknown".to_owned(),
        },
        last_indexed_at: stats.last_indexed_at,
        last_indexed_ref,
        indexed_file_count: stats.file_count.max(0) as usize,
    })
}

pub fn ensure_graph_not_corrupt(graph: &OverviewGraphProvenance) -> Result<()> {
    let state = graph.graph_state.to_ascii_lowercase();
    if state.contains("corrupt") {
        return Err(AtlasError::Other(
            "overview export blocked: graph readiness is corrupt; rebuild the graph first"
                .to_owned(),
        ));
    }
    Ok(())
}

pub fn graph_freshness(
    graph: &OverviewGraphProvenance,
    commit_sha: &str,
) -> OverviewPacketFreshness {
    let indexed = graph.last_indexed_ref.as_deref();
    match indexed {
        Some(indexed) if indexed == commit_sha => OverviewPacketFreshness {
            state: "fresh".to_owned(),
            stale_reason: None,
            safe_to_answer: true,
        },
        Some(indexed) => OverviewPacketFreshness {
            state: "stale".to_owned(),
            stale_reason: Some(format!(
                "graph indexed at {indexed} but repo HEAD is {commit_sha}"
            )),
            safe_to_answer: false,
        },
        None => OverviewPacketFreshness {
            state: "unverified".to_owned(),
            stale_reason: Some("graph has no recorded indexed ref".to_owned()),
            safe_to_answer: false,
        },
    }
}

/// Package name for a node, preferring graph owner metadata.
pub fn node_package(node: &Node) -> Option<String> {
    node.extra_json
        .get("owner_name")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            node.extra_json
                .get("owner_root")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
        .map(str::to_owned)
}

pub fn file_package(file: &FileRecord) -> Option<String> {
    file.owner_name
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            file.owner_root
                .clone()
                .filter(|value| !value.trim().is_empty())
        })
}

/// Module path is the containing directory with `/` separators.
pub fn module_path_for_file(file_path: &str) -> String {
    file_path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or(".")
        .to_owned()
}

pub fn subject_kind_for_node(node: &Node) -> OverviewSubjectKind {
    match node.kind {
        atlas_core::NodeKind::Function => OverviewSubjectKind::Function,
        atlas_core::NodeKind::Method => OverviewSubjectKind::Method,
        _ => OverviewSubjectKind::Symbol,
    }
}

pub fn node_subject(node: &Node, repo_root: &str, commit_sha: &str) -> OverviewSubject {
    OverviewSubject {
        kind: subject_kind_for_node(node),
        repo_root: repo_root.to_owned(),
        commit_sha: commit_sha.to_owned(),
        package_name: node_package(node),
        module_path: Some(module_path_for_file(&node.file_path)),
        canonical_file_path: Some(node.file_path.clone()),
        qualified_name: Some(node.qualified_name.clone()),
        node_kind: Some(node.kind.as_str().to_owned()),
        line_start: Some(node.line_start),
        line_end: Some(node.line_end),
        content_hash: Some(node.file_hash.clone()).filter(|hash| !hash.is_empty()),
    }
}

pub fn file_subject(file: &FileRecord, repo_root: &str, commit_sha: &str) -> OverviewSubject {
    OverviewSubject {
        kind: OverviewSubjectKind::File,
        repo_root: repo_root.to_owned(),
        commit_sha: commit_sha.to_owned(),
        package_name: file_package(file),
        module_path: Some(module_path_for_file(&file.path)),
        canonical_file_path: Some(file.path.clone()),
        qualified_name: None,
        node_kind: Some("file".to_owned()),
        line_start: None,
        line_end: None,
        content_hash: Some(file.hash.clone()).filter(|hash| !hash.is_empty()),
    }
}

pub fn project_subject(repo_root: &str, commit_sha: &str) -> OverviewSubject {
    OverviewSubject {
        kind: OverviewSubjectKind::Project,
        repo_root: repo_root.to_owned(),
        commit_sha: commit_sha.to_owned(),
        package_name: None,
        module_path: None,
        canonical_file_path: None,
        qualified_name: None,
        node_kind: None,
        line_start: None,
        line_end: None,
        content_hash: None,
    }
}

pub fn package_subject(package: &str, repo_root: &str, commit_sha: &str) -> OverviewSubject {
    OverviewSubject {
        kind: OverviewSubjectKind::Package,
        repo_root: repo_root.to_owned(),
        commit_sha: commit_sha.to_owned(),
        package_name: Some(package.to_owned()),
        module_path: None,
        canonical_file_path: None,
        qualified_name: None,
        node_kind: None,
        line_start: None,
        line_end: None,
        content_hash: None,
    }
}

pub fn module_subject(module: &str, repo_root: &str, commit_sha: &str) -> OverviewSubject {
    OverviewSubject {
        kind: OverviewSubjectKind::Module,
        repo_root: repo_root.to_owned(),
        commit_sha: commit_sha.to_owned(),
        package_name: None,
        module_path: Some(module.to_owned()),
        canonical_file_path: None,
        qualified_name: None,
        node_kind: None,
        line_start: None,
        line_end: None,
        content_hash: None,
    }
}

/// Index nodes by qualified name and file path.
pub fn index_nodes(nodes: &[Node]) -> (HashMap<String, usize>, HashMap<String, Vec<usize>>) {
    let mut by_qname = HashMap::new();
    let mut by_file: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        by_qname.insert(node.qualified_name.clone(), index);
        by_file
            .entry(node.file_path.clone())
            .or_default()
            .push(index);
    }
    (by_qname, by_file)
}

pub fn matching_nodes<'a>(
    subject: &OverviewSubject,
    nodes: &'a [Node],
    by_file: &HashMap<String, Vec<usize>>,
) -> Vec<&'a Node> {
    match subject.kind {
        OverviewSubjectKind::Project => nodes.iter().collect(),
        OverviewSubjectKind::Package => {
            let package = subject.package_name.as_deref().unwrap_or_default();
            nodes
                .iter()
                .filter(|node| node_package(node).as_deref() == Some(package))
                .collect()
        }
        OverviewSubjectKind::Module => {
            let module = subject.module_path.as_deref().unwrap_or_default();
            nodes
                .iter()
                .filter(|node| module_path_for_file(&node.file_path) == module)
                .collect()
        }
        OverviewSubjectKind::File => subject
            .canonical_file_path
            .as_deref()
            .and_then(|path| by_file.get(path))
            .map(|indexes| indexes.iter().map(|index| &nodes[*index]).collect())
            .unwrap_or_default(),
        OverviewSubjectKind::Symbol
        | OverviewSubjectKind::Function
        | OverviewSubjectKind::Method => {
            let qname = subject.qualified_name.as_deref().unwrap_or_default();
            let file = subject.canonical_file_path.as_deref().unwrap_or_default();
            nodes
                .iter()
                .filter(|node| node.qualified_name == qname && node.file_path == file)
                .collect()
        }
    }
}

pub fn edge_callers(edges: &[Edge], qnames: &BTreeSet<&str>) -> Vec<String> {
    let mut callers = BTreeSet::new();
    for edge in edges {
        if qnames.contains(edge.target_qn.as_str()) {
            callers.insert(edge.source_qn.clone());
        }
    }
    callers.into_iter().collect()
}

pub fn edge_callees(edges: &[Edge], qnames: &BTreeSet<&str>) -> Vec<String> {
    let mut callees = BTreeSet::new();
    for edge in edges {
        if qnames.contains(edge.source_qn.as_str()) {
            callees.insert(edge.target_qn.clone());
        }
    }
    callees.into_iter().collect()
}

pub fn distinct_files(nodes: &[&Node], limit: usize) -> Vec<String> {
    let mut files = BTreeSet::new();
    for node in nodes {
        files.insert(node.file_path.clone());
        if files.len() >= limit {
            break;
        }
    }
    files.into_iter().collect()
}

pub fn symbol_names(nodes: &[&Node], limit: usize) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| !matches!(node.kind, atlas_core::NodeKind::File))
        .map(|node| node.qualified_name.clone())
        .take(limit)
        .collect()
}

pub fn tests(nodes: &[&Node], limit: usize) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| node.is_test || node.file_path.to_ascii_lowercase().contains("test"))
        .map(|node| node.qualified_name.clone())
        .take(limit)
        .collect()
}

pub fn read_excerpt(
    repo_root: &str,
    path: &str,
    line_start: u32,
    line_end: u32,
    max_bytes: usize,
) -> Option<String> {
    let full = Utf8Path::new(repo_root).join(path);
    let text = std::fs::read_to_string(full).ok()?;
    let mut excerpt = String::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index as u32 + 1;
        if line_no < line_start {
            continue;
        }
        if line_no > line_end {
            break;
        }
        if excerpt.len() + line.len() + 1 > max_bytes {
            break;
        }
        excerpt.push_str(line);
        excerpt.push('\n');
    }
    (!excerpt.is_empty()).then_some(excerpt)
}

/// Stable per-record file hash for freshness checks.
pub fn file_content_hash(repo_root: &str, path: &str) -> Option<String> {
    atlas_repo::hash_file(&Utf8Path::new(repo_root).join(path)).ok()
}

/// Group package labels found in files and nodes.
pub fn collect_packages(files: &[FileRecord], nodes: &[Node]) -> Vec<String> {
    let mut packages = BTreeMap::new();
    for file in files {
        if let Some(package) = file_package(file) {
            packages.entry(package).or_insert(file.path.clone());
        }
    }
    for node in nodes {
        if let Some(package) = node_package(node) {
            packages.entry(package).or_insert(node.file_path.clone());
        }
    }
    packages.into_keys().collect()
}

pub fn collect_modules(files: &[FileRecord]) -> Vec<String> {
    files
        .iter()
        .map(|file| module_path_for_file(&file.path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
