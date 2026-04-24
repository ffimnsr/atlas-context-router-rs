use std::collections::BTreeMap;

use anyhow::{Context, Result};
use atlas_store_sqlite::{
    HistoricalEdge, HistoricalNode, Store, StoredEdgeHistory, StoredNodeHistory,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Default)]
pub struct LifecycleSummary {
    pub repo_id: i64,
    pub snapshot_count: usize,
    pub node_history_rows: usize,
    pub edge_history_rows: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct NodeIdentity {
    qualified_name: String,
    file_path: String,
    kind: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
struct EdgeIdentity {
    source_qn: String,
    target_qn: String,
    kind: String,
    file_path: String,
}

#[derive(Debug, Clone)]
struct NodeAccumulator {
    signature_hash: Option<String>,
    first_snapshot_id: i64,
    last_snapshot_id: i64,
    first_commit_sha: String,
    last_commit_sha: String,
    first_index: usize,
    last_index: usize,
    presence_count: usize,
}

#[derive(Debug, Clone)]
struct EdgeAccumulator {
    metadata_hash: Option<String>,
    first_snapshot_id: i64,
    last_snapshot_id: i64,
    first_commit_sha: String,
    last_commit_sha: String,
    first_index: usize,
    last_index: usize,
    presence_count: usize,
}

pub fn recompute_lifecycle(canonical_root: &str, store: &Store) -> Result<LifecycleSummary> {
    let repo_id = store
        .find_repo_id(canonical_root)?
        .ok_or_else(|| anyhow::anyhow!("repo not yet registered for history: {canonical_root}"))?;
    let snapshots = store.list_snapshots_ordered(repo_id)?;
    if snapshots.is_empty() {
        return Ok(LifecycleSummary {
            repo_id,
            ..LifecycleSummary::default()
        });
    }

    let mut node_map: BTreeMap<NodeIdentity, NodeAccumulator> = BTreeMap::new();
    let mut edge_map: BTreeMap<EdgeIdentity, EdgeAccumulator> = BTreeMap::new();

    for (index, snapshot) in snapshots.iter().enumerate() {
        for node in store.reconstruct_snapshot_nodes(snapshot.snapshot_id)? {
            let key = NodeIdentity {
                qualified_name: node.qualified_name.clone(),
                file_path: node.file_path.clone(),
                kind: node.kind.clone(),
            };
            let signature_hash = node_signature_hash(&node);
            node_map
                .entry(key)
                .and_modify(|acc| {
                    acc.last_snapshot_id = snapshot.snapshot_id;
                    acc.last_commit_sha = snapshot.commit_sha.clone();
                    acc.last_index = index;
                    acc.signature_hash = signature_hash.clone();
                    acc.presence_count += 1;
                })
                .or_insert(NodeAccumulator {
                    signature_hash,
                    first_snapshot_id: snapshot.snapshot_id,
                    last_snapshot_id: snapshot.snapshot_id,
                    first_commit_sha: snapshot.commit_sha.clone(),
                    last_commit_sha: snapshot.commit_sha.clone(),
                    first_index: index,
                    last_index: index,
                    presence_count: 1,
                });
        }

        for edge in store.reconstruct_snapshot_edges(snapshot.snapshot_id)? {
            let key = EdgeIdentity {
                source_qn: edge.source_qn.clone(),
                target_qn: edge.target_qn.clone(),
                kind: edge.kind.clone(),
                file_path: edge.file_path.clone(),
            };
            let metadata_hash = edge_metadata_hash(&edge);
            edge_map
                .entry(key)
                .and_modify(|acc| {
                    acc.last_snapshot_id = snapshot.snapshot_id;
                    acc.last_commit_sha = snapshot.commit_sha.clone();
                    acc.last_index = index;
                    acc.metadata_hash = metadata_hash.clone();
                    acc.presence_count += 1;
                })
                .or_insert(EdgeAccumulator {
                    metadata_hash,
                    first_snapshot_id: snapshot.snapshot_id,
                    last_snapshot_id: snapshot.snapshot_id,
                    first_commit_sha: snapshot.commit_sha.clone(),
                    last_commit_sha: snapshot.commit_sha.clone(),
                    first_index: index,
                    last_index: index,
                    presence_count: 1,
                });
        }
    }

    let node_rows = node_map
        .into_iter()
        .map(|(key, acc)| {
            let removal_commit_sha = snapshots
                .get(acc.last_index + 1)
                .map(|snapshot| snapshot.commit_sha.clone());
            let evidence_json = serde_json::json!({
                "presence_count": acc.presence_count,
                "first_index": acc.first_index,
                "last_index": acc.last_index,
            })
            .to_string();
            StoredNodeHistory {
                repo_id,
                qualified_name: key.qualified_name,
                file_path: key.file_path,
                kind: key.kind,
                signature_hash: acc.signature_hash,
                first_snapshot_id: acc.first_snapshot_id,
                last_snapshot_id: acc.last_snapshot_id,
                first_commit_sha: acc.first_commit_sha.clone(),
                last_commit_sha: acc.last_commit_sha.clone(),
                introduction_commit_sha: acc.first_commit_sha,
                removal_commit_sha,
                confidence: 1.0,
                evidence_json: Some(evidence_json),
            }
        })
        .collect::<Vec<_>>();

    let edge_rows = edge_map
        .into_iter()
        .map(|(key, acc)| {
            let removal_commit_sha = snapshots
                .get(acc.last_index + 1)
                .map(|snapshot| snapshot.commit_sha.clone());
            let evidence_json = serde_json::json!({
                "presence_count": acc.presence_count,
                "first_index": acc.first_index,
                "last_index": acc.last_index,
            })
            .to_string();
            StoredEdgeHistory {
                repo_id,
                source_qn: key.source_qn,
                target_qn: key.target_qn,
                kind: key.kind,
                file_path: key.file_path,
                metadata_hash: acc.metadata_hash,
                first_snapshot_id: acc.first_snapshot_id,
                last_snapshot_id: acc.last_snapshot_id,
                first_commit_sha: acc.first_commit_sha.clone(),
                last_commit_sha: acc.last_commit_sha.clone(),
                introduction_commit_sha: acc.first_commit_sha,
                removal_commit_sha,
                confidence: 1.0,
                evidence_json: Some(evidence_json),
            }
        })
        .collect::<Vec<_>>();

    store
        .replace_node_history(repo_id, &node_rows)
        .context("replace node history")?;
    store
        .replace_edge_history(repo_id, &edge_rows)
        .context("replace edge history")?;

    Ok(LifecycleSummary {
        repo_id,
        snapshot_count: snapshots.len(),
        node_history_rows: node_rows.len(),
        edge_history_rows: edge_rows.len(),
    })
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
    Some(hex_hash(payload.as_bytes()))
}

fn edge_metadata_hash(edge: &HistoricalEdge) -> Option<String> {
    let payload = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{:.6}",
        edge.line.unwrap_or_default(),
        edge.confidence_tier.as_deref().unwrap_or(""),
        edge.extra_json.as_deref().unwrap_or(""),
        edge.confidence,
    );
    if payload == format!("0\u{1f}\u{1f}\u{1f}{:.6}", 1.0) {
        return None;
    }
    Some(hex_hash(payload.as_bytes()))
}

fn hex_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use atlas_store_sqlite::{HistoricalEdge, HistoricalNode, Store, StoredCommit};

