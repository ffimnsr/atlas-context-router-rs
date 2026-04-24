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

pub mod build;
pub mod git;
pub mod ingest;
pub mod select;
pub mod status;

pub use build::{BuildSummary, build_historical_graph};
pub use ingest::IngestError;
pub use select::CommitSelector;
pub use status::HistoryStatus;
