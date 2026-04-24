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
mod migrations;
pub mod store;

pub use identity::SessionId;
pub use store::{
    CurationResult, DEFAULT_DEDUP_WINDOW_SECS, DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_SESSION_DB,
    DEFAULT_SESSION_MAX_EVENTS, DecisionRecord, DecisionSearchHit, EventCategory,
    GlobalAccessEntry, GlobalWorkflowPattern, MAX_INLINE_EVENT_PAYLOAD_BYTES, NewSessionEvent,
    ResumeSnapshot, SessionEventRow, SessionEventType, SessionMeta, SessionStats, SessionStore,
    SessionStoreConfig,
};
