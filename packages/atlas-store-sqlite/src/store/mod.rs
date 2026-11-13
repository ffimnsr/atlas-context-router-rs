use rusqlite::Connection;

mod build_state;
mod context;
mod directory_trie;
mod file_scan;
mod graph;
mod helpers;
mod history;
mod lifecycle;
mod mutation;
mod postprocess;
mod recovery;
mod retrieval;
mod search;
mod taxonomy;
mod trace;

pub use self::build_state::{BuildFinishStats, GraphBuildState, GraphBuildStatus};
pub use self::context::{ConfidenceTierEdge, SemanticReferenceEdge};
pub use self::directory_trie::{DirectoryTrie, TrieNode};
pub use self::graph::{MAX_NODE_LIST_LIMIT, NodeListFilter, NodeListPage};
pub use self::history::{
    HistoricalEdge, HistoricalNode, HistoryStatusSummary, StoredCommit, StoredEdgeHistory,
    StoredNodeHistory, StoredSnapshot, StoredSnapshotFile, StoredSnapshotMembershipBlob,
};
pub use self::recovery::{GraphRecoveryMode, GraphStoreRecovery, GraphStoreRecoveryError};
pub use self::retrieval::{DEFAULT_TRIEHI_MIN_CHUNKS, VectorScope};
pub use self::trace::{
    TraceDirection, TraceNode, TraceOptions, TracePath, TracePathResult, TraceResult,
};

pub type DanglingEdge = (i64, String, String, String, &'static str);

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
    /// Lazily built TrieHI index over retrieval-chunk ids (VR2 stage 2).
    ///
    /// Thread-confined with the connection; dropped on any chunk mutation and
    /// rebuilt on the next scoped vector query when the configured threshold
    /// is met.
    trie_cache: std::cell::RefCell<Option<DirectoryTrie>>,
    /// Marker that opts this struct out of `Send` and `Sync`.
    /// Atlas stores are thread-confined; see struct-level doc.
    _thread_bound: std::marker::PhantomData<*const ()>,
}

#[cfg(test)]
mod tests;
