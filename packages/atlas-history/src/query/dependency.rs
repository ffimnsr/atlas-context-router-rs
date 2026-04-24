use std::collections::BTreeSet;

use anyhow::Result;
use atlas_store_sqlite::Store;

use crate::reports::{build_evidence, edge_identifier_for};

use super::{
    DependencyHistoryPoint, EdgeHistoryFindings, EdgeHistoryReport, EdgeHistorySummary,
    load_snapshot_states, node_identifier_for_source,
};

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
