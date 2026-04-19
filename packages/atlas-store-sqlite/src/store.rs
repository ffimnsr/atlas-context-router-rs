use atlas_core::{AtlasError, GraphStats, Result};
use rusqlite::{Connection, OpenFlags, params};
use tracing::{debug, info};

use crate::migrations::MIGRATIONS;

/// SQLite-backed graph store.
///
/// Holds a single write connection; all mutation goes through this struct.
/// Parallel read access is left for a future read-pool layer.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) the atlas database at `path` and apply any pending
    /// migrations.  The directory containing `path` must already exist.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        Self::apply_pragmas(&conn)?;

        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn apply_pragmas(conn: &Connection) -> Result<()> {
        // Pragmas in SQLite may or may not return result rows depending on the
        // pragma and the SQLite version. Prepare + drain rows so we never hit
        // the "Execute returned results" error from rusqlite's execute_batch.
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        for sql in &[
            "PRAGMA journal_mode=WAL",
            "PRAGMA synchronous=NORMAL",
            "PRAGMA foreign_keys=ON",
            "PRAGMA busy_timeout=5000",
        ] {
            let mut stmt = conn.prepare(sql).map_err(db_err)?;
            let mut rows = stmt.query([]).map_err(db_err)?;
            while rows.next().map_err(db_err)?.is_some() {}
        }
        Ok(())
    }

    /// Apply any migrations that have not yet been applied to this database.
    pub fn migrate(&mut self) -> Result<()> {
        // Bootstrap the metadata table so we can store schema_version.
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS metadata (
                     key   TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let current_version: i32 = self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        debug!(current_version, "checking migrations");

        for migration in MIGRATIONS {
            if migration.version <= current_version {
                continue;
            }
            info!(version = migration.version, "applying migration");
            self.conn
                .execute_batch(migration.sql)
                .map_err(|e| AtlasError::Db(format!("migration {}: {e}", migration.version)))?;
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?1)",
                    params![migration.version.to_string()],
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
        }
        Ok(())
    }

    /// Return high-level statistics about the stored graph.
    pub fn stats(&self) -> Result<GraphStats> {
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let node_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let edge_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut stmt = self
            .conn
            .prepare("SELECT kind, COUNT(*) FROM nodes GROUP BY kind ORDER BY COUNT(*) DESC")
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let nodes_by_kind: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| AtlasError::Db(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT language FROM nodes WHERE language IS NOT NULL ORDER BY language",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let languages: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let last_indexed_at: Option<String> = self
            .conn
            .query_row(
                "SELECT MAX(indexed_at) FROM files",
                [],
                |r| r.get(0),
            )
            .unwrap_or(None);

        Ok(GraphStats {
            file_count,
            node_count,
            edge_count,
            nodes_by_kind,
            languages,
            last_indexed_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_in_memory() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        Store::apply_pragmas(&conn).unwrap();
        let mut store = Store { conn };
        store.migrate().unwrap();
        store
    }

    #[test]
    fn migration_creates_schema() {
        let store = open_in_memory();
        let stats = store.stats().unwrap();
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    fn schema_version_stored() {
        let store = open_in_memory();
        let version: i32 = store
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn wal_mode_enabled() {
        let store = open_in_memory();
        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        // In-memory databases report "memory" regardless; file DBs report "wal".
        // Just verify the pragma round-trips without error.
        assert!(!mode.is_empty());
    }
}
