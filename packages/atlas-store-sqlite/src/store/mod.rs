use rusqlite::Connection;

mod build_state;
mod context;
mod graph;
mod helpers;
mod history;
mod lifecycle;
mod mutation;
mod retrieval;
mod search;
mod taxonomy;

pub use self::build_state::{BuildFinishStats, GraphBuildState, GraphBuildStatus};
pub use self::history::{HistoryStatusSummary, StoredCommit, StoredSnapshot, StoredSnapshotFile};

type DanglingEdge = (i64, String, String, String, &'static str);

/// SQLite-backed graph store.
///
/// Holds a single write connection; all mutation goes through this struct.
/// Parallel read access is left for a future read-pool layer.
pub struct Store {
    conn: Connection,
}

#[cfg(test)]
mod tests;
