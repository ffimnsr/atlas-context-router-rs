#![doc = include_str!("../README.md")]

/// Graph schema migration registry and version constants.
///
/// Public so fixture tooling (e.g. `examples/dump_schema.rs`) can regenerate
/// checked-in schema snapshots without duplicating the migration list.
pub mod migrations;
pub mod store;

pub use store::{
    BuildFinishStats, ConfidenceTierEdge, DEFAULT_TRIEHI_MIN_CHUNKS, DirectoryTrie,
    GraphBuildState, GraphBuildStatus, GraphRecoveryMode, GraphStoreRecovery,
    GraphStoreRecoveryError, HistoricalEdge, HistoricalNode, HistoryStatusSummary,
    MAX_NODE_LIST_LIMIT, NodeListFilter, NodeListPage, SemanticReferenceEdge, Store, StoredCommit,
    StoredEdgeHistory, StoredNodeHistory, StoredSnapshot, StoredSnapshotFile,
    StoredSnapshotMembershipBlob, TrieNode, VectorScope,
};
