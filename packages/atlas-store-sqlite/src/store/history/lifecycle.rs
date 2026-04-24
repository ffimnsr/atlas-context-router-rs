use anyhow::{Context, Result};
use rusqlite::params;

use super::{Store, StoredEdgeHistory, StoredNodeHistory};

impl Store {
    pub fn replace_node_history(&self, repo_id: i64, entries: &[StoredNodeHistory]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM node_history WHERE repo_id = ?1",
            params![repo_id],
        )?;
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO node_history
                (repo_id, qualified_name, file_path, kind, signature_hash,
                 first_snapshot_id, last_snapshot_id, first_commit_sha, last_commit_sha,
                 introduction_commit_sha, removal_commit_sha, confidence, evidence_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        )?;
        for entry in entries {
            stmt.execute(params![
                entry.repo_id,
                entry.qualified_name,
                entry.file_path,
                entry.kind,
                entry.signature_hash,
                entry.first_snapshot_id,
                entry.last_snapshot_id,
                entry.first_commit_sha,
                entry.last_commit_sha,
                entry.introduction_commit_sha,
                entry.removal_commit_sha,
                entry.confidence,
                entry.evidence_json,
            ])?;
        }
        Ok(())
    }

    pub fn replace_edge_history(&self, repo_id: i64, entries: &[StoredEdgeHistory]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM edge_history WHERE repo_id = ?1",
            params![repo_id],
        )?;
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO edge_history
                (repo_id, source_qn, target_qn, kind, file_path, metadata_hash,
                 first_snapshot_id, last_snapshot_id, first_commit_sha, last_commit_sha,
                 introduction_commit_sha, removal_commit_sha, confidence, evidence_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        )?;
        for entry in entries {
            stmt.execute(params![
                entry.repo_id,
                entry.source_qn,
                entry.target_qn,
                entry.kind,
                entry.file_path,
                entry.metadata_hash,
                entry.first_snapshot_id,
                entry.last_snapshot_id,
                entry.first_commit_sha,
                entry.last_commit_sha,
                entry.introduction_commit_sha,
                entry.removal_commit_sha,
                entry.confidence,
                entry.evidence_json,
            ])?;
        }
        Ok(())
    }

    pub fn list_node_history(&self, repo_id: i64) -> Result<Vec<StoredNodeHistory>> {
        let mut stmt = self.conn.prepare(
            "SELECT repo_id, qualified_name, file_path, kind, signature_hash,
                    first_snapshot_id, last_snapshot_id, first_commit_sha, last_commit_sha,
                    introduction_commit_sha, removal_commit_sha, confidence, evidence_json
             FROM node_history
             WHERE repo_id = ?1
             ORDER BY qualified_name ASC, file_path ASC, kind ASC",
        )?;
        let rows = stmt.query_map(params![repo_id], |r| {
            Ok(StoredNodeHistory {
                repo_id: r.get(0)?,
                qualified_name: r.get(1)?,
                file_path: r.get(2)?,
                kind: r.get(3)?,
                signature_hash: r.get(4)?,
                first_snapshot_id: r.get(5)?,
                last_snapshot_id: r.get(6)?,
                first_commit_sha: r.get(7)?,
                last_commit_sha: r.get(8)?,
                introduction_commit_sha: r.get(9)?,
                removal_commit_sha: r.get(10)?,
                confidence: r.get(11)?,
                evidence_json: r.get(12)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list node history")
    }

    pub fn list_edge_history(&self, repo_id: i64) -> Result<Vec<StoredEdgeHistory>> {
        let mut stmt = self.conn.prepare(
            "SELECT repo_id, source_qn, target_qn, kind, file_path, metadata_hash,
                    first_snapshot_id, last_snapshot_id, first_commit_sha, last_commit_sha,
                    introduction_commit_sha, removal_commit_sha, confidence, evidence_json
             FROM edge_history
             WHERE repo_id = ?1
             ORDER BY source_qn ASC, target_qn ASC, kind ASC, file_path ASC",
        )?;
        let rows = stmt.query_map(params![repo_id], |r| {
            Ok(StoredEdgeHistory {
                repo_id: r.get(0)?,
                source_qn: r.get(1)?,
                target_qn: r.get(2)?,
                kind: r.get(3)?,
                file_path: r.get(4)?,
                metadata_hash: r.get(5)?,
                first_snapshot_id: r.get(6)?,
                last_snapshot_id: r.get(7)?,
                first_commit_sha: r.get(8)?,
                last_commit_sha: r.get(9)?,
                introduction_commit_sha: r.get(10)?,
                removal_commit_sha: r.get(11)?,
                confidence: r.get(12)?,
                evidence_json: r.get(13)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list edge history")
    }

    pub fn database_size_bytes(&self) -> Result<u64> {
        let page_count: u64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .context("read PRAGMA page_count")?;
        let page_size: u64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .context("read PRAGMA page_size")?;
        Ok(page_count.saturating_mul(page_size))
    }
}
