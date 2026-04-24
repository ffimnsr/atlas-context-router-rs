use anyhow::{Context, Result};
use rusqlite::params;
use serde::Serialize;

use super::Store;

mod lifecycle;
mod membership;
mod prune;
mod reconstruction;

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
