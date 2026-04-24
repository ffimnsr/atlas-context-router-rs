//! Atlas historical graph metadata ingestion and querying.
//!
//! Slice 1 — metadata foundation:
//! - deterministic git metadata wrappers
//! - commit selection strategies
//! - commit metadata ingestion
//! - `atlas history status` summary

pub mod git;
pub mod ingest;
pub mod select;
pub mod status;

pub use ingest::IngestError;
pub use select::CommitSelector;
pub use status::HistoryStatus;
