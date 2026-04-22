use rusqlite::params;

use atlas_core::{AtlasError, Result};

use super::util::format_now;
use super::{ContentStore, IndexState, IndexingStats, RetrievalIndexStatus};

impl ContentStore {
    /// Begin an indexing run for `repo_root`.
    pub fn begin_indexing(&mut self, repo_root: &str, files_discovered: i64) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO retrieval_index_state
                     (repo_root, state, files_discovered, files_indexed,
                      chunks_written, chunks_reused, last_indexed_at, last_error, updated_at)
                 VALUES (?1, 'indexing', ?2, 0, 0, 0, NULL, NULL, ?3)
                 ON CONFLICT(repo_root) DO UPDATE SET
                     state            = 'indexing',
                     files_discovered = ?2,
                     files_indexed    = 0,
                     chunks_written   = 0,
                     chunks_reused    = 0,
                     last_error       = NULL,
                     updated_at       = ?3",
                params![repo_root, files_discovered, now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Mark indexing run as successfully completed.
    pub fn finish_indexing(&mut self, repo_root: &str, stats: &IndexingStats) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO retrieval_index_state
                     (repo_root, state, files_discovered, files_indexed,
                      chunks_written, chunks_reused, last_indexed_at, last_error, updated_at)
                 VALUES (?1, 'indexed', 0, ?2, ?3, ?4, ?5, NULL, ?5)
                 ON CONFLICT(repo_root) DO UPDATE SET
                     state           = 'indexed',
                     files_indexed   = ?2,
                     chunks_written  = ?3,
                     chunks_reused   = ?4,
                     last_indexed_at = ?5,
                     last_error      = NULL,
                     updated_at      = ?5",
                params![
                    repo_root,
                    stats.files_indexed,
                    stats.chunks_written,
                    stats.chunks_reused,
                    now,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Mark indexing run as failed, recording error reason.
    pub fn fail_indexing(&mut self, repo_root: &str, error: &str) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO retrieval_index_state
                     (repo_root, state, files_discovered, files_indexed,
                      chunks_written, chunks_reused, last_indexed_at, last_error, updated_at)
                 VALUES (?1, 'index_failed', 0, 0, 0, 0, NULL, ?2, ?3)
                 ON CONFLICT(repo_root) DO UPDATE SET
                     state      = 'index_failed',
                     last_error = ?2,
                     updated_at = ?3",
                params![repo_root, error, now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Return current index status for `repo_root`, or `None` if not recorded.
    pub fn get_index_status(&self, repo_root: &str) -> Result<Option<RetrievalIndexStatus>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT repo_root, state, files_discovered, files_indexed,
                        chunks_written, chunks_reused, last_indexed_at, last_error, updated_at
                 FROM retrieval_index_state WHERE repo_root = ?1",
            )
            .map_err(db_err)?;
        let mut rows = stmt.query(params![repo_root]).map_err(db_err)?;
        if let Some(row) = rows.next().map_err(db_err)? {
            Ok(Some(Self::row_to_status(row)?))
        } else {
            Ok(None)
        }
    }

    /// Return status rows for all repos that have ever been indexed.
    pub fn list_index_statuses(&self) -> Result<Vec<RetrievalIndexStatus>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT repo_root, state, files_discovered, files_indexed,
                        chunks_written, chunks_reused, last_indexed_at, last_error, updated_at
                 FROM retrieval_index_state ORDER BY updated_at DESC",
            )
            .map_err(db_err)?;
        let mut rows = stmt.query([]).map_err(db_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(db_err)? {
            out.push(Self::row_to_status(row)?);
        }
        Ok(out)
    }

    fn row_to_status(row: &rusqlite::Row<'_>) -> Result<RetrievalIndexStatus> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        Ok(RetrievalIndexStatus {
            repo_root: row.get::<_, String>(0).map_err(db_err)?,
            state: IndexState::from_str(&row.get::<_, String>(1).map_err(db_err)?),
            files_discovered: row.get::<_, i64>(2).map_err(db_err)?,
            files_indexed: row.get::<_, i64>(3).map_err(db_err)?,
            chunks_written: row.get::<_, i64>(4).map_err(db_err)?,
            chunks_reused: row.get::<_, i64>(5).map_err(db_err)?,
            last_indexed_at: row.get::<_, Option<String>>(6).map_err(db_err)?,
            last_error: row.get::<_, Option<String>>(7).map_err(db_err)?,
            updated_at: row.get::<_, String>(8).map_err(db_err)?,
        })
    }

    /// Increment `chunks_written` and `files_indexed` counters for `repo_root`.
    pub(super) fn increment_index_counters_with_reuse(
        &mut self,
        repo_root: &str,
        chunks_written: i64,
        chunks_reused: i64,
    ) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT OR IGNORE INTO retrieval_index_state
                     (repo_root, state, files_discovered, files_indexed,
                      chunks_written, chunks_reused, last_indexed_at, last_error, updated_at)
                 VALUES (?1, 'indexed', 0, 0, 0, 0, NULL, NULL, ?2)",
                params![repo_root, now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.conn
            .execute(
                "UPDATE retrieval_index_state
                 SET files_indexed  = files_indexed + 1,
                     chunks_written = chunks_written + ?2,
                     chunks_reused  = chunks_reused  + ?3,
                     updated_at     = ?4
                 WHERE repo_root = ?1",
                params![repo_root, chunks_written, chunks_reused, now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }
}
