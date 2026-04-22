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

use crate::migrations::MIGRATIONS;
use util::{is_corruption_error, quarantine_db};

pub use types::{
    ChunkResult, ContentStoreConfig, IndexState, IndexingStats, OutputRouting,
    RetrievalIndexStatus, RoutingStats, SearchFilters, SourceMeta, SourceRow,
};

/// SQLite-backed content store.
pub struct ContentStore {
    pub(super) conn: Connection,
    pub(super) config: ContentStoreConfig,
    pub(super) routing_stats: RoutingStats,
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
        };
        store.apply_pragmas()?;
        Ok(store)
    }

    fn apply_pragmas(&self) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        for sql in &[
            "PRAGMA journal_mode=WAL",
            "PRAGMA synchronous=NORMAL",
            "PRAGMA foreign_keys=ON",
            "PRAGMA busy_timeout=5000",
        ] {
            let mut stmt = self.conn.prepare(sql).map_err(db_err)?;
            let mut rows = stmt.query([]).map_err(db_err)?;
            while rows.next().map_err(db_err)?.is_some() {}
        }
        Ok(())
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
        Ok(())
    }
}
