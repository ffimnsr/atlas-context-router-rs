#![doc = include_str!("../README.md")]

/// Graph schema migration registry and version constants.
///
/// Public so fixture tooling (e.g. `examples/dump_schema.rs`) can regenerate
/// checked-in schema snapshots without duplicating the migration list.
pub mod migrations;
pub mod store;
mod vec_index;

/// Highest graph schema version this binary can open.
///
/// Databases stamped with a higher `PRAGMA user_version` are rejected during
/// open with a schema-mismatch error rather than being quarantined or migrated
/// downwards; callers surface the version pair so operators can upgrade Atlas.
pub const GRAPH_SCHEMA_VERSION: i32 = migrations::LATEST_VERSION;

pub use store::{
    BuildFinishStats, ConfidenceTierEdge, DEFAULT_TRIEHI_MIN_CHUNKS, DanglingEdge, DirectoryTrie,
    GraphBuildState, GraphBuildStatus, GraphRecoveryMode, GraphStoreRecovery,
    GraphStoreRecoveryError, HistoricalEdge, HistoricalNode, HistoryStatusSummary,
    MAX_NODE_LIST_LIMIT, NodeListFilter, NodeListPage, SemanticReferenceEdge, Store, StoredCommit,
    StoredEdgeHistory, StoredNodeHistory, StoredSnapshot, StoredSnapshotFile,
    StoredSnapshotMembershipBlob, TraceDirection, TraceNode, TraceOptions, TracePath,
    TracePathResult, TraceResult, TrieNode, VectorScope,
};
