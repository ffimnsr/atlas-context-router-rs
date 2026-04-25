//! Durable artifact content store for Atlas context memory integration.
//!
//! Stores large command outputs, tool results, and context payloads in
//! `.atlas/context.db`, separate from the graph database and session database.
//! Each `ContentStore` instance owns one thread-confined SQLite connection;
//! concurrent access uses separate store instances and separate connections.

pub mod chunking;
mod migrations;
pub mod store;

pub use store::{
    ChunkResult, ContentStore, ContentStoreConfig, IndexRunStats, IndexState, IndexingStats,
    OutputRouting, OversizedPolicy, RetrievalIndexStatus, RoutingStats, SearchFilters, SourceMeta,
    SourceRow,
};
