use std::collections::BTreeSet;
use std::path::Path;

use crate::error::Result;
use atlas_store_sqlite::Store;

use crate::git;
use crate::reports::{build_evidence, edge_identifier_for, node_identifier_for};

use super::{
    FileHistoryFindings, FileHistoryPoint, FileHistoryReport, FileHistorySummary,
    load_partial_snapshot_state_for_paths, load_snapshot_catalog,
};

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
        return Err(crate::error::HistoryError::Other(format!(
            "no historical file matches found for {file_path}"
        )));
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