    use super::*;

    fn open_store() -> (tempfile::TempDir, String, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("history.sqlite");
        let db = db_path.to_string_lossy().into_owned();
        let store = Store::open(&db).expect("open store");
        (dir, db, store)
    }

    fn sample_node(file_hash: &str, qn: &str, params: Option<&str>) -> HistoricalNode {
        HistoricalNode {
            file_hash: file_hash.to_owned(),
            qualified_name: qn.to_owned(),
            kind: "function".to_owned(),
            name: qn.rsplit("::").next().unwrap_or(qn).to_owned(),
            file_path: "src/lib.rs".to_owned(),
            line_start: Some(1),
            line_end: Some(10),
            language: Some("rust".to_owned()),
            parent_name: None,
            params: params.map(str::to_owned),
            return_type: Some("i32".to_owned()),
            modifiers: None,
            is_test: false,
            extra_json: None,
        }
    }

    fn sample_edge(file_hash: &str, src: &str, tgt: &str) -> HistoricalEdge {
        HistoricalEdge {
            file_hash: file_hash.to_owned(),
            source_qn: src.to_owned(),
            target_qn: tgt.to_owned(),
            kind: "calls".to_owned(),
            file_path: "src/lib.rs".to_owned(),
            line: Some(3),
            confidence: 1.0,
            confidence_tier: Some("definite".to_owned()),
            extra_json: None,
        }
    }

