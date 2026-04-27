//! Atlas historical graph metadata ingestion and querying.
//!
//! Slice 1 — metadata foundation:
//! - deterministic git metadata wrappers
//! - commit selection strategies
//! - commit metadata ingestion
//! - `atlas history status` summary
//!
//! Slice 2 — file graph reuse and historical build:
//! - content-addressed node/edge storage keyed by git blob SHA
//! - checkout-free file reconstruction via `git show`
//! - snapshot node/edge membership tables
//! - `atlas history build` command logic
//!
//! Slice 3 — incremental update, lifecycle, reconstruction, and diff:
//! - detect missing commits on current branch ancestry
//! - detect rewritten history / force-push divergence unless repair is explicit
//! - compute node and edge lifecycle rows from snapshot membership
//! - reconstruct historical graph state for any indexed commit
//! - diff any two indexed commits with file, node, edge, module, and architecture scopes
//! - treat qualified-name/path/kind identity breaks as remove+add; rename continuity is not inferred
//!
//! Slice 4 — history queries and output contracts:
//! - evidence-rich symbol, file, dependency, and module history reports
//! - summary + findings + evidence fields across history outputs
//!
//! Slice 5 — analytics, retention, and diagnostics:
//! - churn/stability/trend reports over indexed history
//! - storage diagnostics and deduplication metrics
//! - retention policies and `atlas history prune`

pub mod analytics;
pub mod build;
pub mod diff;
pub mod error;
pub mod git;
pub mod ingest;
pub mod lifecycle;
pub mod prune;
pub mod query;
pub mod reports;
pub(crate) mod scc;
pub mod select;
pub mod status;
#[cfg(test)]
pub(crate) mod test_support;
pub mod update;

pub use analytics::{
    ArchitecturalHotspotRecord, ChurnReport, ChurnSummary, DependencyChurnRecord, FileChurnRecord,
    ModuleChurnRecord, StorageDiagnostics, TrendMetrics, compute_churn_report,
};
pub use build::{
    BuildFileProgressKind, BuildProgressEvent, BuildSummary, SnapshotRebuildSummary,
    build_historical_graph, build_historical_graph_with_progress, rebuild_historical_snapshot,
    rebuild_historical_snapshot_with_progress,
};
pub use diff::{GraphDiffReport, HistoricalSnapshot, diff_snapshots, reconstruct_snapshot};
pub use error::{HistoryError, Result};
pub use ingest::IngestError;
pub use lifecycle::{LifecycleSummary, recompute_lifecycle};
pub use prune::{HistoryPruneSummary, HistoryRetentionPolicy, prune_historical_graph};
pub use query::{
    EdgeHistoryReport, FileHistoryReport, ModuleHistoryReport, NodeHistoryReport,
    query_dependency_history, query_file_history, query_file_history_with_options,
    query_module_history, query_symbol_history,
};
pub use reports::HistoryEvidence;
pub use select::CommitSelector;
pub use status::HistoryStatus;
pub use update::{
    HistoryUpdateSummary, update_historical_graph, update_historical_graph_with_progress,
};
