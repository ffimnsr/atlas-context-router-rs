//! SQLite-backed session metadata, event ledger, and resume snapshots.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tracing::info;

use atlas_core::{AtlasError, Result};

use crate::SessionId;
use crate::migrations::MIGRATIONS;

mod resume;
#[cfg(test)]
mod tests;
mod types;
mod util;

pub use self::types::{
    DEFAULT_DEDUP_WINDOW_SECS, DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_SESSION_DB,
    DEFAULT_SESSION_MAX_EVENTS, EventCategory, MAX_INLINE_EVENT_PAYLOAD_BYTES, NewSessionEvent,
    ResumeSnapshot, SessionEventRow, SessionEventType, SessionMeta, SessionStats,
    SessionStoreConfig,
};

use self::resume::build_resume_snapshot;
use self::util::{
    canonical_json, enforce_event_limit, format_days_ago, format_now, format_seconds_ago,
    is_corruption_error, normalize_event_payload_paths, path_to_str, quarantine_db, row_to_event,
};

pub struct SessionStore {
    pub(super) conn: Connection,
    pub(super) config: SessionStoreConfig,
}

impl SessionStore {
    pub fn open(path: &str) -> Result<Self> {
        Self::open_with_config(path, SessionStoreConfig::default())
    }

    pub fn open_in_repo(repo_root: impl AsRef<Path>) -> Result<Self> {
        let path = repo_root.as_ref().join(".atlas").join(DEFAULT_SESSION_DB);
        Self::open(path_to_str(&path)?)
    }

    pub fn open_with_config(path: &str, config: SessionStoreConfig) -> Result<Self> {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }

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