    fn insert_snapshot(
        store: &Store,
        repo_id: i64,
        sha: &str,
        author_time: i64,
        nodes: &[HistoricalNode],
        edges: &[HistoricalEdge],
    ) {
        store
            .upsert_commit(&StoredCommit {
                commit_sha: sha.to_owned(),
                repo_id,
                parent_sha: None,
                indexed_ref: None,
                author_name: Some("Atlas Test".to_owned()),
                author_email: Some("test@atlas".to_owned()),
                author_time,
                committer_time: author_time,
                subject: sha.to_owned(),
                message: None,
                indexed_at: String::new(),
            })
            .expect("upsert commit");
        for node in nodes {
            store
                .insert_historical_nodes(std::slice::from_ref(node))
                .expect("insert node");
        }
        for edge in edges {
            store
                .insert_historical_edges(std::slice::from_ref(edge))
                .expect("insert edge");
        }
        let snapshot_id = store
            .insert_snapshot(
                repo_id,
                sha,
                None,
                nodes.len() as i64,
                edges.len() as i64,
                1,
                1.0,
                0,
            )
            .expect("insert snapshot");
        let file_hash = nodes
            .first()
            .map(|node| node.file_hash.clone())
            .or_else(|| edges.first().map(|edge| edge.file_hash.clone()))
            .unwrap_or_else(|| "hash".repeat(10));
        store
            .insert_snapshot_files(&[atlas_store_sqlite::StoredSnapshotFile {
                snapshot_id,
                file_path: "src/lib.rs".to_owned(),
                file_hash: file_hash.clone(),
                language: Some("rust".to_owned()),
                size: Some(32),
            }])
            .expect("insert files");
        let qns = nodes
            .iter()
            .map(|node| node.qualified_name.clone())
            .collect::<Vec<_>>();
        let edge_keys = edges
            .iter()
            .map(|edge| {
                (
                    edge.source_qn.clone(),
                    edge.target_qn.clone(),
                    edge.kind.clone(),
                )
            })
            .collect::<Vec<_>>();
        store
            .attach_snapshot_nodes(snapshot_id, &file_hash, &qns)
            .expect("attach nodes");
        store
            .attach_snapshot_edges(snapshot_id, &file_hash, &edge_keys)
            .expect("attach edges");
    }

    #[test]
    fn recompute_lifecycle_sets_removal_commit_to_next_absent_snapshot() {
        let (_dir, _db, store) = open_store();
        let root = Path::new("/repo");
        let repo_id = store
            .upsert_repo(root.to_string_lossy().as_ref())
            .expect("repo");

        insert_snapshot(
            &store,
            repo_id,
            &"1".repeat(40),
            1,
            &[sample_node(&"a".repeat(40), "crate::alpha", Some("()"))],
            &[],
        );
        insert_snapshot(
            &store,
            repo_id,
            &"2".repeat(40),
            2,
            &[sample_node(&"b".repeat(40), "crate::beta", Some("()"))],
            &[],
        );

        let summary =
            recompute_lifecycle(root.to_string_lossy().as_ref(), &store).expect("lifecycle");
        assert_eq!(summary.snapshot_count, 2);
        let rows = store.list_node_history(repo_id).expect("node history");
        let alpha = rows
            .iter()
            .find(|row| row.qualified_name == "crate::alpha")
            .expect("alpha row");
        assert_eq!(alpha.introduction_commit_sha, "1".repeat(40));
        let expected_removal = "2".repeat(40);
        assert_eq!(
            alpha.removal_commit_sha.as_deref(),
            Some(expected_removal.as_str())
        );
    }

    #[test]
    fn recompute_lifecycle_persists_edge_rows() {
        let (_dir, _db, store) = open_store();
        let root = "/repo2";
        let repo_id = store.upsert_repo(root).expect("repo");
        let hash = "c".repeat(40);
        let node_a = sample_node(&hash, "crate::a", Some("()"));
        let node_b = sample_node(&hash, "crate::b", Some("()"));
        let edge = sample_edge(&hash, "crate::a", "crate::b");
        insert_snapshot(
            &store,
            repo_id,
            &"3".repeat(40),
            3,
            &[node_a, node_b],
            &[edge],
        );

        let summary = recompute_lifecycle(root, &store).expect("lifecycle");
        assert_eq!(summary.edge_history_rows, 1);
        let edges = store.list_edge_history(repo_id).expect("edge history");
        assert_eq!(edges[0].source_qn, "crate::a");
        assert_eq!(edges[0].target_qn, "crate::b");
        assert!(edges[0].metadata_hash.is_some());
    }
}
