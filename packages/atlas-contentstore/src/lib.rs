#![doc = include_str!("../README.md")]

pub mod chunking;
mod migrations;
pub mod store;

/// Highest content-store schema version this binary can open.
///
/// A `context.db` stamped with a higher `PRAGMA user_version` fails closed on
/// open with a schema-mismatch error and is never quarantined.
pub const CONTENT_SCHEMA_VERSION: i32 = migrations::LATEST_VERSION;

pub use store::{
    ChunkResult, ContentStore, ContentStoreConfig, DimensionMismatchError, EmbeddingProviderEntry,
    IndexRunStats, IndexState, IndexingStats, OutputRouting, OversizedPolicy, RetrievalIndexStatus,
    RoutingStats, SearchFilters, SourceMeta, SourceRow,
};
