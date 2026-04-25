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

use rusqlite::{Connection, OpenFlags, params};
use tracing::info;

use atlas_core::{AtlasError, Result};
use atlas_db_utils::{application_id, apply_atlas_pragmas, set_application_id, set_user_version};

use crate::migrations::MIGRATIONS;
use util::{is_corruption_error, quarantine_db};

pub use types::{
    ChunkResult, ContentStoreConfig, IndexRunStats, IndexState, IndexingStats, OutputRouting,
    OversizedPolicy, RetrievalIndexStatus, RoutingStats, SearchFilters, SourceMeta, SourceRow,
};

/// SQLite-backed content store.
pub struct ContentStore {
    pub(super) conn: Connection,
    pub(super) config: ContentStoreConfig,
    pub(super) routing_stats: RoutingStats,
    pub(super) run_stats: IndexRunStats,
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

        let store = Self {
            conn,
            config,
            routing_stats: RoutingStats::default(),
            run_stats: IndexRunStats::default(),
        };
        apply_atlas_pragmas(&store.conn)?;
        set_application_id(&store.conn, application_id::CONTEXT)?;
        Ok(store)
    }

    /// Apply any pending schema migrations.
    pub fn migrate(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS metadata (
                     key   TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let current: i32 = self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        for m in MIGRATIONS {
            if m.version > current {
                info!("applying content store migration v{}", m.version);
                self.conn
                    .execute_batch(m.sql)
                    .map_err(|e| AtlasError::Db(format!("migration {}: {e}", m.version)))?;
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?1)",
                        params![m.version.to_string()],
                    )
                    .map_err(|e| AtlasError::Db(e.to_string()))?;
            }
        }
        let applied: i32 = self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        set_user_version(&self.conn, applied)?;
        Ok(())
    }
}
