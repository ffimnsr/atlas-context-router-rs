use anyhow::{Context, Result};
use rusqlite::params;

use super::{
    HistoricalEdge, HistoricalNode, Store, StoredSnapshotFile, StoredSnapshotMembershipBlob,
};

impl Store {
    /// Begin an explicit `IMMEDIATE` write transaction.
    ///
    /// Used by callers that batch many history inserts into a single commit so
    /// SQLite does not auto-commit (fsync) after every statement.  The caller
    /// **must** call [`Self::commit_write`] on success or [`Self::rollback_write`] on any
    /// error; failing to do so leaves the connection in an open transaction.
    pub fn begin_write(&self) -> Result<()> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .context("BEGIN IMMEDIATE")
    }

    /// Commit the write transaction opened by [`Self::begin_write`].
    pub fn commit_write(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT").context("COMMIT")
    }

    /// Roll back the write transaction opened by [`Self::begin_write`].  Ignores
    /// errors since this is typically called from error paths where the
    /// original error is already propagated.
    pub fn rollback_write(&self) {
        let _ = self.conn.execute_batch("ROLLBACK");
    }

    pub fn insert_snapshot_files(&self, files: &[StoredSnapshotFile]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR REPLACE INTO snapshot_files
                (snapshot_id, file_path, file_hash, language, size)
             VALUES (?1,?2,?3,?4,?5)",
        )?;
        for f in files {
            stmt.execute(params![
                f.snapshot_id,
                f.file_path,
                f.file_hash,
                f.language,
                f.size
            ])?;
        }
        Ok(())
    }

    pub fn list_snapshot_files(&self, snapshot_id: i64) -> Result<Vec<StoredSnapshotFile>> {
        let mut stmt = self.conn.prepare(
            "SELECT snapshot_id, file_path, file_hash, language, size
             FROM snapshot_files
             WHERE snapshot_id = ?1
             ORDER BY file_path ASC",
        )?;
        let rows = stmt.query_map(params![snapshot_id], |r| {
            Ok(StoredSnapshotFile {
                snapshot_id: r.get(0)?,
                file_path: r.get(1)?,
                file_hash: r.get(2)?,
                language: r.get(3)?,
                size: r.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list snapshot files")
    }

    pub fn insert_snapshot_membership_blobs(
        &self,
        blobs: &[StoredSnapshotMembershipBlob],
    ) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR REPLACE INTO snapshot_membership_blobs
                (snapshot_id, file_path, file_hash, node_membership, edge_membership)
             VALUES (?1,?2,?3,?4,?5)",
        )?;
        for blob in blobs {
            stmt.execute(params![
                blob.snapshot_id,
                blob.file_path,
                blob.file_hash,
                blob.node_membership,
                blob.edge_membership,
            ])?;
        }
        Ok(())
    }

    pub fn list_snapshot_membership_blobs(
        &self,
        snapshot_id: i64,
    ) -> Result<Vec<StoredSnapshotMembershipBlob>> {
        let mut stmt = self.conn.prepare(
            "SELECT snapshot_id, file_path, file_hash, node_membership, edge_membership
             FROM snapshot_membership_blobs
             WHERE snapshot_id = ?1
             ORDER BY file_path ASC",
        )?;
        let rows = stmt.query_map(params![snapshot_id], |r| {
            Ok(StoredSnapshotMembershipBlob {
                snapshot_id: r.get(0)?,
                file_path: r.get(1)?,
                file_hash: r.get(2)?,
                node_membership: r.get(3)?,
                edge_membership: r.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list snapshot membership blobs")
    }

    pub fn has_historical_file_graph(&self, file_hash: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM historical_nodes WHERE file_hash = ?1 LIMIT 1",
            params![file_hash],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn insert_historical_nodes(&self, nodes: &[HistoricalNode]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR IGNORE INTO historical_nodes
                (file_hash, qualified_name, kind, name, file_path,
                 line_start, line_end, language, parent_name, params,
                 return_type, modifiers, is_test, extra_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        )?;
        for n in nodes {
            stmt.execute(params![
                n.file_hash,
                n.qualified_name,
                n.kind,
                n.name,
                n.file_path,
                n.line_start,
                n.line_end,
                n.language,
                n.parent_name,
                n.params,
                n.return_type,
                n.modifiers,
                n.is_test as i64,
                n.extra_json,
            ])?;
        }
        Ok(())
    }

    pub fn insert_historical_edges(&self, edges: &[HistoricalEdge]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR IGNORE INTO historical_edges
                (file_hash, source_qn, target_qn, kind, file_path,
                 line, confidence, confidence_tier, extra_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        )?;
        for e in edges {
            stmt.execute(params![
                e.file_hash,
                e.source_qn,
                e.target_qn,
                e.kind,
                e.file_path,
                e.line,
                e.confidence,
                e.confidence_tier,
                e.extra_json,
            ])?;
        }
        Ok(())
    }

    pub fn count_historical_nodes(&self, file_hash: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM historical_nodes WHERE file_hash = ?1",
            params![file_hash],
            |r| r.get(0),
        )?;
        Ok(count)
    }

    pub fn count_historical_edges(&self, file_hash: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM historical_edges WHERE file_hash = ?1",
            params![file_hash],
            |r| r.get(0),
        )?;
        Ok(count)
    }

    pub fn attach_snapshot_nodes(
        &self,
        snapshot_id: i64,
        file_hash: &str,
        qualified_names: &[String],
    ) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR IGNORE INTO snapshot_nodes
                (snapshot_id, file_hash, qualified_name)
             VALUES (?1,?2,?3)",
        )?;
        for qn in qualified_names {
            stmt.execute(params![snapshot_id, file_hash, qn])?;
        }
        Ok(())
    }

    pub fn attach_snapshot_edges(
        &self,
        snapshot_id: i64,
        file_hash: &str,
        edges: &[(String, String, String)],
    ) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR IGNORE INTO snapshot_edges
                (snapshot_id, file_hash, source_qn, target_qn, kind)
             VALUES (?1,?2,?3,?4,?5)",
        )?;
        for (src, tgt, kind) in edges {
            stmt.execute(params![snapshot_id, file_hash, src, tgt, kind])?;
        }
        Ok(())
    }

    pub fn count_snapshot_nodes(&self, snapshot_id: i64) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM snapshot_nodes WHERE snapshot_id = ?1",
            params![snapshot_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn count_snapshot_edges(&self, snapshot_id: i64) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM snapshot_edges WHERE snapshot_id = ?1",
            params![snapshot_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }
}
