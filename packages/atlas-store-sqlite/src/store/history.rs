use anyhow::{Context, Result};
use rusqlite::params;
use serde::Serialize;

use super::Store;

#[derive(Debug, Clone, Serialize)]
pub struct StoredCommit {
    pub commit_sha: String,
    pub repo_id: i64,
    pub parent_sha: Option<String>,
    pub indexed_ref: Option<String>,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub author_time: i64,
    pub committer_time: i64,
    pub subject: String,
    pub message: Option<String>,
    pub indexed_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredSnapshot {
    pub snapshot_id: i64,
    pub repo_id: i64,
    pub commit_sha: String,
    pub root_tree_hash: Option<String>,
    pub node_count: i64,
    pub edge_count: i64,
    pub file_count: i64,
    pub created_at: String,
    pub completeness: f64,
    pub parse_error_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredSnapshotFile {
    pub snapshot_id: i64,
    pub file_path: String,
    pub file_hash: String,
    pub language: Option<String>,
    pub size: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredSnapshotMembershipBlob {
    pub snapshot_id: i64,
    pub file_path: String,
    pub file_hash: String,
    pub node_membership: String,
    pub edge_membership: String,
}

#[derive(Debug)]
pub struct HistoryStatusSummary {
    pub repo_id: Option<i64>,
    pub indexed_commit_count: i64,
    pub snapshot_count: i64,
    pub partial_snapshot_count: i64,
    pub parse_error_snapshot_count: i64,
    pub latest_commit_sha: Option<String>,
    pub latest_commit_subject: Option<String>,
    pub latest_author_time: Option<i64>,
    pub latest_indexed_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalNode {
    pub file_hash: String,
    pub qualified_name: String,
    pub kind: String,
    pub name: String,
    pub file_path: String,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub language: Option<String>,
    pub parent_name: Option<String>,
    pub params: Option<String>,
    pub return_type: Option<String>,
    pub modifiers: Option<String>,
    pub is_test: bool,
    pub extra_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalEdge {
    pub file_hash: String,
    pub source_qn: String,
    pub target_qn: String,
    pub kind: String,
    pub file_path: String,
    pub line: Option<i64>,
    pub confidence: f64,
    pub confidence_tier: Option<String>,
    pub extra_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredNodeHistory {
    pub repo_id: i64,
    pub qualified_name: String,
    pub file_path: String,
    pub kind: String,
    pub signature_hash: Option<String>,
    pub first_snapshot_id: i64,
    pub last_snapshot_id: i64,
    pub first_commit_sha: String,
    pub last_commit_sha: String,
    pub introduction_commit_sha: String,
    pub removal_commit_sha: Option<String>,
    pub confidence: f64,
    pub evidence_json: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoredEdgeHistory {
    pub repo_id: i64,
    pub source_qn: String,
    pub target_qn: String,
    pub kind: String,
    pub file_path: String,
    pub metadata_hash: Option<String>,
    pub first_snapshot_id: i64,
    pub last_snapshot_id: i64,
    pub first_commit_sha: String,
    pub last_commit_sha: String,
    pub introduction_commit_sha: String,
    pub removal_commit_sha: Option<String>,
    pub confidence: f64,
    pub evidence_json: Option<String>,
}

impl Store {
    pub fn upsert_repo(&self, root_path: &str) -> Result<i64> {
        let now = chrono_now();
        self.conn
            .execute(
                "INSERT OR IGNORE INTO repos (root_path, created_at) VALUES (?1, ?2)",
                params![root_path, now],
            )
            .context("insert repo row")?;
        let id: i64 = self.conn.query_row(
            "SELECT repo_id FROM repos WHERE root_path = ?1",
            params![root_path],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    pub fn find_repo_id(&self, root_path: &str) -> Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT repo_id FROM repos WHERE root_path = ?1")?;
        let mut rows = stmt.query(params![root_path])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_commit(&self, c: &StoredCommit) -> Result<()> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT OR REPLACE INTO commits
                (commit_sha, repo_id, parent_sha, indexed_ref, author_name, author_email,
                 author_time, committer_time, subject, message, indexed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                c.commit_sha,
                c.repo_id,
                c.parent_sha,
                c.indexed_ref,
                c.author_name,
                c.author_email,
                c.author_time,
                c.committer_time,
                c.subject,
                c.message,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn find_commit(&self, repo_id: i64, commit_sha: &str) -> Result<Option<StoredCommit>> {
        let mut stmt = self.conn.prepare(
            "SELECT commit_sha, repo_id, parent_sha, indexed_ref, author_name, author_email,
                    author_time, committer_time, subject, message, indexed_at
             FROM commits WHERE repo_id = ?1 AND commit_sha = ?2",
        )?;
        let mut rows = stmt.query(params![repo_id, commit_sha])?;
        if let Some(r) = rows.next()? {
            Ok(Some(StoredCommit {
                commit_sha: r.get(0)?,
                repo_id: r.get(1)?,
                parent_sha: r.get(2)?,
                indexed_ref: r.get(3)?,
                author_name: r.get(4)?,
                author_email: r.get(5)?,
                author_time: r.get(6)?,
                committer_time: r.get(7)?,
                subject: r.get(8)?,
                message: r.get(9)?,
                indexed_at: r.get(10)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_commits(&self, repo_id: i64) -> Result<Vec<StoredCommit>> {
        let mut stmt = self.conn.prepare(
            "SELECT commit_sha, repo_id, parent_sha, indexed_ref, author_name, author_email,
                    author_time, committer_time, subject, message, indexed_at
             FROM commits
             WHERE repo_id = ?1
             ORDER BY author_time DESC, commit_sha DESC",
        )?;
        let rows = stmt.query_map(params![repo_id], |r| {
            Ok(StoredCommit {
                commit_sha: r.get(0)?,
                repo_id: r.get(1)?,
                parent_sha: r.get(2)?,
                indexed_ref: r.get(3)?,
                author_name: r.get(4)?,
                author_email: r.get(5)?,
                author_time: r.get(6)?,
                committer_time: r.get(7)?,
                subject: r.get(8)?,
                message: r.get(9)?,
                indexed_at: r.get(10)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list commits")
    }

    pub fn latest_commit_sha(&self, repo_id: i64) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.commit_sha
             FROM commits c
             JOIN graph_snapshots s
               ON s.repo_id = c.repo_id AND s.commit_sha = c.commit_sha
             WHERE c.repo_id = ?1
                         ORDER BY s.snapshot_id DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![repo_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_snapshot(
        &self,
        repo_id: i64,
        commit_sha: &str,
        root_tree_hash: Option<&str>,
        node_count: i64,
        edge_count: i64,
        file_count: i64,
        completeness: f64,
        parse_error_count: i64,
    ) -> Result<i64> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT OR REPLACE INTO graph_snapshots
                (repo_id, commit_sha, root_tree_hash, node_count, edge_count,
                 file_count, created_at, completeness, parse_error_count)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                repo_id,
                commit_sha,
                root_tree_hash,
                node_count,
                edge_count,
                file_count,
                now,
                completeness,
                parse_error_count,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn find_snapshot(&self, repo_id: i64, commit_sha: &str) -> Result<Option<StoredSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT snapshot_id, repo_id, commit_sha, root_tree_hash,
                    node_count, edge_count, file_count, created_at,
                    completeness, parse_error_count
             FROM graph_snapshots WHERE repo_id = ?1 AND commit_sha = ?2",
        )?;
        let mut rows = stmt.query(params![repo_id, commit_sha])?;
        if let Some(r) = rows.next()? {
            Ok(Some(StoredSnapshot {
                snapshot_id: r.get(0)?,
                repo_id: r.get(1)?,
                commit_sha: r.get(2)?,
                root_tree_hash: r.get(3)?,
                node_count: r.get(4)?,
                edge_count: r.get(5)?,
                file_count: r.get(6)?,
                created_at: r.get(7)?,
                completeness: r.get(8)?,
                parse_error_count: r.get(9)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_snapshots_ordered(&self, repo_id: i64) -> Result<Vec<StoredSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.snapshot_id, s.repo_id, s.commit_sha, s.root_tree_hash,
                    s.node_count, s.edge_count, s.file_count, s.created_at,
                    s.completeness, s.parse_error_count
             FROM graph_snapshots s
             JOIN commits c
               ON c.repo_id = s.repo_id AND c.commit_sha = s.commit_sha
             WHERE s.repo_id = ?1
             ORDER BY c.author_time ASC, s.snapshot_id ASC",
        )?;
        let rows = stmt.query_map(params![repo_id], |r| {
            Ok(StoredSnapshot {
                snapshot_id: r.get(0)?,
                repo_id: r.get(1)?,
                commit_sha: r.get(2)?,
                root_tree_hash: r.get(3)?,
                node_count: r.get(4)?,
                edge_count: r.get(5)?,
                file_count: r.get(6)?,
                created_at: r.get(7)?,
                completeness: r.get(8)?,
                parse_error_count: r.get(9)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list ordered snapshots")
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

    pub fn history_status(&self, root_path: &str) -> Result<HistoryStatusSummary> {
        let repo_id = self.find_repo_id(root_path)?;
        let Some(rid) = repo_id else {
            return Ok(HistoryStatusSummary {
                repo_id: None,
                indexed_commit_count: 0,
                snapshot_count: 0,
                partial_snapshot_count: 0,
                parse_error_snapshot_count: 0,
                latest_commit_sha: None,
                latest_commit_subject: None,
                latest_author_time: None,
                latest_indexed_ref: None,
            });
        };

        let commit_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM commits WHERE repo_id = ?1",
            params![rid],
            |r| r.get(0),
        )?;
        let snapshot_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM graph_snapshots WHERE repo_id = ?1",
            params![rid],
            |r| r.get(0),
        )?;
        let partial_snapshot_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM graph_snapshots WHERE repo_id = ?1 AND completeness < 1.0",
            params![rid],
            |r| r.get(0),
        )?;
        let parse_error_snapshot_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM graph_snapshots WHERE repo_id = ?1 AND parse_error_count > 0",
            params![rid],
            |r| r.get(0),
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT commit_sha, subject, author_time, indexed_ref FROM commits
             WHERE repo_id = ?1 ORDER BY author_time DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![rid])?;
        let (latest_sha, latest_subject, latest_time, latest_indexed_ref) =
            if let Some(r) = rows.next()? {
                (
                    Some(r.get::<_, String>(0)?),
                    Some(r.get::<_, String>(1)?),
                    Some(r.get::<_, i64>(2)?),
                    r.get::<_, Option<String>>(3)?,
                )
            } else {
                (None, None, None, None)
            };
        Ok(HistoryStatusSummary {
            repo_id: Some(rid),
            indexed_commit_count: commit_count,
            snapshot_count,
            partial_snapshot_count,
            parse_error_snapshot_count,
            latest_commit_sha: latest_sha,
            latest_commit_subject: latest_subject,
            latest_author_time: latest_time,
            latest_indexed_ref,
        })
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

    pub fn get_historical_file_language(&self, file_hash: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT language FROM historical_nodes WHERE file_hash = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![file_hash])?;
        if let Some(r) = rows.next()? {
            Ok(r.get(0)?)
        } else {
            Ok(None)
        }
    }

    pub fn list_historical_node_qns(&self, file_hash: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT qualified_name FROM historical_nodes WHERE file_hash = ?1")?;
        let rows = stmt.query_map(params![file_hash], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list historical node qns")
    }

    pub fn list_historical_nodes_for_hash(&self, file_hash: &str) -> Result<Vec<HistoricalNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_hash, qualified_name, kind, name, file_path,
                    line_start, line_end, language, parent_name,
                    params, return_type, modifiers, is_test, extra_json
             FROM historical_nodes
             WHERE file_hash = ?1
             ORDER BY file_path ASC, qualified_name ASC",
        )?;
        let rows = stmt.query_map(params![file_hash], |r| {
            Ok(HistoricalNode {
                file_hash: r.get(0)?,
                qualified_name: r.get(1)?,
                kind: r.get(2)?,
                name: r.get(3)?,
                file_path: r.get(4)?,
                line_start: r.get(5)?,
                line_end: r.get(6)?,
                language: r.get(7)?,
                parent_name: r.get(8)?,
                params: r.get(9)?,
                return_type: r.get(10)?,
                modifiers: r.get(11)?,
                is_test: r.get::<_, i64>(12)? != 0,
                extra_json: r.get(13)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list historical nodes for hash")
    }

    pub fn list_historical_edge_keys(
        &self,
        file_hash: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_qn, target_qn, kind FROM historical_edges WHERE file_hash = ?1",
        )?;
        let rows = stmt.query_map(params![file_hash], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list historical edge keys")
    }

    pub fn list_historical_edges_for_hash(&self, file_hash: &str) -> Result<Vec<HistoricalEdge>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_hash, source_qn, target_qn, kind, file_path,
                    line, confidence, confidence_tier, extra_json
             FROM historical_edges
             WHERE file_hash = ?1
             ORDER BY file_path ASC, source_qn ASC, target_qn ASC, kind ASC",
        )?;
        let rows = stmt.query_map(params![file_hash], |r| {
            Ok(HistoricalEdge {
                file_hash: r.get(0)?,
                source_qn: r.get(1)?,
                target_qn: r.get(2)?,
                kind: r.get(3)?,
                file_path: r.get(4)?,
                line: r.get(5)?,
                confidence: r.get(6)?,
                confidence_tier: r.get(7)?,
                extra_json: r.get(8)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list historical edges for hash")
    }

    pub fn reconstruct_snapshot_nodes(&self, snapshot_id: i64) -> Result<Vec<HistoricalNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT hn.file_hash, hn.qualified_name, hn.kind, hn.name, hn.file_path,
                    hn.line_start, hn.line_end, hn.language, hn.parent_name,
                    hn.params, hn.return_type, hn.modifiers, hn.is_test, hn.extra_json
             FROM snapshot_nodes sn
             JOIN historical_nodes hn
               ON hn.file_hash = sn.file_hash
              AND hn.qualified_name = sn.qualified_name
             WHERE sn.snapshot_id = ?1
             ORDER BY hn.file_path ASC, hn.qualified_name ASC",
        )?;
        let rows = stmt.query_map(params![snapshot_id], |r| {
            Ok(HistoricalNode {
                file_hash: r.get(0)?,
                qualified_name: r.get(1)?,
                kind: r.get(2)?,
                name: r.get(3)?,
                file_path: r.get(4)?,
                line_start: r.get(5)?,
                line_end: r.get(6)?,
                language: r.get(7)?,
                parent_name: r.get(8)?,
                params: r.get(9)?,
                return_type: r.get(10)?,
                modifiers: r.get(11)?,
                is_test: r.get::<_, i64>(12)? != 0,
                extra_json: r.get(13)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("reconstruct snapshot nodes")
    }

    pub fn reconstruct_snapshot_edges(&self, snapshot_id: i64) -> Result<Vec<HistoricalEdge>> {
        let mut stmt = self.conn.prepare(
            "SELECT he.file_hash, he.source_qn, he.target_qn, he.kind, he.file_path,
                    he.line, he.confidence, he.confidence_tier, he.extra_json
             FROM snapshot_edges se
             JOIN historical_edges he
               ON he.file_hash = se.file_hash
              AND he.source_qn = se.source_qn
              AND he.target_qn = se.target_qn
              AND he.kind = se.kind
             WHERE se.snapshot_id = ?1
             ORDER BY he.file_path ASC, he.source_qn ASC, he.target_qn ASC, he.kind ASC",
        )?;
        let rows = stmt.query_map(params![snapshot_id], |r| {
            Ok(HistoricalEdge {
                file_hash: r.get(0)?,
                source_qn: r.get(1)?,
                target_qn: r.get(2)?,
                kind: r.get(3)?,
                file_path: r.get(4)?,
                line: r.get(5)?,
                confidence: r.get(6)?,
                confidence_tier: r.get(7)?,
                extra_json: r.get(8)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("reconstruct snapshot edges")
    }

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

    pub fn delete_history_snapshots(&self, snapshot_ids: &[i64]) -> Result<()> {
        if snapshot_ids.is_empty() {
            return Ok(());
        }

        let tx = self.conn.unchecked_transaction()?;
        {
            let mut delete_files =
                tx.prepare_cached("DELETE FROM snapshot_files WHERE snapshot_id = ?1")?;
            let mut delete_snapshots =
                tx.prepare_cached("DELETE FROM graph_snapshots WHERE snapshot_id = ?1")?;
            for snapshot_id in snapshot_ids {
                delete_files.execute(params![snapshot_id])?;
                delete_snapshots.execute(params![snapshot_id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_history_commits(&self, repo_id: i64, commit_shas: &[String]) -> Result<()> {
        if commit_shas.is_empty() {
            return Ok(());
        }

        let tx = self.conn.unchecked_transaction()?;
        {
            let mut delete_commit =
                tx.prepare_cached("DELETE FROM commits WHERE repo_id = ?1 AND commit_sha = ?2")?;
            for commit_sha in commit_shas {
                delete_commit.execute(params![repo_id, commit_sha])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn prune_orphan_historical_file_graphs(&self) -> Result<(u64, u64, u64)> {
        let orphan_hashes = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT hn.file_hash
                 FROM historical_nodes hn
                 LEFT JOIN snapshot_files sf ON sf.file_hash = hn.file_hash
                 WHERE sf.file_hash IS NULL
                 ORDER BY hn.file_hash ASC",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if orphan_hashes.is_empty() {
            return Ok((0, 0, 0));
        }

        let tx = self.conn.unchecked_transaction()?;
        let mut removed_hashes = 0u64;
        let mut removed_nodes = 0u64;
        let mut removed_edges = 0u64;
        {
            let mut count_nodes =
                tx.prepare_cached("SELECT COUNT(*) FROM historical_nodes WHERE file_hash = ?1")?;
            let mut count_edges =
                tx.prepare_cached("SELECT COUNT(*) FROM historical_edges WHERE file_hash = ?1")?;
            let mut delete_nodes =
                tx.prepare_cached("DELETE FROM historical_nodes WHERE file_hash = ?1")?;
            let mut delete_edges =
                tx.prepare_cached("DELETE FROM historical_edges WHERE file_hash = ?1")?;
            for file_hash in orphan_hashes {
                let node_count: u64 =
                    count_nodes.query_row(params![&file_hash], |row| row.get(0))?;
                let edge_count: u64 =
                    count_edges.query_row(params![&file_hash], |row| row.get(0))?;
                delete_nodes.execute(params![&file_hash])?;
                delete_edges.execute(params![&file_hash])?;
                removed_hashes += 1;
                removed_nodes += node_count;
                removed_edges += edge_count;
            }
        }
        tx.commit()?;
        Ok((removed_hashes, removed_nodes, removed_edges))
    }
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format_unix_secs(secs)
}

pub(crate) fn format_unix_secs(secs: u64) -> String {
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let span = if is_leap(year) { 366 } else { 365 };
        if days < span {
            break;
        }
        days -= span;
        year += 1;
    }
    let months = if is_leap(year) {
        [31u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &days_in_month in &months {
        if days < days_in_month {
            break;
        }
        days -= days_in_month;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}
