use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;

use crate::error::{HistoryError, Result};
use serde::Serialize;

use atlas_store_sqlite::Store;

use crate::git;
use crate::lifecycle::{LifecycleSummary, recompute_lifecycle};

#[derive(Debug, Clone, Serialize, Default)]
pub struct HistoryRetentionPolicy {
    pub keep_all: bool,
    pub keep_latest: Option<usize>,
    pub older_than_days: Option<u64>,
    pub keep_tagged_only: bool,
    pub keep_weekly: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPruneSummary {
    pub policy: HistoryRetentionPolicy,
    pub commits_before: usize,
    pub commits_after: usize,
    pub snapshots_before: usize,
    pub snapshots_after: usize,
    pub deleted_commit_shas: Vec<String>,
    pub deleted_snapshot_ids: Vec<i64>,
    pub reclaimed_file_hashes: u64,
    pub reclaimed_historical_nodes: u64,
    pub reclaimed_historical_edges: u64,
    pub lifecycle: LifecycleSummary,
}

pub fn prune_historical_graph(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    policy: &HistoryRetentionPolicy,
) -> Result<HistoryPruneSummary> {
    let repo_id = store
        .find_repo_id(canonical_root)?
        .ok_or_else(|| anyhow::anyhow!("repo not yet registered for history: {canonical_root}"))?;
    let commits = store.list_commits(repo_id)?;
    let snapshots = store.list_snapshots_ordered(repo_id)?;

    if commits.is_empty() || snapshots.is_empty() {
        return Ok(HistoryPruneSummary {
            policy: policy.clone(),
            commits_before: commits.len(),
            commits_after: commits.len(),
            snapshots_before: snapshots.len(),
            snapshots_after: snapshots.len(),
            deleted_commit_shas: Vec::new(),
            deleted_snapshot_ids: Vec::new(),
            reclaimed_file_hashes: 0,
            reclaimed_historical_nodes: 0,
            reclaimed_historical_edges: 0,
            lifecycle: recompute_lifecycle(canonical_root, store)?,
        });
    }

    if !policy.keep_all
        && policy.keep_latest.is_none()
        && policy.older_than_days.is_none()
        && !policy.keep_tagged_only
        && !policy.keep_weekly
    {
        return Err(HistoryError::InvalidSelector(
            "no history prune retention policy selected".to_owned(),
        ));
    }

    if policy.keep_all {
        return Ok(HistoryPruneSummary {
            policy: policy.clone(),
            commits_before: commits.len(),
            commits_after: commits.len(),
            snapshots_before: snapshots.len(),
            snapshots_after: snapshots.len(),
            deleted_commit_shas: Vec::new(),
            deleted_snapshot_ids: Vec::new(),
            reclaimed_file_hashes: 0,
            reclaimed_historical_nodes: 0,
            reclaimed_historical_edges: 0,
            lifecycle: recompute_lifecycle(canonical_root, store)?,
        });
    }

    let commit_by_sha = commits
        .iter()
        .map(|commit| (commit.commit_sha.clone(), commit))
        .collect::<BTreeMap<_, _>>();
    let mut keep_shas = BTreeSet::new();

    if let Some(keep_latest) = policy.keep_latest {
        for snapshot in snapshots.iter().rev().take(keep_latest) {
            keep_shas.insert(snapshot.commit_sha.clone());
        }
    }

    if let Some(days) = policy.older_than_days {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or_default();
        let cutoff = now.saturating_sub((days as i64).saturating_mul(86_400));
        for commit in &commits {
            if commit.author_time >= cutoff {
                keep_shas.insert(commit.commit_sha.clone());
            }
        }
    }

    if policy.keep_tagged_only {
        for tag in git::list_tag_refs(repo).context("list tag refs for history prune")? {
            keep_shas.insert(tag.commit_sha);
        }
    }

    if policy.keep_weekly {
        let mut latest_per_week = BTreeMap::<i64, String>::new();
        for snapshot in &snapshots {
            let Some(commit) = commit_by_sha.get(&snapshot.commit_sha) else {
                continue;
            };
            let week = commit.author_time.div_euclid(86_400 * 7);
            latest_per_week.insert(week, snapshot.commit_sha.clone());
        }
        keep_shas.extend(latest_per_week.into_values());
    }

    let mut delete_snapshot_ids = Vec::new();
    let mut delete_commit_shas = Vec::new();
    for snapshot in &snapshots {
        if !keep_shas.contains(&snapshot.commit_sha) {
            delete_snapshot_ids.push(snapshot.snapshot_id);
            delete_commit_shas.push(snapshot.commit_sha.clone());
        }
    }

    store.delete_history_snapshots(&delete_snapshot_ids)?;
    store.delete_history_commits(repo_id, &delete_commit_shas)?;
    let (reclaimed_hashes, reclaimed_nodes, reclaimed_edges) =
        store.prune_orphan_historical_file_graphs()?;
    let lifecycle = recompute_lifecycle(canonical_root, store)?;

    let commits_after = store.list_commits(repo_id)?.len();
    let snapshots_after = store.list_snapshots_ordered(repo_id)?.len();

    Ok(HistoryPruneSummary {
        policy: policy.clone(),
        commits_before: commits.len(),
        commits_after,
        snapshots_before: snapshots.len(),
        snapshots_after,
        deleted_commit_shas: delete_commit_shas,
        deleted_snapshot_ids: delete_snapshot_ids,
        reclaimed_file_hashes: reclaimed_hashes,
        reclaimed_historical_nodes: reclaimed_nodes,
        reclaimed_historical_edges: reclaimed_edges,
        lifecycle,
    })
}