    fn try_open(path: &str, config: SessionStoreConfig) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut store = Self { conn, config };
        store.apply_pragmas()?;
        store.migrate()?;
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
                |row| row.get(0),
            )
            .unwrap_or(0);

        for migration in MIGRATIONS {
            if migration.version <= current {
                continue;
            }
            info!(version = migration.version, "applying session migration");
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

    pub fn upsert_session_meta(
        &mut self,
        session_id: SessionId,
        repo_root: &str,
        frontend: &str,
        worktree_id: Option<&str>,
    ) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO session_meta (
                    session_id, repo_root, frontend, worktree_id, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(session_id) DO UPDATE SET
                    repo_root = excluded.repo_root,
                    frontend = excluded.frontend,
                    worktree_id = excluded.worktree_id,
                    updated_at = excluded.updated_at",
                params![
                    session_id.as_str(),
                    repo_root,
                    frontend,
                    worktree_id,
                    now,
                    now
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn get_session_meta(&self, session_id: &SessionId) -> Result<Option<SessionMeta>> {
        self.conn
            .query_row(
                "SELECT session_id, repo_root, frontend, worktree_id, created_at, updated_at,
                        last_resume_at, last_compaction_at
                 FROM session_meta
                 WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| {
                    Ok(SessionMeta {
                        session_id: SessionId(row.get(0)?),
                        repo_root: row.get(1)?,
                        frontend: row.get(2)?,
                        worktree_id: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        last_resume_at: row.get(6)?,
                        last_compaction_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|e| AtlasError::Db(e.to_string()))
    }

    pub fn append_event(&mut self, event: NewSessionEvent) -> Result<Option<SessionEventRow>> {
        let mut payload = event.payload;
        if let Some(meta) = self.get_session_meta(&event.session_id)? {
            normalize_event_payload_paths(&meta.repo_root, &mut payload);
        }
        let payload_json = canonical_json(&payload);
        self.ensure_payload_fits(&payload_json)?;
        let created_at = event.created_at.unwrap_or_else(format_now);
        let event_hash = util::hash_event(&event.event_type, event.priority, &payload_json);

        if self.config.dedup_window_secs > 0 {
            let window_cutoff = format_seconds_ago(self.config.dedup_window_secs);
            let recent: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM session_events
                     WHERE session_id = ?1
                       AND event_type = ?2
                       AND priority = ?3
                       AND created_at >= ?4",
                    params![
                        event.session_id.as_str(),
                        event.event_type.as_str(),
                        event.priority,
                        window_cutoff,
                    ],
                    |row| row.get(0),
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            if recent > 0 {
                return Ok(None);
            }
        }

        let tx = self
            .conn
            .transaction()
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO session_events (
                    session_id, event_type, priority, payload_json, event_hash, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    event.session_id.as_str(),
                    event.event_type.as_str(),
                    event.priority,
                    payload_json,
                    event_hash,
                    created_at,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        tx.execute(
            "UPDATE session_meta SET updated_at = ?2 WHERE session_id = ?1",
            params![event.session_id.as_str(), format_now()],
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut evicted = 0usize;
        if inserted > 0 {
            evicted = enforce_event_limit(
                &tx,
                event.session_id.as_str(),
                self.config.max_events_per_session,
            )?;
        }

        if evicted > 0 {
            let compacted_at = format_now();
            tx.execute(
                "UPDATE session_meta
                 SET updated_at = ?2, last_compaction_at = ?2
                 WHERE session_id = ?1",
                params![event.session_id.as_str(), compacted_at],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        }

        let row = tx
            .query_row(
                "SELECT id, session_id, event_type, priority, payload_json, event_hash, created_at
                 FROM session_events
                 WHERE session_id = ?1 AND event_hash = ?2",
                params![event.session_id.as_str(), event_hash],
                row_to_event,
            )
            .optional()
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;

        if inserted == 0 {
            return Ok(None);
        }

        Ok(row)
    }

    pub fn list_events(&self, session_id: &SessionId) -> Result<Vec<SessionEventRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, event_type, priority, payload_json, event_hash, created_at
                 FROM session_events
                 WHERE session_id = ?1
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let rows = stmt
            .query_map(params![session_id.as_str()], row_to_event)
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn put_resume_snapshot(
        &mut self,
        session_id: &SessionId,
        snapshot: &str,
        event_count: i64,
        consumed: bool,
    ) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO session_resume (
                    session_id, snapshot, event_count, consumed, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(session_id) DO UPDATE SET
                    snapshot = excluded.snapshot,
                    event_count = excluded.event_count,
                    consumed = excluded.consumed,
                    updated_at = excluded.updated_at",
                params![
                    session_id.as_str(),
                    snapshot,
                    event_count,
                    i32::from(consumed),
                    now,
                    now,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.conn
            .execute(
                "UPDATE session_meta SET updated_at = ?2 WHERE session_id = ?1",
                params![session_id.as_str(), now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn get_resume_snapshot(&self, session_id: &SessionId) -> Result<Option<ResumeSnapshot>> {
        self.conn
            .query_row(
                "SELECT session_id, snapshot, event_count, consumed, created_at, updated_at
                 FROM session_resume
                 WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| {
                    Ok(ResumeSnapshot {
                        session_id: SessionId(row.get(0)?),
                        snapshot: row.get(1)?,
                        event_count: row.get(2)?,
                        consumed: row.get::<_, i32>(3)? != 0,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|e| AtlasError::Db(e.to_string()))
    }

    pub fn mark_resume_consumed(&mut self, session_id: &SessionId, consumed: bool) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "UPDATE session_resume
                 SET consumed = ?2, updated_at = ?3
                 WHERE session_id = ?1",
                params![session_id.as_str(), i32::from(consumed), now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.conn
            .execute(
                "UPDATE session_meta
                 SET updated_at = ?2, last_resume_at = ?2
                 WHERE session_id = ?1",
                params![session_id.as_str(), now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT session_id, repo_root, frontend, worktree_id, created_at, updated_at,
                        last_resume_at, last_compaction_at
                 FROM session_meta
                 ORDER BY updated_at DESC, session_id ASC",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionMeta {
                    session_id: SessionId(row.get(0)?),
                    repo_root: row.get(1)?,
                    frontend: row.get(2)?,
                    worktree_id: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    last_resume_at: row.get(6)?,
                    last_compaction_at: row.get(7)?,
                })
            })
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn delete_session(&mut self, session_id: &SessionId) -> Result<bool> {
        let rows = self
            .conn
            .execute(
                "DELETE FROM session_meta WHERE session_id = ?1",
                params![session_id.as_str()],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(rows > 0)
    }

    pub fn stats(&self) -> Result<SessionStats> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let session_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM session_meta", [], |r| r.get(0))
            .map_err(db_err)?;
        let total_events: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM session_events", [], |r| r.get(0))
            .map_err(db_err)?;
        let snapshot_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM session_resume", [], |r| r.get(0))
            .map_err(db_err)?;
        Ok(SessionStats {
            session_count: session_count as usize,
            total_events: total_events as usize,
            snapshot_count: snapshot_count as usize,
        })
    }

    pub fn cleanup_stale_sessions(&mut self, keep_days: u32) -> Result<usize> {
        let cutoff = format_days_ago(keep_days);
        let count = self
            .conn
            .execute(
                "DELETE FROM session_meta WHERE updated_at < ?1",
                params![cutoff],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(count)
    }

    pub fn build_resume(&mut self, session_id: &SessionId) -> Result<ResumeSnapshot> {
        build_resume_snapshot(self, session_id)
    }

    fn ensure_payload_fits(&self, payload_json: &str) -> Result<()> {
        if payload_json.len() > self.config.max_inline_payload_bytes {
            return Err(AtlasError::Other(format!(
                "session event payload {} bytes exceeds inline limit {} bytes; store raw output in content store and write source_id into payload",
                payload_json.len(),
                self.config.max_inline_payload_bytes
            )));
        }
        Ok(())
    }
}
