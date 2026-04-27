use std::collections::BTreeSet;

use crate::error::Result;
use atlas_store_sqlite::Store;

use crate::reports::{build_evidence, node_identifier_for};

use super::{
    NodeChangeRecord, NodeFilePathSnapshot, NodeHistoryFindings, NodeHistoryReport,
    NodeHistorySummary, NodeSignatureRecord, NodeSignatureSnapshot, load_snapshot_states,
    node_signature_hash, signature_record_key, sorted_strings,
};

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

            findings.appearances.push(NodeChangeRecord {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                change_kinds: vec!["present".to_owned()],
                file_paths: file_paths.clone(),
                node_identifiers: node_ids.clone(),
                signature_hashes: signature_hashes.clone(),
            });

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
        return Err(crate::error::HistoryError::Other(format!(
            "no historical symbol matches found for {qualified_name}"
        )));
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
