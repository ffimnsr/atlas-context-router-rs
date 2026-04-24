use std::collections::BTreeSet;

use atlas_store_sqlite::{HistoricalEdge, HistoricalNode};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default)]
pub struct HistoryEvidence {
    pub snapshot_ids: Vec<i64>,
    pub commit_shas: Vec<String>,
    pub node_identifiers: Vec<String>,
    pub edge_identifiers: Vec<String>,
    pub canonical_file_paths: Vec<String>,
}

pub(crate) fn build_evidence(
    snapshot_ids: impl IntoIterator<Item = i64>,
    commit_shas: impl IntoIterator<Item = String>,
    node_identifiers: impl IntoIterator<Item = String>,
    edge_identifiers: impl IntoIterator<Item = String>,
    canonical_file_paths: impl IntoIterator<Item = String>,
) -> HistoryEvidence {
    HistoryEvidence {
        snapshot_ids: snapshot_ids
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        commit_shas: commit_shas
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        node_identifiers: node_identifiers
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        edge_identifiers: edge_identifiers
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        canonical_file_paths: canonical_file_paths
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

pub(crate) fn node_identifier(qualified_name: &str, kind: &str, file_path: &str) -> String {
    format!("{qualified_name}|{kind}|{file_path}")
}

pub(crate) fn node_identifier_for(node: &HistoricalNode) -> String {
    node_identifier(&node.qualified_name, &node.kind, &node.file_path)
}

pub(crate) fn edge_identifier(
    source_qn: &str,
    target_qn: &str,
    kind: &str,
    file_path: &str,
) -> String {
    format!("{source_qn}|{kind}|{target_qn}|{file_path}")
}

pub(crate) fn edge_identifier_for(edge: &HistoricalEdge) -> String {
    edge_identifier(
        &edge.source_qn,
        &edge.target_qn,
        &edge.kind,
        &edge.file_path,
    )
}
