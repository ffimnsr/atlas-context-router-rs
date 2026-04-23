use atlas_core::{AtlasError, Result};
use rusqlite::{Row, params};

use super::Store;

/// Lifecycle state of the graph build for a given repo root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphBuildState {
    Building,
    Built,
    Degraded,
    BuildFailed,
}

impl GraphBuildState {
    fn from_str(s: &str) -> Self {
        match s {
            "building" => Self::Building,
            "degraded" => Self::Degraded,
            "build_failed" => Self::BuildFailed,
            _ => Self::Built,
        }
    }
}

/// Persisted build counters and timestamps for a repo.
#[derive(Debug, Clone)]
pub struct GraphBuildStatus {
    pub repo_root: String,
    pub state: GraphBuildState,
    pub files_discovered: i64,
    pub files_processed: i64,
    pub files_accepted: i64,
    pub files_skipped_by_byte_budget: i64,
    pub files_failed: i64,
    pub bytes_accepted: i64,
    pub bytes_skipped: i64,
    pub nodes_written: i64,
    pub edges_written: i64,
    pub budget_stop_reason: Option<String>,
    pub last_built_at: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

/// Counters provided when finishing a successful or degraded build.
pub struct BuildFinishStats {
    pub state: GraphBuildState,
    pub files_discovered: i64,
    pub files_processed: i64,
    pub files_accepted: i64,
    pub files_skipped_by_byte_budget: i64,
    pub files_failed: i64,
    pub bytes_accepted: i64,
    pub bytes_skipped: i64,
    pub nodes_written: i64,
    pub edges_written: i64,
    pub budget_stop_reason: Option<String>,
}

fn state_as_str(state: &GraphBuildState) -> &'static str {
    match state {
        GraphBuildState::Building => "building",
        GraphBuildState::Built => "built",
        GraphBuildState::Degraded => "degraded",
        GraphBuildState::BuildFailed => "build_failed",
    }
}

fn row_to_build_status(row: &Row<'_>) -> rusqlite::Result<GraphBuildStatus> {
    let state_str: String = row.get(1)?;
    Ok(GraphBuildStatus {
        repo_root: row.get(0)?,
        state: GraphBuildState::from_str(&state_str),
        files_discovered: row.get(2)?,
        files_processed: row.get(3)?,
        files_accepted: row.get(4)?,
        files_skipped_by_byte_budget: row.get(5)?,
        files_failed: row.get(6)?,
        bytes_accepted: row.get(7)?,
        bytes_skipped: row.get(8)?,
        nodes_written: row.get(9)?,
        edges_written: row.get(10)?,
        budget_stop_reason: row.get(11)?,
        last_built_at: row.get(12)?,
        last_error: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

impl Store {
    /// Mark a build/update as in-progress for `repo_root`.
    pub fn begin_build(&self, repo_root: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO graph_build_state
                    (repo_root, state, files_discovered, files_processed, files_accepted,
                     files_skipped_by_byte_budget, files_failed, bytes_accepted, bytes_skipped,
                     nodes_written, edges_written, budget_stop_reason, last_built_at, last_error,
                     updated_at)
                 VALUES (?1, 'building', 0, 0, 0, 0, 0, 0, 0, 0, 0, NULL, NULL, NULL, datetime('now'))
                 ON CONFLICT(repo_root) DO UPDATE SET
                    state            = 'building',
                    files_discovered = 0,
                    files_processed  = 0,
                    files_accepted   = 0,
                    files_skipped_by_byte_budget = 0,
                    files_failed     = 0,
                    bytes_accepted   = 0,
                    bytes_skipped    = 0,
                    nodes_written    = 0,
                    edges_written    = 0,
                    budget_stop_reason = NULL,
                    last_error       = NULL,
                    updated_at       = datetime('now')",
                params![repo_root],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Record a successful build completion with final counters.
    pub fn finish_build(&self, repo_root: &str, stats: BuildFinishStats) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO graph_build_state
                    (repo_root, state, files_discovered, files_processed, files_accepted,
                     files_skipped_by_byte_budget, files_failed, bytes_accepted, bytes_skipped,
                     nodes_written, edges_written, budget_stop_reason, last_built_at, last_error,
                     updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'), NULL, datetime('now'))
                 ON CONFLICT(repo_root) DO UPDATE SET
                    state            = ?2,
                    files_discovered = ?3,
                    files_processed  = ?4,
                    files_accepted   = ?5,
                    files_skipped_by_byte_budget = ?6,
                    files_failed     = ?7,
                    bytes_accepted   = ?8,
                    bytes_skipped    = ?9,
                    nodes_written    = ?10,
                    edges_written    = ?11,
                    budget_stop_reason = ?12,
                    last_built_at    = datetime('now'),
                    last_error       = NULL,
                    updated_at       = datetime('now')",
                params![
                    repo_root,
                    state_as_str(&stats.state),
                    stats.files_discovered,
                    stats.files_processed,
                    stats.files_accepted,
                    stats.files_skipped_by_byte_budget,
                    stats.files_failed,
                    stats.bytes_accepted,
                    stats.bytes_skipped,
                    stats.nodes_written,
                    stats.edges_written,
                    stats.budget_stop_reason,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Record a build failure with an error message.
    pub fn fail_build(&self, repo_root: &str, error: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO graph_build_state
                    (repo_root, state, files_discovered, files_processed, files_failed,
                     nodes_written, edges_written, last_built_at, last_error, updated_at)
                 VALUES (?1, 'build_failed', 0, 0, 0, 0, 0, NULL, ?2, datetime('now'))
                 ON CONFLICT(repo_root) DO UPDATE SET
                    state      = 'build_failed',
                    last_error = ?2,
                    updated_at = datetime('now')",
                params![repo_root, error],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Return the build status for a single repo root, or `None` if no record exists.
    pub fn get_build_status(&self, repo_root: &str) -> Result<Option<GraphBuildStatus>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT repo_root, state, files_discovered, files_processed,
                    files_accepted, files_skipped_by_byte_budget, files_failed,
                    bytes_accepted, bytes_skipped, nodes_written, edges_written,
                    budget_stop_reason, last_built_at, last_error, updated_at
                 FROM graph_build_state
                 WHERE repo_root = ?1",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let mut rows = stmt
            .query_map(params![repo_root], row_to_build_status)
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        match rows.next() {
            Some(Ok(status)) => Ok(Some(status)),
            Some(Err(e)) => Err(AtlasError::Db(e.to_string())),
            None => Ok(None),
        }
    }

    /// Return build statuses for all repos recorded in this database.
    pub fn list_build_statuses(&self) -> Result<Vec<GraphBuildStatus>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT repo_root, state, files_discovered, files_processed,
                    files_accepted, files_skipped_by_byte_budget, files_failed,
                    bytes_accepted, bytes_skipped, nodes_written, edges_written,
                    budget_stop_reason, last_built_at, last_error, updated_at
                 FROM graph_build_state
                 ORDER BY repo_root",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], row_to_build_status)
            .map_err(|e| AtlasError::Db(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}
