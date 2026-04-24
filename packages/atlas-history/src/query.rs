use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use atlas_store_sqlite::{
    HistoricalEdge, HistoricalNode, Store, StoredCommit, StoredSnapshot, StoredSnapshotFile,
};
use serde::Serialize;

use crate::diff::{module_key, validate_snapshot_materialization};
use crate::git;
use crate::reports::{HistoryEvidence, build_evidence, edge_identifier_for, node_identifier_for};

#[derive(Debug, Clone)]
pub(crate) struct SnapshotState {
    pub(crate) snapshot_id: i64,
    pub(crate) commit_sha: String,
    pub(crate) author_time: i64,
    pub(crate) files: Vec<StoredSnapshotFile>,
    pub(crate) nodes: Vec<HistoricalNode>,
    pub(crate) edges: Vec<HistoricalEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeHistorySummary {
    pub first_appearance_snapshot_id: Option<i64>,
    pub last_appearance_snapshot_id: Option<i64>,
    pub first_appearance_commit_sha: Option<String>,
    pub last_appearance_commit_sha: Option<String>,
    pub removal_commit_sha: Option<String>,
    pub change_commit_count: usize,
    pub signature_version_count: usize,
    pub current_file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeChangeRecord {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub change_kinds: Vec<String>,
    pub file_paths: Vec<String>,
    pub node_identifiers: Vec<String>,
    pub signature_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeSignatureRecord {
    pub file_path: String,
    pub kind: String,
    pub params: Option<String>,
    pub return_type: Option<String>,
    pub modifiers: Option<String>,
    pub signature_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeSignatureSnapshot {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub signatures: Vec<NodeSignatureRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeFilePathSnapshot {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NodeHistoryFindings {
    pub appearances: Vec<NodeChangeRecord>,
    pub commits_where_changed: Vec<NodeChangeRecord>,
    pub signature_evolution: Vec<NodeSignatureSnapshot>,
    pub file_path_changes: Vec<NodeFilePathSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeHistoryReport {
    pub qualified_name: String,
    pub summary: NodeHistorySummary,
    pub findings: NodeHistoryFindings,
    pub evidence: HistoryEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHistorySummary {
    pub file_path: String,
    pub first_appearance_snapshot_id: Option<i64>,
    pub last_appearance_snapshot_id: Option<i64>,
    pub first_appearance_commit_sha: Option<String>,
    pub last_appearance_commit_sha: Option<String>,
    pub removal_commit_sha: Option<String>,
    pub commit_touch_count: usize,
    pub timeline_points: usize,
    pub current_file_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHistoryPoint {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub exists: bool,
    pub file_hash: Option<String>,
    pub node_count: usize,
    pub edge_count: usize,
    pub symbol_additions: Vec<String>,
    pub symbol_removals: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHistoryFindings {
    pub commits_touched: Vec<FileHistoryPoint>,
    pub timeline: Vec<FileHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHistoryReport {
    pub file_path: String,
    pub summary: FileHistorySummary,
    pub findings: FileHistoryFindings,
    pub evidence: HistoryEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeHistorySummary {
    pub source_qn: String,
    pub target_qn: String,
    pub first_appearance_snapshot_id: Option<i64>,
    pub last_appearance_snapshot_id: Option<i64>,
    pub first_appearance_commit_sha: Option<String>,
    pub last_appearance_commit_sha: Option<String>,
    pub disappearance_commit_sha: Option<String>,
    pub currently_present: bool,
    pub change_commit_count: usize,
    pub persistence_duration_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyHistoryPoint {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub present: bool,
    pub edge_count: usize,
    pub edge_identifiers: Vec<String>,
    pub added_edges: Vec<String>,
    pub removed_edges: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeHistoryFindings {
    pub timeline: Vec<DependencyHistoryPoint>,
    pub commits_where_changed: Vec<DependencyHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeHistoryReport {
    pub summary: EdgeHistorySummary,
    pub findings: EdgeHistoryFindings,
    pub evidence: HistoryEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleHistorySummary {
    pub module: String,
    pub first_appearance_snapshot_id: Option<i64>,
    pub last_appearance_snapshot_id: Option<i64>,
    pub first_appearance_commit_sha: Option<String>,
    pub last_appearance_commit_sha: Option<String>,
    pub removal_commit_sha: Option<String>,
    pub max_node_count: usize,
    pub max_dependency_count: usize,
    pub max_coupling_count: usize,
    pub max_test_adjacency_count: usize,
    pub timeline_points: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleHistoryPoint {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub node_count: usize,
    pub dependency_count: usize,
    pub coupling_count: usize,
    pub test_adjacency_count: usize,
    pub file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleHistoryFindings {
    pub timeline: Vec<ModuleHistoryPoint>,
    pub commits_where_changed: Vec<ModuleHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleHistoryReport {
    pub module: String,
    pub summary: ModuleHistorySummary,
    pub findings: ModuleHistoryFindings,
    pub evidence: HistoryEvidence,
}

pub fn query_symbol_history(
    store: &Store,
    canonical_root: &str,
    qualified_name: &str,
) -> Result<NodeHistoryReport> {
    let states = load_snapshot_states(store, canonical_root)?;
    let mut findings = NodeHistoryFindings::default();
    let mut evidence_snapshot_ids = BTreeSet::new();
    let mut evidence_commit_shas = BTreeSet::new();
    let mut evidence_node_ids = BTreeSet::new();
    let mut evidence_file_paths = BTreeSet::new();
    let mut first_seen = None;
    let mut last_seen = None;
    let mut removal_commit_sha = None;
    let mut prev_present = false;
    let mut prev_signature_keys = BTreeSet::new();
    let mut prev_paths = BTreeSet::new();
    let mut prev_node_ids = BTreeSet::new();

    for state in &states {
        let matches = state
            .nodes
            .iter()
            .filter(|node| node.qualified_name == qualified_name)
            .cloned()
            .collect::<Vec<_>>();
        let present = !matches.is_empty();

        if present {
            let file_paths = sorted_strings(matches.iter().map(|node| node.file_path.clone()));
            let node_ids = sorted_strings(matches.iter().map(node_identifier_for));
            let signature_records = matches
                .iter()
                .map(|node| NodeSignatureRecord {
                    file_path: node.file_path.clone(),
                    kind: node.kind.clone(),
                    params: node.params.clone(),
                    return_type: node.return_type.clone(),
                    modifiers: node.modifiers.clone(),
                    signature_hash: node_signature_hash(node),
                })
                .collect::<Vec<_>>();
            let signature_hashes = sorted_strings(
                signature_records
                    .iter()
                    .filter_map(|record| record.signature_hash.clone()),
            );
            let signature_keys = signature_records
                .iter()
                .map(signature_record_key)
                .collect::<BTreeSet<_>>();

            evidence_snapshot_ids.insert(state.snapshot_id);
            evidence_commit_shas.insert(state.commit_sha.clone());
            evidence_node_ids.extend(node_ids.iter().cloned());
            evidence_file_paths.extend(file_paths.iter().cloned());

            let appearance = NodeChangeRecord {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                change_kinds: vec!["present".to_owned()],
                file_paths: file_paths.clone(),
                node_identifiers: node_ids.clone(),
                signature_hashes: signature_hashes.clone(),
            };
            findings.appearances.push(appearance);

            let mut change_kinds = Vec::new();
            if !prev_present {
                change_kinds.push("introduced".to_owned());
                first_seen = first_seen.or(Some((state.snapshot_id, state.commit_sha.clone())));
            }
            if signature_keys != prev_signature_keys {
                findings.signature_evolution.push(NodeSignatureSnapshot {
                    snapshot_id: state.snapshot_id,
                    commit_sha: state.commit_sha.clone(),
                    signatures: signature_records.clone(),
                });
                if prev_present {
                    change_kinds.push("signature_evolution".to_owned());
                }
            }
            let current_paths = file_paths.iter().cloned().collect::<BTreeSet<_>>();
            if current_paths != prev_paths {
                findings.file_path_changes.push(NodeFilePathSnapshot {
                    snapshot_id: state.snapshot_id,
                    commit_sha: state.commit_sha.clone(),
                    file_paths: file_paths.clone(),
                });
                if prev_present {
                    change_kinds.push("file_paths".to_owned());
                }
            }
            if !change_kinds.is_empty() {
                findings.commits_where_changed.push(NodeChangeRecord {
                    snapshot_id: state.snapshot_id,
                    commit_sha: state.commit_sha.clone(),
                    change_kinds,
                    file_paths: file_paths.clone(),
                    node_identifiers: node_ids.clone(),
                    signature_hashes: signature_hashes.clone(),
                });
            }

            last_seen = Some((
                state.snapshot_id,
                state.commit_sha.clone(),
                file_paths.clone(),
            ));
            prev_signature_keys = signature_keys;
            prev_paths = current_paths;
            prev_node_ids = node_ids.into_iter().collect();
        } else if prev_present {
            removal_commit_sha = Some(state.commit_sha.clone());
            findings.commits_where_changed.push(NodeChangeRecord {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                change_kinds: vec!["removed".to_owned()],
                file_paths: prev_paths.iter().cloned().collect(),
                node_identifiers: prev_node_ids.iter().cloned().collect(),
                signature_hashes: Vec::new(),
            });
            prev_signature_keys.clear();
            prev_paths.clear();
            prev_node_ids.clear();
        }

        prev_present = present;
    }

    let Some((first_snapshot_id, first_commit_sha)) = first_seen else {
        anyhow::bail!("no historical symbol matches found for {qualified_name}");
    };
    let (last_snapshot_id, last_commit_sha, current_file_paths) = last_seen.ok_or_else(|| {
        anyhow::anyhow!("symbol history missing last appearance for {qualified_name}")
    })?;

    Ok(NodeHistoryReport {
        qualified_name: qualified_name.to_owned(),
        summary: NodeHistorySummary {
            first_appearance_snapshot_id: Some(first_snapshot_id),
            last_appearance_snapshot_id: Some(last_snapshot_id),
            first_appearance_commit_sha: Some(first_commit_sha),
            last_appearance_commit_sha: Some(last_commit_sha),
            removal_commit_sha,
            change_commit_count: findings.commits_where_changed.len(),
            signature_version_count: findings.signature_evolution.len(),
            current_file_paths,
        },
        findings,
        evidence: build_evidence(
            evidence_snapshot_ids,
            evidence_commit_shas,
            evidence_node_ids,
            std::iter::empty::<String>(),
            evidence_file_paths,
        ),
    })
}

pub fn query_file_history(
    store: &Store,
    canonical_root: &str,
    file_path: &str,
) -> Result<FileHistoryReport> {
    query_file_history_with_options(
        store,
        canonical_root,
        Path::new(canonical_root),
        file_path,
        false,
    )
}

pub fn query_file_history_with_options(
    store: &Store,
    canonical_root: &str,
    repo: &Path,
    file_path: &str,
    follow_renames: bool,
) -> Result<FileHistoryReport> {
    let snapshot_catalog = load_snapshot_catalog(store, canonical_root)?;
    let commit_shas = snapshot_catalog
        .iter()
        .map(|(snapshot, _)| snapshot.commit_sha.clone())
        .collect::<Vec<_>>();
    let tracked_paths = resolve_file_history_paths(&commit_shas, repo, file_path, follow_renames)?;
    let mut timeline = Vec::new();
    let mut commits_touched = Vec::new();
    let mut evidence_snapshot_ids = BTreeSet::new();
    let mut evidence_commit_shas = BTreeSet::new();
    let mut evidence_node_ids = BTreeSet::new();
    let mut evidence_edge_ids = BTreeSet::new();
    let mut evidence_file_paths = BTreeSet::new();
    let mut first_seen = None;
    let mut last_seen = None;
    let mut removal_commit_sha = None;
    let mut prev_exists = false;
    let mut prev_hash = None::<String>;
    let mut prev_symbols = BTreeSet::new();

    for (index, (snapshot, commit)) in snapshot_catalog.iter().enumerate() {
        let candidate_paths = &tracked_paths[index];
        let state =
            load_partial_snapshot_state_for_paths(store, snapshot, commit, candidate_paths)?;
        let file = candidate_paths.iter().find_map(|candidate_path| {
            state
                .files
                .iter()
                .find(|file| file.file_path == *candidate_path)
                .cloned()
        });
        let nodes = state.nodes.clone();
        let edges = state.edges.clone();
        let exists = file.is_some();
        let file_hash = file.as_ref().map(|file| file.file_hash.clone());
        let symbols = nodes
            .iter()
            .map(|node| node.qualified_name.clone())
            .collect::<BTreeSet<_>>();
        let symbol_additions = symbols
            .difference(&prev_symbols)
            .cloned()
            .collect::<Vec<_>>();
        let symbol_removals = prev_symbols
            .difference(&symbols)
            .cloned()
            .collect::<Vec<_>>();

        if exists || prev_exists {
            timeline.push(FileHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: snapshot.commit_sha.clone(),
                exists,
                file_hash: file_hash.clone(),
                node_count: nodes.len(),
                edge_count: edges.len(),
                symbol_additions: symbol_additions.clone(),
                symbol_removals: symbol_removals.clone(),
            });
        }

        let touched = if exists && !prev_exists {
            true
        } else if !exists && prev_exists {
            removal_commit_sha = Some(state.commit_sha.clone());
            true
        } else {
            file_hash != prev_hash
        };
        if touched && (exists || prev_exists) {
            commits_touched.push(FileHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: snapshot.commit_sha.clone(),
                exists,
                file_hash: file_hash.clone(),
                node_count: nodes.len(),
                edge_count: edges.len(),
                symbol_additions: symbol_additions.clone(),
                symbol_removals: symbol_removals.clone(),
            });
        }

        if exists {
            evidence_snapshot_ids.insert(state.snapshot_id);
            evidence_commit_shas.insert(snapshot.commit_sha.clone());
            evidence_node_ids.extend(nodes.iter().map(node_identifier_for));
            evidence_edge_ids.extend(edges.iter().map(edge_identifier_for));
            if let Some(file) = &file {
                evidence_file_paths.insert(file.file_path.clone());
            }
            if first_seen.is_none() {
                first_seen = Some((state.snapshot_id, snapshot.commit_sha.clone()));
            }
            last_seen = Some((
                state.snapshot_id,
                snapshot.commit_sha.clone(),
                file_hash.clone(),
            ));
        }

        prev_exists = exists;
        prev_hash = file_hash;
        prev_symbols = symbols;
    }

    let Some((first_snapshot_id, first_commit_sha)) = first_seen else {
        anyhow::bail!("no historical file matches found for {file_path}");
    };
    let (last_snapshot_id, last_commit_sha, current_file_hash) = last_seen
        .ok_or_else(|| anyhow::anyhow!("file history missing last appearance for {file_path}"))?;

    Ok(FileHistoryReport {
        file_path: file_path.to_owned(),
        summary: FileHistorySummary {
            file_path: file_path.to_owned(),
            first_appearance_snapshot_id: Some(first_snapshot_id),
            last_appearance_snapshot_id: Some(last_snapshot_id),
            first_appearance_commit_sha: Some(first_commit_sha),
            last_appearance_commit_sha: Some(last_commit_sha),
            removal_commit_sha,
            commit_touch_count: commits_touched.len(),
            timeline_points: timeline.len(),
            current_file_hash,
        },
        findings: FileHistoryFindings {
            commits_touched,
            timeline,
        },
        evidence: build_evidence(
            evidence_snapshot_ids,
            evidence_commit_shas,
            evidence_node_ids,
            evidence_edge_ids,
            evidence_file_paths,
        ),
    })
}

fn resolve_file_history_paths(
    commit_shas: &[String],
    repo: &Path,
    file_path: &str,
    follow_renames: bool,
) -> Result<Vec<BTreeSet<String>>> {
    let mut paths = vec![BTreeSet::new(); commit_shas.len()];
    let mut active = BTreeSet::from([file_path.to_owned()]);

    for index in (0..commit_shas.len()).rev() {
        paths[index] = active.clone();
        if !follow_renames || index == 0 {
            continue;
        }

        let renames =
            git::diff_tree_files(repo, &commit_shas[index], Some(&commit_shas[index - 1]))?
                .into_iter()
                .filter(|(_, _, status)| *status == 'R')
                .collect::<Vec<_>>();
        let mut previous = active.clone();
        for (old_path, new_path, _) in renames {
            if active.contains(&new_path) {
                previous.remove(&new_path);
                previous.insert(old_path);
            }
        }
        active = previous;
    }

    Ok(paths)
}

pub fn query_dependency_history(
    store: &Store,
    canonical_root: &str,
    source_qn: &str,
    target_qn: &str,
) -> Result<EdgeHistoryReport> {
    let states = load_snapshot_states(store, canonical_root)?;
    let mut timeline = Vec::new();
    let mut commits_where_changed = Vec::new();
    let mut evidence_snapshot_ids = BTreeSet::new();
    let mut evidence_commit_shas = BTreeSet::new();
    let mut evidence_edge_ids = BTreeSet::new();
    let mut evidence_file_paths = BTreeSet::new();
    let mut first_seen = None;
    let mut last_seen = None;
    let mut disappearance_commit_sha = None;
    let mut prev_edges = BTreeSet::new();
    let mut prev_present = false;
    let mut first_author_time = None;
    let mut last_author_time = None;

    for state in &states {
        let edges = state
            .edges
            .iter()
            .filter(|edge| edge.source_qn == source_qn && edge.target_qn == target_qn)
            .cloned()
            .collect::<Vec<_>>();
        let edge_ids = edges
            .iter()
            .map(edge_identifier_for)
            .collect::<BTreeSet<_>>();
        let present = !edge_ids.is_empty();
        let added_edges = edge_ids
            .difference(&prev_edges)
            .cloned()
            .collect::<Vec<_>>();
        let removed_edges = prev_edges
            .difference(&edge_ids)
            .cloned()
            .collect::<Vec<_>>();

        if present || prev_present {
            timeline.push(DependencyHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                present,
                edge_count: edge_ids.len(),
                edge_identifiers: edge_ids.iter().cloned().collect(),
                added_edges: added_edges.clone(),
                removed_edges: removed_edges.clone(),
            });
        }

        if !added_edges.is_empty() || !removed_edges.is_empty() || (present && !prev_present) {
            commits_where_changed.push(DependencyHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                present,
                edge_count: edge_ids.len(),
                edge_identifiers: edge_ids.iter().cloned().collect(),
                added_edges: added_edges.clone(),
                removed_edges: removed_edges.clone(),
            });
        }

        if present {
            evidence_snapshot_ids.insert(state.snapshot_id);
            evidence_commit_shas.insert(state.commit_sha.clone());
            evidence_edge_ids.extend(edge_ids.iter().cloned());
            evidence_file_paths.extend(edges.iter().map(|edge| edge.file_path.clone()));
            if first_seen.is_none() {
                first_seen = Some((state.snapshot_id, state.commit_sha.clone()));
                first_author_time = Some(state.author_time);
            }
            last_seen = Some((state.snapshot_id, state.commit_sha.clone()));
            last_author_time = Some(state.author_time);
        } else if prev_present {
            disappearance_commit_sha = Some(state.commit_sha.clone());
        }

        prev_edges = edge_ids;
        prev_present = present;
    }

    let Some((first_snapshot_id, first_commit_sha)) = first_seen else {
        anyhow::bail!("no historical dependency matches found for {source_qn} -> {target_qn}");
    };
    let (last_snapshot_id, last_commit_sha) = last_seen.ok_or_else(|| {
        anyhow::anyhow!("dependency history missing last appearance for {source_qn} -> {target_qn}")
    })?;
    let persistence_duration_secs = first_author_time
        .zip(last_author_time)
        .map(|(first, last)| last.saturating_sub(first));

    Ok(EdgeHistoryReport {
        summary: EdgeHistorySummary {
            source_qn: source_qn.to_owned(),
            target_qn: target_qn.to_owned(),
            first_appearance_snapshot_id: Some(first_snapshot_id),
            last_appearance_snapshot_id: Some(last_snapshot_id),
            first_appearance_commit_sha: Some(first_commit_sha),
            last_appearance_commit_sha: Some(last_commit_sha),
            disappearance_commit_sha,
            currently_present: prev_present,
            change_commit_count: commits_where_changed.len(),
            persistence_duration_secs,
        },
        findings: EdgeHistoryFindings {
            timeline,
            commits_where_changed,
        },
        evidence: build_evidence(
            evidence_snapshot_ids,
            evidence_commit_shas,
            [
                node_identifier_for_source(source_qn),
                node_identifier_for_source(target_qn),
            ],
            evidence_edge_ids,
            evidence_file_paths,
        ),
    })
}

pub fn query_module_history(
    store: &Store,
    canonical_root: &str,
    module: &str,
) -> Result<ModuleHistoryReport> {
    let states = load_snapshot_states(store, canonical_root)?;
    let mut timeline = Vec::new();
    let mut commits_where_changed = Vec::new();
    let mut evidence_snapshot_ids = BTreeSet::new();
    let mut evidence_commit_shas = BTreeSet::new();
    let mut evidence_node_ids = BTreeSet::new();
    let mut evidence_edge_ids = BTreeSet::new();
    let mut evidence_file_paths = BTreeSet::new();
    let mut first_seen = None;
    let mut last_seen = None;
    let mut removal_commit_sha = None;
    let mut prev_point = None::<(usize, usize, usize, usize)>;
    let mut prev_present = false;

    for state in &states {
        let node_lookup = state
            .nodes
            .iter()
            .map(|node| (node.qualified_name.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let node_modules = state
            .nodes
            .iter()
            .map(|node| (node.qualified_name.clone(), module_key(&node.file_path)))
            .collect::<BTreeMap<_, _>>();
        let nodes = state
            .nodes
            .iter()
            .filter(|node| module_key(&node.file_path) == module)
            .cloned()
            .collect::<Vec<_>>();
        let node_count = nodes.len();
        let file_paths = sorted_strings(nodes.iter().map(|node| node.file_path.clone()));
        let mut dependency_paths = BTreeSet::new();
        let mut coupling_count = 0usize;
        let mut test_adjacent = BTreeSet::new();
        let mut edge_ids = BTreeSet::new();

        for node in &nodes {
            if node.is_test {
                test_adjacent.insert(node.qualified_name.clone());
            }
        }

        for edge in &state.edges {
            let source_module = node_modules
                .get(&edge.source_qn)
                .cloned()
                .unwrap_or_else(|| module_key(&edge.file_path));
            let target_module = node_modules
                .get(&edge.target_qn)
                .cloned()
                .unwrap_or_else(|| module_key(&edge.file_path));

            if source_module != target_module
                && (source_module == module || target_module == module)
            {
                dependency_paths.insert((source_module.clone(), target_module.clone()));
                coupling_count += 1;
                edge_ids.insert(edge_identifier_for(edge));
            }

            if source_module == module || target_module == module {
                if let Some(node) = node_lookup.get(&edge.source_qn)
                    && node.is_test
                {
                    test_adjacent.insert(node.qualified_name.clone());
                }
                if let Some(node) = node_lookup.get(&edge.target_qn)
                    && node.is_test
                {
                    test_adjacent.insert(node.qualified_name.clone());
                }
            }
        }

        let active = node_count > 0 || !dependency_paths.is_empty() || coupling_count > 0;
        if active || prev_present {
            timeline.push(ModuleHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                node_count,
                dependency_count: dependency_paths.len(),
                coupling_count,
                test_adjacency_count: test_adjacent.len(),
                file_paths: file_paths.clone(),
            });
        }

        let point_tuple = (
            node_count,
            dependency_paths.len(),
            coupling_count,
            test_adjacent.len(),
        );
        if active && (!prev_present || prev_point != Some(point_tuple)) {
            commits_where_changed.push(ModuleHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                node_count,
                dependency_count: dependency_paths.len(),
                coupling_count,
                test_adjacency_count: test_adjacent.len(),
                file_paths: file_paths.clone(),
            });
        } else if !active && prev_present {
            removal_commit_sha = Some(state.commit_sha.clone());
            commits_where_changed.push(ModuleHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                node_count,
                dependency_count: dependency_paths.len(),
                coupling_count,
                test_adjacency_count: test_adjacent.len(),
                file_paths: file_paths.clone(),
            });
        }

        if active {
            evidence_snapshot_ids.insert(state.snapshot_id);
            evidence_commit_shas.insert(state.commit_sha.clone());
            evidence_node_ids.extend(nodes.iter().map(node_identifier_for));
            evidence_edge_ids.extend(edge_ids);
            evidence_file_paths.extend(file_paths.iter().cloned());
            if first_seen.is_none() {
                first_seen = Some((state.snapshot_id, state.commit_sha.clone()));
            }
            last_seen = Some((
                state.snapshot_id,
                state.commit_sha.clone(),
                point_tuple.0,
                point_tuple.1,
                point_tuple.2,
                point_tuple.3,
            ));
        }

        prev_point = Some(point_tuple);
        prev_present = active;
    }

    let Some((first_snapshot_id, first_commit_sha)) = first_seen else {
        anyhow::bail!("no historical module matches found for {module}");
    };
    let (last_snapshot_id, last_commit_sha, _, _, _, _) = last_seen
        .ok_or_else(|| anyhow::anyhow!("module history missing last appearance for {module}"))?;
    let max_node_count = timeline
        .iter()
        .map(|point| point.node_count)
        .max()
        .unwrap_or(0);
    let max_dependency_count = timeline
        .iter()
        .map(|point| point.dependency_count)
        .max()
        .unwrap_or(0);
    let max_coupling_count = timeline
        .iter()
        .map(|point| point.coupling_count)
        .max()
        .unwrap_or(0);
    let max_test_adjacency_count = timeline
        .iter()
        .map(|point| point.test_adjacency_count)
        .max()
        .unwrap_or(0);

    Ok(ModuleHistoryReport {
        module: module.to_owned(),
        summary: ModuleHistorySummary {
            module: module.to_owned(),
            first_appearance_snapshot_id: Some(first_snapshot_id),
            last_appearance_snapshot_id: Some(last_snapshot_id),
            first_appearance_commit_sha: Some(first_commit_sha),
            last_appearance_commit_sha: Some(last_commit_sha),
            removal_commit_sha,
            max_node_count,
            max_dependency_count,
            max_coupling_count,
            max_test_adjacency_count,
            timeline_points: timeline.len(),
        },
        findings: ModuleHistoryFindings {
            timeline,
            commits_where_changed,
        },
        evidence: build_evidence(
            evidence_snapshot_ids,
            evidence_commit_shas,
            evidence_node_ids,
            evidence_edge_ids,
            evidence_file_paths,
        ),
    })
}

pub(crate) fn load_snapshot_states(
    store: &Store,
    canonical_root: &str,
) -> Result<Vec<SnapshotState>> {
    let snapshot_catalog = load_snapshot_catalog(store, canonical_root)?;

    snapshot_catalog
        .into_iter()
        .map(|(snapshot, commit)| {
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
            Ok(SnapshotState {
                snapshot_id: snapshot.snapshot_id,
                commit_sha: snapshot.commit_sha.clone(),
                author_time: commit.author_time,
                files,
                nodes,
                edges,
            })
        })
        .collect()
}

fn load_snapshot_catalog(
    store: &Store,
    canonical_root: &str,
) -> Result<Vec<(StoredSnapshot, StoredCommit)>> {
    let repo_id = store
        .find_repo_id(canonical_root)?
        .ok_or_else(|| anyhow::anyhow!("repo not yet registered for history: {canonical_root}"))?;
    let commits = store
        .list_commits(repo_id)?
        .into_iter()
        .map(|commit| (commit.commit_sha.clone(), commit))
        .collect::<BTreeMap<_, StoredCommit>>();
    let snapshots = store.list_snapshots_ordered(repo_id)?;
    if snapshots.is_empty() {
        anyhow::bail!("no indexed historical snapshots for {canonical_root}");
    }

    snapshots
        .into_iter()
        .map(|snapshot| {
            let commit = commits.get(&snapshot.commit_sha).ok_or_else(|| {
                anyhow::anyhow!(
                    "commit metadata missing for snapshot {}",
                    snapshot.commit_sha
                )
            })?;
            Ok((snapshot, commit.clone()))
        })
        .collect()
}

fn load_partial_snapshot_state_for_paths(
    store: &Store,
    snapshot: &StoredSnapshot,
    commit: &StoredCommit,
    target_paths: &BTreeSet<String>,
) -> Result<SnapshotState> {
    if target_paths.is_empty() {
        return Ok(SnapshotState {
            snapshot_id: snapshot.snapshot_id,
            commit_sha: snapshot.commit_sha.clone(),
            author_time: commit.author_time,
            files: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        });
    }

    let files = store
        .list_snapshot_files(snapshot.snapshot_id)?
        .into_iter()
        .filter(|file| target_paths.contains(&file.file_path))
        .collect::<Vec<_>>();
    let blobs = store
        .list_snapshot_membership_blobs(snapshot.snapshot_id)?
        .into_iter()
        .filter(|blob| target_paths.contains(&blob.file_path))
        .collect::<Vec<_>>();
    let mut node_cache = BTreeMap::<String, Vec<HistoricalNode>>::new();
    let mut edge_cache = BTreeMap::<String, Vec<HistoricalEdge>>::new();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for blob in blobs {
        let node_membership = decode_node_membership(&blob.node_membership)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let edge_membership = decode_edge_membership(&blob.edge_membership)
            .into_iter()
            .collect::<BTreeSet<_>>();

        let file_nodes = if let Some(cached) = node_cache.get(&blob.file_hash) {
            cached.clone()
        } else {
            let loaded = store.list_historical_nodes_for_hash(&blob.file_hash)?;
            node_cache.insert(blob.file_hash.clone(), loaded.clone());
            loaded
        };
        let file_edges = if let Some(cached) = edge_cache.get(&blob.file_hash) {
            cached.clone()
        } else {
            let loaded = store.list_historical_edges_for_hash(&blob.file_hash)?;
            edge_cache.insert(blob.file_hash.clone(), loaded.clone());
            loaded
        };

        nodes.extend(
            file_nodes
                .into_iter()
                .filter(|node| node_membership.contains(&node.qualified_name)),
        );
        edges.extend(file_edges.into_iter().filter(|edge| {
            edge_membership.contains(&(
                edge.source_qn.clone(),
                edge.target_qn.clone(),
                edge.kind.clone(),
            ))
        }));
    }

    Ok(SnapshotState {
        snapshot_id: snapshot.snapshot_id,
        commit_sha: snapshot.commit_sha.clone(),
        author_time: commit.author_time,
        files,
        nodes,
        edges,
    })
}

fn decode_node_membership(encoded: &str) -> Vec<String> {
    encoded
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn decode_edge_membership(encoded: &str) -> Vec<(String, String, String)> {
    encoded
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            Some((
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
                parts.next()?.to_owned(),
            ))
        })
        .collect()
}

fn node_signature_hash(node: &HistoricalNode) -> Option<String> {
    if node.params.is_none() && node.return_type.is_none() && node.modifiers.is_none() {
        return None;
    }
    let payload = format!(
        "{}\u{1f}{}\u{1f}{}",
        node.params.as_deref().unwrap_or(""),
        node.return_type.as_deref().unwrap_or(""),
        node.modifiers.as_deref().unwrap_or(""),
    );
    Some(sha256_hex(payload.as_bytes()))
}

fn signature_record_key(record: &NodeSignatureRecord) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        record.file_path,
        record.kind,
        record.params.as_deref().unwrap_or(""),
        record.return_type.as_deref().unwrap_or(""),
        record.modifiers.as_deref().unwrap_or(""),
        record.signature_hash.as_deref().unwrap_or(""),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(crate) fn sorted_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn node_identifier_for_source(value: &str) -> String {
    value.to_owned()
}
