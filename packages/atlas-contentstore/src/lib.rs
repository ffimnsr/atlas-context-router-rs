//! Durable artifact content store for Atlas context-mode integration.
//!
//! Stores large command outputs, tool results, and context payloads in
//! `.atlas/context.db`, separate from the graph database and session database.

pub mod chunking;
mod migrations;
pub mod store;

pub use store::{ChunkResult, ContentStore, OutputRouting, SearchFilters, SourceMeta, SourceRow};
