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
///
/// The `_thread_bound` field holds `PhantomData<*const ()>` to explicitly
/// opt out of `Send` and `Sync` auto-traits. This enforces thread confinement
/// at the compiler level even though `rusqlite::Connection` gained `Send` in
/// 0.32 (single-threaded serialized mode). Atlas stores must stay on the
/// thread that opened them; Rayon closures receive only parse inputs.
pub struct Store {
    conn: Connection,
    /// Marker that opts this struct out of `Send` and `Sync`.
    /// Atlas stores are thread-confined; see struct-level doc.
    _thread_bound: std::marker::PhantomData<*const ()>,
}

#[cfg(test)]
mod tests;
