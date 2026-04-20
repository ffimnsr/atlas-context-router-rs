//! Context save/restore coordinator for Atlas context-mode integration.
//!
//! Composes `atlas-contentstore` and `atlas-session` to provide higher-level
//! save and restore operations.  This crate must not depend on the graph
//! database.

pub use atlas_contentstore::{ContentStore, OutputRouting, SearchFilters, SourceMeta};
pub use atlas_session::SessionId;
