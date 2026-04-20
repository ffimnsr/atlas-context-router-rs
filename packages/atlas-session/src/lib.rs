//! Atlas session identity, event ledger, and resume snapshot storage.
//!
//! Responsibilities:
//! - derive stable session ids from repo + worktree + frontend anchors
//! - persist session metadata across runs
//! - record bounded event history per session
//! - produce and consume resume snapshots
//!
//! This crate must not depend on the graph database (`atlas-store-sqlite`)
//! or on content-storage concerns (`atlas-contentstore`).

pub mod identity;

pub use identity::SessionId;
