//! SQLite graph store and migration crate for Atlas.
//!
//! Owns schema creation, graph persistence, history storage, and graph-backed
//! query helpers over `.atlas/worldtree.db`.
//!
//! Exposed surfaces include:
//! - [`Store`] for graph, history, analytics, and maintenance operations
//! - build and postprocess status types for CLI and MCP diagnostics
//! - historical snapshot and lifecycle record types

mod migrations;
pub mod store;

pub use store::{
    BuildFinishStats, GraphBuildState, GraphBuildStatus, HistoricalEdge, HistoricalNode,
    HistoryStatusSummary, Store, StoredCommit, StoredEdgeHistory, StoredNodeHistory,
    StoredSnapshot, StoredSnapshotFile, StoredSnapshotMembershipBlob,
};
