//! Planner maintenance for the graph database.
//!
//! Index quality is only half of query speed: without `sqlite_stat1` the
//! planner falls back to hard-coded heuristics and can pick a full scan over a
//! selective index. Full builds and postprocess runs change table sizes enough
//! that the statistics should be refreshed before the next read-heavy MCP
//! session.

use atlas_core::{AtlasError, Result};

use super::Store;

impl Store {
    /// Refresh planner statistics for the graph database.
    ///
    /// Runs `ANALYZE` so `sqlite_stat1` exists, then `PRAGMA optimize` so any
    /// remaining per-query planner hints are recorded. Both are cheap relative
    /// to a build/update/postprocess pass and are safe to repeat.
    pub fn optimize_for_reads(&self) -> Result<()> {
        self.conn
            .execute_batch("ANALYZE; PRAGMA optimize;")
            .map_err(|e| AtlasError::Db(e.to_string()))
    }
}
