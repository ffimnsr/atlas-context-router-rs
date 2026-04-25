use rusqlite::Connection;

mod build_state;
mod context;
mod graph;
mod helpers;
mod history;
mod lifecycle;
mod mutation;
mod postprocess;
mod retrieval;
mod search;
mod taxonomy;

pub use self::build_state::{BuildFinishStats, GraphBuildState, GraphBuildStatus};
pub use self::history::{
    HistoricalEdge, HistoricalNode, HistoryStatusSummary, StoredCommit, StoredEdgeHistory,
    StoredNodeHistory, StoredSnapshot, StoredSnapshotFile, StoredSnapshotMembershipBlob,
};

type DanglingEdge = (i64, String, String, String, &'static str);

/// SQLite-backed graph store.
///
/// Owns exactly one thread-confined SQLite connection for `worldtree.db`.
/// All graph mutation goes through this struct. Concurrent reads, if added
/// later, must use separate connections rather than shared ownership of this
/// one. No read pool exists today.
pub struct Store {
    conn: Connection,
}

#[cfg(test)]
mod tests;
