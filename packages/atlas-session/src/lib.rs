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
    DEFAULT_SESSION_DB, DEFAULT_SESSION_MAX_EVENTS, MAX_INLINE_EVENT_PAYLOAD_BYTES,
    NewSessionEvent, ResumeSnapshot, SessionEventRow, SessionEventType, SessionMeta, SessionStore,
    SessionStoreConfig,
};
