use anyhow::{Context, Result};
use rusqlite::params;

use super::Store;

/// Lightweight commit metadata stored per repo.
#[derive(Debug, Clone)]
pub struct StoredCommit {
    pub commit_sha: String,
    pub repo_id: i64,
    pub parent_sha: Option<String>,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    /// Unix timestamp.
    pub author_time: i64,
    /// Unix timestamp.
    pub committer_time: i64,
    pub subject: String,
    pub message: Option<String>,
    pub indexed_at: String,
}

/// Graph snapshot metadata row.
#[derive(Debug, Clone)]
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

/// File membership row within a snapshot.
#[derive(Debug, Clone)]
pub struct StoredSnapshotFile {
    pub snapshot_id: i64,
    pub file_path: String,
    pub file_hash: String,
    pub language: Option<String>,
    pub size: Option<i64>,
}

/// Summary returned by `atlas history status`.
#[derive(Debug)]
pub struct HistoryStatusSummary {
    pub repo_id: Option<i64>,
    pub indexed_commit_count: i64,
    pub snapshot_count: i64,
    pub latest_commit_sha: Option<String>,
    pub latest_commit_subject: Option<String>,
    pub latest_author_time: Option<i64>,
}

impl Store {
    // ── repo identity ──────────────────────────────────────────────────────────

    /// Return existing `repo_id` for `root_path`, or insert a new row and
    /// return the new id.
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

    /// Return the `repo_id` for `root_path`, or `None` if not registered.
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

    // ── commits ────────────────────────────────────────────────────────────────

    /// Insert or replace a commit metadata row.  The primary key is
    /// `(commit_sha, repo_id)`.
    pub fn upsert_commit(&self, c: &StoredCommit) -> Result<()> {
        let now = chrono_now();
        self.conn.execute(
            "INSERT OR REPLACE INTO commits
                (commit_sha, repo_id, parent_sha, author_name, author_email,
                 author_time, committer_time, subject, message, indexed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                c.commit_sha,
                c.repo_id,
                c.parent_sha,
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

    /// Return a commit row, or `None` if not indexed.
    pub fn find_commit(&self, repo_id: i64, commit_sha: &str) -> Result<Option<StoredCommit>> {
        let mut stmt = self.conn.prepare(
            "SELECT commit_sha, repo_id, parent_sha, author_name, author_email,
                    author_time, committer_time, subject, message, indexed_at
             FROM commits WHERE repo_id = ?1 AND commit_sha = ?2",
        )?;
        let mut rows = stmt.query(params![repo_id, commit_sha])?;
        if let Some(r) = rows.next()? {
            Ok(Some(StoredCommit {
                commit_sha: r.get(0)?,
                repo_id: r.get(1)?,
                parent_sha: r.get(2)?,
                author_name: r.get(3)?,
                author_email: r.get(4)?,
                author_time: r.get(5)?,
                committer_time: r.get(6)?,
                subject: r.get(7)?,
                message: r.get(8)?,
                indexed_at: r.get(9)?,
            }))
        } else {
            Ok(None)
        }
    }

    // ── graph snapshots ────────────────────────────────────────────────────────

    /// Insert a new graph snapshot row. Returns the assigned `snapshot_id`.
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

    /// Return a snapshot for `(repo_id, commit_sha)`, or `None`.
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

    // ── snapshot files ─────────────────────────────────────────────────────────

    /// Bulk-insert file membership rows for a snapshot.
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

    // ── history status summary ─────────────────────────────────────────────────

    /// Build the summary used by `atlas history status`.
    pub fn history_status(&self, root_path: &str) -> Result<HistoryStatusSummary> {
        let repo_id = self.find_repo_id(root_path)?;
        let Some(rid) = repo_id else {
            return Ok(HistoryStatusSummary {
                repo_id: None,
                indexed_commit_count: 0,
                snapshot_count: 0,
                latest_commit_sha: None,
                latest_commit_subject: None,
                latest_author_time: None,
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

        let mut stmt = self.conn.prepare(
            "SELECT commit_sha, subject, author_time FROM commits
             WHERE repo_id = ?1 ORDER BY author_time DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![rid])?;
        let (latest_sha, latest_subject, latest_time) = if let Some(r) = rows.next()? {
            (
                Some(r.get::<_, String>(0)?),
                Some(r.get::<_, String>(1)?),
                Some(r.get::<_, i64>(2)?),
            )
        } else {
            (None, None, None)
        };

        Ok(HistoryStatusSummary {
            repo_id: Some(rid),
            indexed_commit_count: commit_count,
            snapshot_count,
            latest_commit_sha: latest_sha,
            latest_commit_subject: latest_subject,
            latest_author_time: latest_time,
        })
    }
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // RFC 3339-ish compact form stored as TEXT
    format_unix_secs(secs)
}

pub(crate) fn format_unix_secs(secs: u64) -> String {
    // Simple ISO-8601 UTC formatter without a heavy dependency.
    // Using the time crate is available in workspace but we keep this
    // self-contained within the store crate.
    let s = secs;
    let sec = s % 60;
    let min = (s / 60) % 60;
    let hour = (s / 3600) % 24;
    let days = s / 86400;
    // days since 1970-01-01
    let (y, mo, d) = days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Proleptic Gregorian calendar computation.
    let mut year = 1970u64;
    loop {
        let leap = is_leap(year);
        let dy = if leap { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let leap = is_leap(year);
    let months = if leap {
        [31u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &dm in &months {
        if days < dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}
