//! SQLite-backed content store for Atlas artifact persistence.
//!
//! Stores large command outputs, tool results, and context payloads so they
//! can be retrieved by session or keyword search without growing the prompt
//! context window.
//!
//! Uses `.atlas/context.db`, kept strictly separate from the graph database.

mod artifact;
mod lifecycle;
mod markdown;
pub mod registry;
mod search;
#[cfg(test)]
mod tests;
mod types;
mod util;

use std::collections::HashMap;

use rusqlite::{Connection, OpenFlags};
use tracing::info;

use atlas_core::{AtlasError, Result};
use atlas_db_utils::{
    application_id, apply_atlas_pragmas, migrate_database_to, set_application_id,
};

use crate::migrations::{LATEST_VERSION, MIGRATION_SET};
use util::{is_corruption_error, quarantine_db};

pub use types::{
    ChunkResult, ContentStoreConfig, DimensionMismatchError, EmbeddingProviderEntry, IndexRunStats,
    IndexState, IndexingStats, OutputRouting, OversizedPolicy, RetrievalIndexStatus, RoutingStats,
    SearchFilters, SourceMeta, SourceRow,
};

/// SQLite-backed content store.
///
/// Owns exactly one thread-confined SQLite connection for `context.db`.
/// Concurrent access, when needed, must use separate connections rather than
/// sharing this one across threads.
///
/// The `_thread_bound` field holds `PhantomData<*const ()>` to explicitly
/// opt out of `Send` and `Sync` auto-traits at the compiler level.
pub struct ContentStore {
    pub(super) conn: Connection,
    pub(super) config: ContentStoreConfig,
    pub(super) routing_stats: RoutingStats,
    pub(super) run_stats: IndexRunStats,
    /// Markdown truth store rooted at the `.atlas/` directory.
    pub(super) markdown: atlas_store_markdown::MarkdownStore,
    /// In-process cache of frozen embedding dimensions, keyed by
    /// `(provider_name, model_name)`.  Populated lazily or by `warm_dimension_cache()`.
    pub(super) dim_cache: HashMap<(String, String), u32>,
    /// Marker that opts this struct out of `Send` and `Sync`.
    _thread_bound: std::marker::PhantomData<*const ()>,
}

impl ContentStore {
    /// Open (or create) the content store database at `path` with default config.
    pub fn open(path: &str) -> Result<Self> {
        Self::open_with_config(path, ContentStoreConfig::default())
    }

    /// Open (or create) the content store database at `path` with custom config.
    pub fn open_with_config(path: &str, config: ContentStoreConfig) -> Result<Self> {
        match Self::try_open(path, config.clone()) {
            Ok(store) => Ok(store),
            Err(e) => {
                if is_corruption_error(&e) {
                    quarantine_db(path);
                }
                Err(e)
            }
        }
    }

    fn try_open(path: &str, config: ContentStoreConfig) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        let atlas_dir = std::path::Path::new(path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::Path::new(".").join(".atlas"));
        let markdown = atlas_store_markdown::MarkdownStore::open(&atlas_dir)
            .map_err(|error| AtlasError::Other(format!("markdown store: {error}")))?;
        let mut store = Self {
            conn,
            config,
            routing_stats: RoutingStats::default(),
            run_stats: IndexRunStats::default(),
            markdown,
            dim_cache: HashMap::new(),
            _thread_bound: std::marker::PhantomData,
        };
        apply_atlas_pragmas(&store.conn)?;
        set_application_id(&store.conn, application_id::CONTEXT)?;
        store.migrate()?;
        Ok(store)
    }

    /// Rebuild derived chunk/FTS/vocabulary indexes from `.atlas/artifacts/**`.
    /// `full` clears existing derived rows first; otherwise indexing is
    /// incremental and unchanged chunks are reused.
    pub fn rebuild_from_markdown(&mut self, full: bool) -> Result<usize> {
        if full {
            self.conn
                .execute_batch(
                    "DELETE FROM chunks_fts;
                     DELETE FROM chunks_trigram;
                     DELETE FROM chunks;
                     DELETE FROM sources;
                     DELETE FROM vocabulary;",
                )
                .map_err(|error| AtlasError::Db(error.to_string()))?;
        }
        let artifacts = markdown::read_all(&self.markdown)?;
        let present = artifacts
            .iter()
            .map(|artifact| artifact.source_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        // Markdown truth: drop derived sources whose file was deleted outside
        // the delete path before indexing the remaining files.
        let indexed_ids = {
            let mut stmt = self
                .conn
                .prepare("SELECT id FROM sources")
                .map_err(|error| AtlasError::Db(error.to_string()))?;
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| AtlasError::Db(error.to_string()))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| AtlasError::Db(error.to_string()))?
        };
        for source_id in indexed_ids {
            if !present.contains(source_id.as_str()) {
                self.delete_source(&source_id)?;
            }
        }
        let mut indexed = 0usize;
        for artifact in artifacts {
            self.index_artifact(artifact.to_meta(), &artifact.body, &artifact.content_type)?;
            indexed += 1;
        }
        Ok(indexed)
    }

    /// Apply any pending schema migrations.
    pub fn migrate(&mut self) -> Result<()> {
        self.migrate_to(LATEST_VERSION)
    }

    pub fn migrate_to(&mut self, target_version: i32) -> Result<()> {
        let current: i32 = self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if target_version >= current {
            for migration in MIGRATION_SET.migrations.iter().filter(|migration| {
                migration.version > current && migration.version <= target_version
            }) {
                info!(
                    version = migration.version,
                    name = migration.name,
                    "applying content store migration"
                );
            }
        } else {
            info!(
                current,
                target_version, "rebuilding content store schema for downgrade"
            );
        }
        migrate_database_to(&mut self.conn, &MIGRATION_SET, target_version)
    }

    pub fn schema_version(&self) -> Result<i32> {
        self.conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| AtlasError::Db(e.to_string()))
    }
}
