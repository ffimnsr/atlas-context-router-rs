use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use atlas_store_sqlite::{
    HistoricalEdge, HistoricalNode, Store, StoredSnapshot, StoredSnapshotFile,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::git;
use crate::reports::{HistoryEvidence, build_evidence, edge_identifier_for, node_identifier_for};

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalSnapshotSummary {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub node_count: i64,
    pub edge_count: i64,
    pub file_count: i64,
    pub completeness: f64,
    pub parse_error_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalSnapshot {
    pub summary: HistoricalSnapshotSummary,
    pub evidence: HistoryEvidence,
    pub repo_id: i64,
    pub commit_sha: String,
    pub root_tree_hash: Option<String>,
    pub node_count: i64,
    pub edge_count: i64,
    pub file_count: i64,
    pub completeness: f64,
    pub parse_error_count: i64,
    pub files: Vec<StoredSnapshotFile>,
    pub nodes: Vec<HistoricalNode>,
    pub edges: Vec<HistoricalEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRename {
    pub old_path: String,
    pub new_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileChange {
    pub file_path: String,
    pub old_hash: Option<String>,
    pub new_hash: Option<String>,
    pub old_language: Option<String>,
    pub new_language: Option<String>,
    pub old_size: Option<i64>,
    pub new_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeDiffEntry {
    pub qualified_name: String,
    pub file_path: String,
    pub kind: String,
    pub line_start_before: Option<i64>,
    pub line_end_before: Option<i64>,
    pub line_start_after: Option<i64>,
    pub line_end_after: Option<i64>,
    pub signature_hash_before: Option<String>,
    pub signature_hash_after: Option<String>,
    pub modifiers_before: Option<String>,
    pub modifiers_after: Option<String>,
    pub is_test_before: Option<bool>,
    pub is_test_after: Option<bool>,
    pub extra_hash_before: Option<String>,
    pub extra_hash_after: Option<String>,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeDiffEntry {
    pub source_qn: String,
    pub target_qn: String,
    pub kind: String,
    pub file_path: String,
    pub confidence_tier_before: Option<String>,
    pub confidence_tier_after: Option<String>,
    pub metadata_hash_before: Option<String>,
    pub metadata_hash_after: Option<String>,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleDiffEntry {
    pub module: String,
    pub node_count_before: usize,
    pub node_count_after: usize,
    pub edge_count_before: usize,
    pub edge_count_after: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleDependencyChange {
    pub source_module: String,
    pub target_module: String,
    pub count_before: usize,
    pub count_after: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct HubChange {
    pub module: String,
    pub degree_before: usize,
    pub degree_after: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchitectureDiff {
    pub new_dependency_paths: Vec<(String, String)>,
    pub removed_dependency_paths: Vec<(String, String)>,
    pub new_cycles: Vec<Vec<String>>,
    pub broken_cycles: Vec<Vec<String>>,
    pub changed_central_hubs: Vec<HubChange>,
    pub changed_coupling: Vec<ModuleDependencyChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDiffSummary {
    pub added_file_count: usize,
    pub removed_file_count: usize,
    pub modified_file_count: usize,
    pub renamed_file_count: usize,
    pub added_node_count: usize,
    pub removed_node_count: usize,
    pub changed_node_count: usize,
    pub added_edge_count: usize,
    pub removed_edge_count: usize,
    pub changed_edge_count: usize,
    pub module_change_count: usize,
    pub new_cycle_count: usize,
    pub broken_cycle_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDiffReport {
    pub summary: GraphDiffSummary,
    pub evidence: HistoryEvidence,
    pub commit_a: String,
    pub commit_b: String,
    pub snapshot_a: HistoricalSnapshot,
    pub snapshot_b: HistoricalSnapshot,
    pub added_files: Vec<StoredSnapshotFile>,
    pub removed_files: Vec<StoredSnapshotFile>,
    pub modified_files: Vec<FileChange>,
    pub renamed_files: Vec<FileRename>,
    pub added_nodes: Vec<NodeDiffEntry>,
    pub removed_nodes: Vec<NodeDiffEntry>,
    pub changed_nodes: Vec<NodeDiffEntry>,
    pub added_edges: Vec<EdgeDiffEntry>,
    pub removed_edges: Vec<EdgeDiffEntry>,
    pub changed_edges: Vec<EdgeDiffEntry>,
    pub module_changes: Vec<ModuleDiffEntry>,
    pub architecture: ArchitectureDiff,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct NodeKey {
    qualified_name: String,
    file_path: String,
    kind: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct EdgeKey {
    source_qn: String,
    target_qn: String,
    kind: String,
    file_path: String,
}

pub fn reconstruct_snapshot(
    store: &Store,
    canonical_root: &str,
    commit_sha: &str,
) -> Result<HistoricalSnapshot> {
    let repo_id = store
        .find_repo_id(canonical_root)?
        .ok_or_else(|| anyhow::anyhow!("repo not yet registered for history: {canonical_root}"))?;
    let snapshot = store
        .find_snapshot(repo_id, commit_sha)?
        .ok_or_else(|| anyhow::anyhow!("indexed snapshot not found for commit {commit_sha}"))?;
    reconstruct_snapshot_from_row(store, snapshot)
}

pub fn diff_snapshots(
    repo: &Path,
    store: &Store,
    canonical_root: &str,
    commit_a: &str,
    commit_b: &str,
) -> Result<GraphDiffReport> {
    let snapshot_a = reconstruct_snapshot(store, canonical_root, commit_a)?;
    let snapshot_b = reconstruct_snapshot(store, canonical_root, commit_b)?;

    let files_a = snapshot_a
        .files
        .iter()
        .map(|file| (file.file_path.clone(), file.clone()))
        .collect::<BTreeMap<_, _>>();
    let files_b = snapshot_b
        .files
        .iter()
        .map(|file| (file.file_path.clone(), file.clone()))
        .collect::<BTreeMap<_, _>>();

    let added_files = files_b
        .iter()
        .filter(|(path, _)| !files_a.contains_key(*path))
        .map(|(_, file)| file.clone())
        .collect::<Vec<_>>();
    let removed_files = files_a
        .iter()
        .filter(|(path, _)| !files_b.contains_key(*path))
        .map(|(_, file)| file.clone())
        .collect::<Vec<_>>();
    let modified_files = files_a
        .iter()
        .filter_map(|(path, before)| {
            let after = files_b.get(path)?;
            if before.file_hash == after.file_hash
                && before.language == after.language
                && before.size == after.size
            {
                return None;
            }
            Some(FileChange {
                file_path: path.clone(),
                old_hash: Some(before.file_hash.clone()),
                new_hash: Some(after.file_hash.clone()),
                old_language: before.language.clone(),
                new_language: after.language.clone(),
                old_size: before.size,
                new_size: after.size,
            })
        })
        .collect::<Vec<_>>();

    let renamed_files = git::diff_tree_files(repo, commit_b, Some(commit_a))
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, _, status)| *status == 'R')
        .map(|(old_path, new_path, _)| FileRename { old_path, new_path })
        .collect::<Vec<_>>();

    let nodes_a = snapshot_a
        .nodes
        .iter()
        .map(|node| {
            (
                NodeKey {
                    qualified_name: node.qualified_name.clone(),
                    file_path: node.file_path.clone(),
                    kind: node.kind.clone(),
                },
                node,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let nodes_b = snapshot_b
        .nodes
        .iter()
        .map(|node| {
            (
                NodeKey {
                    qualified_name: node.qualified_name.clone(),
                    file_path: node.file_path.clone(),
                    kind: node.kind.clone(),
                },
                node,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let added_nodes = nodes_b
        .iter()
        .filter(|(key, _)| !nodes_a.contains_key(*key))
        .map(|(_, node)| node_added(node))
        .collect::<Vec<_>>();
    let removed_nodes = nodes_a
        .iter()
        .filter(|(key, _)| !nodes_b.contains_key(*key))
        .map(|(_, node)| node_removed(node))
        .collect::<Vec<_>>();
    let changed_nodes = nodes_a
        .iter()
        .filter_map(|(key, before)| {
            let after = nodes_b.get(key)?;
            let diff = node_changed(before, after);
            if diff.changed_fields.is_empty() {
                None
            } else {
                Some(diff)
            }
        })
        .collect::<Vec<_>>();

    let edges_a = snapshot_a
        .edges
        .iter()
        .map(|edge| {
            (
                EdgeKey {
                    source_qn: edge.source_qn.clone(),
                    target_qn: edge.target_qn.clone(),
                    kind: edge.kind.clone(),
                    file_path: edge.file_path.clone(),
                },
                edge,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let edges_b = snapshot_b
        .edges
        .iter()
        .map(|edge| {
            (
                EdgeKey {
                    source_qn: edge.source_qn.clone(),
                    target_qn: edge.target_qn.clone(),
                    kind: edge.kind.clone(),
                    file_path: edge.file_path.clone(),
                },
                edge,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let added_edges = edges_b
        .iter()
        .filter(|(key, _)| !edges_a.contains_key(*key))
        .map(|(_, edge)| edge_added(edge))
        .collect::<Vec<_>>();
    let removed_edges = edges_a
        .iter()
        .filter(|(key, _)| !edges_b.contains_key(*key))
        .map(|(_, edge)| edge_removed(edge))
        .collect::<Vec<_>>();
    let changed_edges = edges_a
        .iter()
        .filter_map(|(key, before)| {
            let after = edges_b.get(key)?;
            let diff = edge_changed(before, after);
            if diff.changed_fields.is_empty() {
                None
            } else {
                Some(diff)
            }
        })
        .collect::<Vec<_>>();

    let module_counts_a = module_counts(&snapshot_a);
    let module_counts_b = module_counts(&snapshot_b);
    let mut module_names = module_counts_a.keys().cloned().collect::<BTreeSet<_>>();
    module_names.extend(module_counts_b.keys().cloned());
    let module_changes = module_names
        .into_iter()
        .filter_map(|module| {
            let before = module_counts_a.get(&module).copied().unwrap_or((0, 0));
            let after = module_counts_b.get(&module).copied().unwrap_or((0, 0));
            if before == after {
                return None;
            }
            Some(ModuleDiffEntry {
                module,
                node_count_before: before.0,
                node_count_after: after.0,
                edge_count_before: before.1,
                edge_count_after: after.1,
            })
        })
        .collect::<Vec<_>>();

    let architecture = architecture_diff(&snapshot_a, &snapshot_b);
    let evidence = build_evidence(
        [
            snapshot_a.summary.snapshot_id,
            snapshot_b.summary.snapshot_id,
        ],
        [commit_a.to_owned(), commit_b.to_owned()],
        snapshot_a
            .nodes
            .iter()
            .map(node_identifier_for)
            .chain(snapshot_b.nodes.iter().map(node_identifier_for)),
        snapshot_a
            .edges
            .iter()
            .map(edge_identifier_for)
            .chain(snapshot_b.edges.iter().map(edge_identifier_for)),
        snapshot_a
            .files
            .iter()
            .map(|file| file.file_path.clone())
            .chain(snapshot_b.files.iter().map(|file| file.file_path.clone())),
    );

    Ok(GraphDiffReport {
        summary: GraphDiffSummary {
            added_file_count: added_files.len(),
            removed_file_count: removed_files.len(),
            modified_file_count: modified_files.len(),
            renamed_file_count: renamed_files.len(),
            added_node_count: added_nodes.len(),
            removed_node_count: removed_nodes.len(),
            changed_node_count: changed_nodes.len(),
            added_edge_count: added_edges.len(),
            removed_edge_count: removed_edges.len(),
            changed_edge_count: changed_edges.len(),
            module_change_count: module_changes.len(),
            new_cycle_count: architecture.new_cycles.len(),
            broken_cycle_count: architecture.broken_cycles.len(),
        },
        evidence,
        commit_a: commit_a.to_owned(),
        commit_b: commit_b.to_owned(),
        snapshot_a,
        snapshot_b,
        added_files,
        removed_files,
        modified_files,
        renamed_files,
        added_nodes,
        removed_nodes,
        changed_nodes,
        added_edges,
        removed_edges,
        changed_edges,
        module_changes,
        architecture,
    })
}

fn reconstruct_snapshot_from_row(
    store: &Store,
    snapshot: StoredSnapshot,
) -> Result<HistoricalSnapshot> {
    let files = store
        .list_snapshot_files(snapshot.snapshot_id)
        .context("list snapshot files")?;
    let nodes = store
        .reconstruct_snapshot_nodes(snapshot.snapshot_id)
        .context("reconstruct snapshot nodes")?;
    let edges = store
        .reconstruct_snapshot_edges(snapshot.snapshot_id)
        .context("reconstruct snapshot edges")?;
    validate_snapshot_materialization(&snapshot, &files, &nodes, &edges)?;
    let evidence = build_evidence(
        [snapshot.snapshot_id],
        [snapshot.commit_sha.clone()],
        nodes.iter().map(node_identifier_for),
        edges.iter().map(edge_identifier_for),
        files.iter().map(|file| file.file_path.clone()),
    );
    Ok(HistoricalSnapshot {
        summary: HistoricalSnapshotSummary {
            snapshot_id: snapshot.snapshot_id,
            commit_sha: snapshot.commit_sha.clone(),
            node_count: snapshot.node_count,
            edge_count: snapshot.edge_count,
            file_count: snapshot.file_count,
            completeness: snapshot.completeness,
            parse_error_count: snapshot.parse_error_count,
        },
        evidence,
        repo_id: snapshot.repo_id,
        commit_sha: snapshot.commit_sha,
        root_tree_hash: snapshot.root_tree_hash,
        node_count: snapshot.node_count,
        edge_count: snapshot.edge_count,
        file_count: snapshot.file_count,
        completeness: snapshot.completeness,
        parse_error_count: snapshot.parse_error_count,
        files,
        nodes,
        edges,
    })
}

pub(crate) fn validate_snapshot_materialization(
    snapshot: &StoredSnapshot,
    files: &[StoredSnapshotFile],
    nodes: &[HistoricalNode],
    edges: &[HistoricalEdge],
) -> Result<()> {
    let mut mismatches = Vec::new();

    if snapshot.file_count != files.len() as i64 {
        mismatches.push(format!(
            "files expected={} actual={}",
            snapshot.file_count,
            files.len()
        ));
    }
    if snapshot.node_count != nodes.len() as i64 {
        mismatches.push(format!(
            "nodes expected={} actual={}",
            snapshot.node_count,
            nodes.len()
        ));
    }
    if snapshot.edge_count != edges.len() as i64 {
        mismatches.push(format!(
            "edges expected={} actual={}",
            snapshot.edge_count,
            edges.len()
        ));
    }

    if mismatches.is_empty() {
        return Ok(());
    }

    anyhow::bail!(
        "snapshot membership rows appear corrupted for commit {} (snapshot_id={}): {}",
        snapshot.commit_sha,
        snapshot.snapshot_id,
        mismatches.join(", ")
    )
}

fn node_added(node: &HistoricalNode) -> NodeDiffEntry {
    NodeDiffEntry {
        qualified_name: node.qualified_name.clone(),
        file_path: node.file_path.clone(),
        kind: node.kind.clone(),
        line_start_before: None,
        line_end_before: None,
        line_start_after: node.line_start,
        line_end_after: node.line_end,
        signature_hash_before: None,
        signature_hash_after: node_signature_hash(node),
        modifiers_before: None,
        modifiers_after: node.modifiers.clone(),
        is_test_before: None,
        is_test_after: Some(node.is_test),
        extra_hash_before: None,
        extra_hash_after: stable_hash(node.extra_json.as_deref()),
        changed_fields: vec!["added".to_owned()],
    }
}

fn node_removed(node: &HistoricalNode) -> NodeDiffEntry {
    NodeDiffEntry {
        qualified_name: node.qualified_name.clone(),
        file_path: node.file_path.clone(),
        kind: node.kind.clone(),
        line_start_before: node.line_start,
        line_end_before: node.line_end,
        line_start_after: None,
        line_end_after: None,
        signature_hash_before: node_signature_hash(node),
        signature_hash_after: None,
        modifiers_before: node.modifiers.clone(),
        modifiers_after: None,
        is_test_before: Some(node.is_test),
        is_test_after: None,
        extra_hash_before: stable_hash(node.extra_json.as_deref()),
        extra_hash_after: None,
        changed_fields: vec!["removed".to_owned()],
    }
}

fn node_changed(before: &HistoricalNode, after: &HistoricalNode) -> NodeDiffEntry {
    let mut changed_fields = Vec::new();
    if before.line_start != after.line_start || before.line_end != after.line_end {
        changed_fields.push("line_span".to_owned());
    }
    let before_signature = node_signature_hash(before);
    let after_signature = node_signature_hash(after);
    if before_signature != after_signature {
        changed_fields.push("signature".to_owned());
    }
    if before.modifiers != after.modifiers {
        changed_fields.push("modifiers".to_owned());
    }
    if before.is_test != after.is_test {
        changed_fields.push("is_test".to_owned());
    }
    let before_extra = stable_hash(before.extra_json.as_deref());
    let after_extra = stable_hash(after.extra_json.as_deref());
    if before_extra != after_extra {
        changed_fields.push("extra_metadata".to_owned());
    }
    NodeDiffEntry {
        qualified_name: before.qualified_name.clone(),
        file_path: before.file_path.clone(),
        kind: before.kind.clone(),
        line_start_before: before.line_start,
        line_end_before: before.line_end,
        line_start_after: after.line_start,
        line_end_after: after.line_end,
        signature_hash_before: before_signature,
        signature_hash_after: after_signature,
        modifiers_before: before.modifiers.clone(),
        modifiers_after: after.modifiers.clone(),
        is_test_before: Some(before.is_test),
        is_test_after: Some(after.is_test),
        extra_hash_before: before_extra,
        extra_hash_after: after_extra,
        changed_fields,
    }
}

fn edge_added(edge: &HistoricalEdge) -> EdgeDiffEntry {
    EdgeDiffEntry {
        source_qn: edge.source_qn.clone(),
        target_qn: edge.target_qn.clone(),
        kind: edge.kind.clone(),
        file_path: edge.file_path.clone(),
        confidence_tier_before: None,
        confidence_tier_after: edge.confidence_tier.clone(),
        metadata_hash_before: None,
        metadata_hash_after: edge_metadata_hash(edge),
        changed_fields: vec!["added".to_owned()],
    }
}

fn edge_removed(edge: &HistoricalEdge) -> EdgeDiffEntry {
    EdgeDiffEntry {
        source_qn: edge.source_qn.clone(),
        target_qn: edge.target_qn.clone(),
        kind: edge.kind.clone(),
        file_path: edge.file_path.clone(),
        confidence_tier_before: edge.confidence_tier.clone(),
        confidence_tier_after: None,
        metadata_hash_before: edge_metadata_hash(edge),
        metadata_hash_after: None,
        changed_fields: vec!["removed".to_owned()],
    }
}

fn edge_changed(before: &HistoricalEdge, after: &HistoricalEdge) -> EdgeDiffEntry {
    let mut changed_fields = Vec::new();
    if before.confidence_tier != after.confidence_tier {
        changed_fields.push("confidence_tier".to_owned());
    }
    let before_hash = edge_metadata_hash(before);
    let after_hash = edge_metadata_hash(after);
    if before_hash != after_hash {
        changed_fields.push("metadata".to_owned());
    }
    EdgeDiffEntry {
        source_qn: before.source_qn.clone(),
        target_qn: before.target_qn.clone(),
        kind: before.kind.clone(),
        file_path: before.file_path.clone(),
        confidence_tier_before: before.confidence_tier.clone(),
        confidence_tier_after: after.confidence_tier.clone(),
        metadata_hash_before: before_hash,
        metadata_hash_after: after_hash,
        changed_fields,
    }
}

fn node_signature_hash(node: &HistoricalNode) -> Option<String> {
    if node.params.is_none() && node.return_type.is_none() && node.modifiers.is_none() {
        return None;
    }
    Some(hex_hash(
        format!(
            "{}\u{1f}{}\u{1f}{}",
            node.params.as_deref().unwrap_or(""),
            node.return_type.as_deref().unwrap_or(""),
            node.modifiers.as_deref().unwrap_or(""),
        )
        .as_bytes(),
    ))
}

fn edge_metadata_hash(edge: &HistoricalEdge) -> Option<String> {
    Some(hex_hash(
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{:.6}",
            edge.line.unwrap_or_default(),
            edge.confidence_tier.as_deref().unwrap_or(""),
            edge.extra_json.as_deref().unwrap_or(""),
            edge.confidence,
        )
        .as_bytes(),
    ))
}

fn stable_hash(value: Option<&str>) -> Option<String> {
    value.map(|value| hex_hash(value.as_bytes()))
}

fn hex_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn module_counts(snapshot: &HistoricalSnapshot) -> BTreeMap<String, (usize, usize)> {
    let mut counts = BTreeMap::new();
    for node in &snapshot.nodes {
        let module = module_key(&node.file_path);
        counts.entry(module).or_insert((0, 0)).0 += 1;
    }
    for edge in &snapshot.edges {
        let module = module_key(&edge.file_path);
        counts.entry(module).or_insert((0, 0)).1 += 1;
    }
    counts
}

fn architecture_diff(before: &HistoricalSnapshot, after: &HistoricalSnapshot) -> ArchitectureDiff {
    let deps_before = module_dependency_counts(before);
    let deps_after = module_dependency_counts(after);

    let keys_before = deps_before.keys().cloned().collect::<BTreeSet<_>>();
    let keys_after = deps_after.keys().cloned().collect::<BTreeSet<_>>();

    let new_dependency_paths = keys_after
        .difference(&keys_before)
        .cloned()
        .collect::<Vec<_>>();
    let removed_dependency_paths = keys_before
        .difference(&keys_after)
        .cloned()
        .collect::<Vec<_>>();

    let changed_coupling = keys_before
        .union(&keys_after)
        .filter_map(|key| {
            let before_count = deps_before.get(key).copied().unwrap_or_default();
            let after_count = deps_after.get(key).copied().unwrap_or_default();
            if before_count == after_count {
                return None;
            }
            Some(ModuleDependencyChange {
                source_module: key.0.clone(),
                target_module: key.1.clone(),
                count_before: before_count,
                count_after: after_count,
            })
        })
        .collect::<Vec<_>>();

    let graph_before = adjacency_from_counts(&deps_before);
    let graph_after = adjacency_from_counts(&deps_after);
    let cycles_before = strongly_connected_components(&graph_before);
    let cycles_after = strongly_connected_components(&graph_after);
    let cycles_before_set = cycles_before.iter().cloned().collect::<BTreeSet<_>>();
    let cycles_after_set = cycles_after.iter().cloned().collect::<BTreeSet<_>>();

    let hubs_before = module_degrees(&graph_before);
    let hubs_after = module_degrees(&graph_after);
    let hub_names = hubs_before
        .keys()
        .cloned()
        .chain(hubs_after.keys().cloned())
        .collect::<BTreeSet<_>>();
    let changed_central_hubs = hub_names
        .into_iter()
        .filter_map(|module| {
            let before_degree = hubs_before.get(&module).copied().unwrap_or_default();
            let after_degree = hubs_after.get(&module).copied().unwrap_or_default();
            if before_degree == after_degree {
                return None;
            }
            Some(HubChange {
                module,
                degree_before: before_degree,
                degree_after: after_degree,
            })
        })
        .collect::<Vec<_>>();

    ArchitectureDiff {
        new_dependency_paths,
        removed_dependency_paths,
        new_cycles: cycles_after_set
            .difference(&cycles_before_set)
            .cloned()
            .collect(),
        broken_cycles: cycles_before_set
            .difference(&cycles_after_set)
            .cloned()
            .collect(),
        changed_central_hubs,
        changed_coupling,
    }
}

fn module_dependency_counts(snapshot: &HistoricalSnapshot) -> BTreeMap<(String, String), usize> {
    let node_modules = snapshot
        .nodes
        .iter()
        .map(|node| (node.qualified_name.clone(), module_key(&node.file_path)))
        .collect::<BTreeMap<_, _>>();
    let mut deps = BTreeMap::new();
    for edge in &snapshot.edges {
        let source_module = node_modules
            .get(&edge.source_qn)
            .cloned()
            .unwrap_or_else(|| module_key(&edge.file_path));
        let target_module = node_modules
            .get(&edge.target_qn)
            .cloned()
            .unwrap_or_else(|| module_key(&edge.file_path));
        if source_module == target_module {
            continue;
        }
        *deps.entry((source_module, target_module)).or_insert(0) += 1;
    }
    deps
}

fn adjacency_from_counts(
    deps: &BTreeMap<(String, String), usize>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut graph = BTreeMap::new();
    for (source, target) in deps.keys() {
        graph
            .entry(source.clone())
            .or_insert_with(BTreeSet::new)
            .insert(target.clone());
        graph.entry(target.clone()).or_insert_with(BTreeSet::new);
    }
    graph
}

fn module_degrees(graph: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, usize> {
    let mut degrees = BTreeMap::new();
    for (node, targets) in graph {
        *degrees.entry(node.clone()).or_insert(0) += targets.len();
        for target in targets {
            *degrees.entry(target.clone()).or_insert(0) += 1;
        }
    }
    degrees
}

fn strongly_connected_components(graph: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    struct TarjanState {
        index: usize,
        stack: Vec<String>,
        on_stack: BTreeSet<String>,
        indices: BTreeMap<String, usize>,
        lowlinks: BTreeMap<String, usize>,
        components: Vec<Vec<String>>,
    }

    fn strong_connect(
        node: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        state: &mut TarjanState,
    ) {
        let index = state.index;
        state.indices.insert(node.to_owned(), index);
        state.lowlinks.insert(node.to_owned(), index);
        state.index += 1;
        state.stack.push(node.to_owned());
        state.on_stack.insert(node.to_owned());

        for target in graph
            .get(node)
            .into_iter()
            .flat_map(|targets| targets.iter())
        {
            if !state.indices.contains_key(target) {
                strong_connect(target, graph, state);
                let target_low = *state.lowlinks.get(target).expect("lowlink");
                let lowlink = state.lowlinks.get_mut(node).expect("node lowlink");
                *lowlink = (*lowlink).min(target_low);
            } else if state.on_stack.contains(target) {
                let target_index = *state.indices.get(target).expect("target index");
                let lowlink = state.lowlinks.get_mut(node).expect("node lowlink");
                *lowlink = (*lowlink).min(target_index);
            }
        }

        if state.lowlinks.get(node) == state.indices.get(node) {
            let mut component = Vec::new();
            while let Some(entry) = state.stack.pop() {
                state.on_stack.remove(&entry);
                component.push(entry.clone());
                if entry == node {
                    break;
                }
            }
            component.sort();
            if component.len() > 1
                || graph
                    .get(node)
                    .is_some_and(|targets| targets.contains(node))
            {
                state.components.push(component);
            }
        }
    }

    let mut state = TarjanState {
        index: 0,
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        indices: BTreeMap::new(),
        lowlinks: BTreeMap::new(),
        components: Vec::new(),
    };
    for node in graph.keys() {
        if !state.indices.contains_key(node) {
            strong_connect(node, graph, &mut state);
        }
    }
    state.components.sort();
    state.components
}

pub(crate) fn module_key(path: &str) -> String {
    let parts = path.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        ["packages", crate_name, ..] => format!("packages/{crate_name}"),
        [first, second, ..] if matches!(*first, "src" | "tests" | "benches" | "examples") => {
            format!("{first}/{second}")
        }
        [first, ..] => (*first).to_owned(),
        [] => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use atlas_store_sqlite::{HistoricalEdge, HistoricalNode, StoredSnapshot};

    use super::*;

    fn snapshot(
        commit: &str,
        nodes: Vec<HistoricalNode>,
        edges: Vec<HistoricalEdge>,
    ) -> HistoricalSnapshot {
        let files = vec![StoredSnapshotFile {
            snapshot_id: 1,
            file_path: "packages/app/src/lib.rs".to_owned(),
            file_hash: "hash".to_owned(),
            language: Some("rust".to_owned()),
            size: Some(12),
        }];
        HistoricalSnapshot {
            summary: HistoricalSnapshotSummary {
                snapshot_id: 1,
                commit_sha: commit.to_owned(),
                node_count: nodes.len() as i64,
                edge_count: edges.len() as i64,
                file_count: 1,
                completeness: 1.0,
                parse_error_count: 0,
            },
            evidence: build_evidence(
                [1],
                [commit.to_owned()],
                nodes.iter().map(node_identifier_for),
                edges.iter().map(edge_identifier_for),
                files.iter().map(|file| file.file_path.clone()),
            ),
            repo_id: 1,
            commit_sha: commit.to_owned(),
            root_tree_hash: None,
            node_count: nodes.len() as i64,
            edge_count: edges.len() as i64,
            file_count: 1,
            completeness: 1.0,
            parse_error_count: 0,
            files,
            nodes,
            edges,
        }
    }

    fn node(qn: &str, file_path: &str, params: &str, line_end: i64) -> HistoricalNode {
        HistoricalNode {
            file_hash: "hash".to_owned(),
            qualified_name: qn.to_owned(),
            kind: "function".to_owned(),
            name: qn.rsplit("::").next().unwrap_or(qn).to_owned(),
            file_path: file_path.to_owned(),
            line_start: Some(1),
            line_end: Some(line_end),
            language: Some("rust".to_owned()),
            parent_name: None,
            params: Some(params.to_owned()),
            return_type: Some("i32".to_owned()),
            modifiers: None,
            is_test: false,
            extra_json: None,
        }
    }

    fn edge(src: &str, tgt: &str, file_path: &str) -> HistoricalEdge {
        HistoricalEdge {
            file_hash: "hash".to_owned(),
            source_qn: src.to_owned(),
            target_qn: tgt.to_owned(),
            kind: "calls".to_owned(),
            file_path: file_path.to_owned(),
            line: Some(4),
            confidence: 1.0,
            confidence_tier: Some("definite".to_owned()),
            extra_json: None,
        }
    }

    #[test]
    fn node_diff_detects_add_remove_and_change() {
        let before = node("crate::alpha", "packages/app/src/lib.rs", "()", 8);
        let after = node("crate::alpha", "packages/app/src/lib.rs", "(x: i32)", 10);
        let added = node_added(&after);
        let removed = node_removed(&before);
        let changed = node_changed(&before, &after);
        assert_eq!(added.changed_fields, vec!["added"]);
        assert_eq!(removed.changed_fields, vec!["removed"]);
        assert!(changed.changed_fields.contains(&"line_span".to_owned()));
        assert!(changed.changed_fields.contains(&"signature".to_owned()));
    }

    #[test]
    fn diff_treats_qualified_name_change_as_remove_plus_add() {
        let before = snapshot(
            "a",
            vec![node("crate::alpha", "packages/app/src/lib.rs", "()", 8)],
            vec![],
        );
        let after = snapshot(
            "b",
            vec![node("crate::beta", "packages/app/src/lib.rs", "()", 8)],
            vec![],
        );

        let removed = before
            .nodes
            .iter()
            .filter(|node| {
                !after.nodes.iter().any(|other| {
                    other.qualified_name == node.qualified_name
                        && other.file_path == node.file_path
                        && other.kind == node.kind
                })
            })
            .map(|node| node_removed(node).qualified_name)
            .collect::<Vec<_>>();
        let added = after
            .nodes
            .iter()
            .filter(|node| {
                !before.nodes.iter().any(|other| {
                    other.qualified_name == node.qualified_name
                        && other.file_path == node.file_path
                        && other.kind == node.kind
                })
            })
            .map(|node| node_added(node).qualified_name)
            .collect::<Vec<_>>();

        assert_eq!(removed, vec!["crate::alpha"]);
        assert_eq!(added, vec!["crate::beta"]);
    }

    #[test]
    fn edge_diff_detects_add_remove() {
        let before = edge("crate::a", "crate::b", "packages/app/src/lib.rs");
        let after = edge("crate::a", "crate::c", "packages/app/src/lib.rs");
        assert_eq!(edge_added(&after).changed_fields, vec!["added"]);
        assert_eq!(edge_removed(&before).changed_fields, vec!["removed"]);
    }

    #[test]
    fn architecture_diff_detects_new_cycle_and_broken_cycle() {
        let before = snapshot(
            "a",
            vec![
                node("crate::a", "packages/app/src/a.rs", "()", 5),
                node("crate::b", "packages/lib/src/b.rs", "()", 5),
            ],
            vec![edge("crate::a", "crate::b", "packages/app/src/a.rs")],
        );
        let after = snapshot(
            "b",
            vec![
                node("crate::a", "packages/app/src/a.rs", "()", 5),
                node("crate::b", "packages/lib/src/b.rs", "()", 5),
            ],
            vec![
                edge("crate::a", "crate::b", "packages/app/src/a.rs"),
                edge("crate::b", "crate::a", "packages/lib/src/b.rs"),
            ],
        );
        let new_cycle = architecture_diff(&before, &after);
        assert_eq!(new_cycle.new_cycles.len(), 1);
        let broken_cycle = architecture_diff(&after, &before);
        assert_eq!(broken_cycle.broken_cycles.len(), 1);
    }

    #[test]
    fn validate_snapshot_materialization_rejects_membership_count_mismatch() {
        let snapshot = StoredSnapshot {
            snapshot_id: 7,
            repo_id: 1,
            commit_sha: "a".repeat(40),
            root_tree_hash: None,
            node_count: 2,
            edge_count: 1,
            file_count: 1,
            created_at: String::new(),
            completeness: 1.0,
            parse_error_count: 0,
        };
        let files = vec![StoredSnapshotFile {
            snapshot_id: 7,
            file_path: "packages/app/src/lib.rs".to_owned(),
            file_hash: "hash".to_owned(),
            language: Some("rust".to_owned()),
            size: Some(12),
        }];
        let nodes = vec![node("crate::only_one", "packages/app/src/lib.rs", "", 4)];
        let edges = vec![edge(
            "crate::only_one",
            "crate::other",
            "packages/app/src/lib.rs",
        )];

        let error = validate_snapshot_materialization(&snapshot, &files, &nodes, &edges)
            .expect_err("mismatched node count must fail");
        assert!(
            error
                .to_string()
                .contains("snapshot membership rows appear corrupted")
        );
        assert!(error.to_string().contains("nodes expected=2 actual=1"));
    }
}
