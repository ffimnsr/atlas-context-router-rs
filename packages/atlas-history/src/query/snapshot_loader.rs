use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use atlas_store_sqlite::{HistoricalEdge, HistoricalNode, Store, StoredCommit, StoredSnapshot};

use crate::diff::validate_snapshot_materialization;

use super::types::SnapshotState;

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

pub(crate) fn load_snapshot_catalog(
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

pub(crate) fn load_partial_snapshot_state_for_paths(
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
