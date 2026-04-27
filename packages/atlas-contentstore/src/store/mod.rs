//! SQLite-backed content store for Atlas artifact persistence.
//!
//! Stores large command outputs, tool results, and context payloads so they
//! can be retrieved by session or keyword search without growing the prompt
//! context window.
//!
//! Uses `.atlas/context.db`, kept strictly separate from the graph database.

mod artifact;
mod lifecycle;
mod search;
#[cfg(test)]
mod tests;
mod types;
mod util;

use rusqlite::{Connection, OpenFlags};
use tracing::info;

use atlas_core::{AtlasError, Result};
use atlas_db_utils::{
    application_id, apply_atlas_pragmas, migrate_database_to, set_application_id,
};

use crate::migrations::{LATEST_VERSION, MIGRATION_SET};
use util::{is_corruption_error, quarantine_db};

pub use types::{
    ChunkResult, ContentStoreConfig, IndexRunStats, IndexState, IndexingStats, OutputRouting,
    OversizedPolicy, RetrievalIndexStatus, RoutingStats, SearchFilters, SourceMeta, SourceRow,
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

        let mut store = Self {
            conn,
            config,
            routing_stats: RoutingStats::default(),
            run_stats: IndexRunStats::default(),
            _thread_bound: std::marker::PhantomData,
        };
        apply_atlas_pragmas(&store.conn)?;
        set_application_id(&store.conn, application_id::CONTEXT)?;
        store.migrate()?;
        Ok(store)
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
}
