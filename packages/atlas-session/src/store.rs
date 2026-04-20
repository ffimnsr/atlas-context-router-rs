//! SQLite-backed session metadata, event ledger, and resume snapshots.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tracing::info;

use atlas_core::{AtlasError, Result};

use crate::SessionId;
use crate::migrations::MIGRATIONS;

pub const DEFAULT_SESSION_DB: &str = "session.db";
pub const DEFAULT_SESSION_MAX_EVENTS: usize = 256;
pub const MAX_INLINE_EVENT_PAYLOAD_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct SessionStoreConfig {
    pub max_events_per_session: usize,
    pub max_inline_payload_bytes: usize,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            max_events_per_session: DEFAULT_SESSION_MAX_EVENTS,
            max_inline_payload_bytes: MAX_INLINE_EVENT_PAYLOAD_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub session_id: SessionId,
    pub repo_root: String,
    pub frontend: String,
    pub worktree_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_resume_at: Option<String>,
    pub last_compaction_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionEventType {
    FileRead,
    FileWrite,
    CommandRun,
    CommandFail,
    GraphBuild,
    GraphUpdate,
    ReviewContext,
    ImpactAnalysis,
    ContextRequest,
    ReasoningResult,
    UserIntent,
    Error,
    SessionStart,
    SessionResume,
}

impl SessionEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FileRead => "FILE_READ",
            Self::FileWrite => "FILE_WRITE",
            Self::CommandRun => "COMMAND_RUN",
            Self::CommandFail => "COMMAND_FAIL",
            Self::GraphBuild => "GRAPH_BUILD",
            Self::GraphUpdate => "GRAPH_UPDATE",
            Self::ReviewContext => "REVIEW_CONTEXT",
            Self::ImpactAnalysis => "IMPACT_ANALYSIS",
            Self::ContextRequest => "CONTEXT_REQUEST",
            Self::ReasoningResult => "REASONING_RESULT",
            Self::UserIntent => "USER_INTENT",
            Self::Error => "ERROR",
            Self::SessionStart => "SESSION_START",
            Self::SessionResume => "SESSION_RESUME",
        }
    }
}

impl std::fmt::Display for SessionEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SessionEventType {
    type Err = AtlasError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "FILE_READ" => Ok(Self::FileRead),
            "FILE_WRITE" => Ok(Self::FileWrite),
            "COMMAND_RUN" => Ok(Self::CommandRun),
            "COMMAND_FAIL" => Ok(Self::CommandFail),
            "GRAPH_BUILD" => Ok(Self::GraphBuild),
            "GRAPH_UPDATE" => Ok(Self::GraphUpdate),
            "REVIEW_CONTEXT" => Ok(Self::ReviewContext),
            "IMPACT_ANALYSIS" => Ok(Self::ImpactAnalysis),
            "CONTEXT_REQUEST" => Ok(Self::ContextRequest),
            "REASONING_RESULT" => Ok(Self::ReasoningResult),
            "USER_INTENT" => Ok(Self::UserIntent),
            "ERROR" => Ok(Self::Error),
            "SESSION_START" => Ok(Self::SessionStart),
            "SESSION_RESUME" => Ok(Self::SessionResume),
            other => Err(AtlasError::Other(format!(
                "unknown session event type: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewSessionEvent {
    pub session_id: SessionId,
    pub event_type: SessionEventType,
    pub priority: i32,
    pub payload: Value,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventRow {
    pub id: i64,
    pub session_id: SessionId,
    pub event_type: SessionEventType,
    pub priority: i32,
    pub payload_json: String,
    pub event_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSnapshot {
    pub session_id: SessionId,
    pub snapshot: String,
    pub event_count: i64,
    pub consumed: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub struct SessionStore {
    conn: Connection,
    config: SessionStoreConfig,
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
        let payload_json = canonical_json(&event.payload);
        self.ensure_payload_fits(&payload_json)?;
        let created_at = event.created_at.unwrap_or_else(format_now);
        let event_hash = hash_event(&event.event_type, event.priority, &payload_json);

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

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionEventRow> {
    let event_type: String = row.get(2)?;
    let event_type = event_type.parse().map_err(to_from_sql_error)?;
    Ok(SessionEventRow {
        id: row.get(0)?,
        session_id: SessionId(row.get(1)?),
        event_type,
        priority: row.get(3)?,
        payload_json: row.get(4)?,
        event_hash: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn enforce_event_limit(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    max_events_per_session: usize,
) -> Result<usize> {
    let current_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM session_events WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let current_count = current_count as usize;
    if current_count <= max_events_per_session {
        return Ok(0);
    }

    let overflow = current_count - max_events_per_session;
    tx.execute(
        "DELETE FROM session_events
         WHERE id IN (
             SELECT id
             FROM session_events
             WHERE session_id = ?1
             ORDER BY priority ASC, created_at ASC, id ASC
             LIMIT ?2
         )",
        params![session_id, overflow as i64],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;

    Ok(overflow)
}

fn hash_event(event_type: &SessionEventType, priority: i32, payload_json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(event_type.as_str().as_bytes());
    hasher.update(b"\x00");
    hasher.update(priority.to_string().as_bytes());
    hasher.update(b"\x00");
    hasher.update(payload_json.as_bytes());
    hex_encode(&hasher.finalize())
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(values) => {
            let items = values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{items}]")
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(left, _)| *left);
            let items = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}:{}", canonical_json(value))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{items}}}")
        }
    }
}

fn format_now() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| AtlasError::Other(format!("non-utf8 path: {}", path.display())))
}

fn to_from_sql_error(error: AtlasError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn open_store(
        max_events_per_session: usize,
        max_inline_payload_bytes: usize,
    ) -> (TempDir, SessionStore) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
        let store = SessionStore::open_with_config(
            path.to_str().unwrap(),
            SessionStoreConfig {
                max_events_per_session,
                max_inline_payload_bytes,
            },
        )
        .unwrap();
        (dir, store)
    }

    fn session_id() -> SessionId {
        SessionId::derive("/repo", "main", "cli")
    }

    fn seed_session(store: &mut SessionStore, session_id: &SessionId) {
        store
            .upsert_session_meta(session_id.clone(), "/repo", "cli", Some("main"))
            .unwrap();
    }

    #[test]
    fn session_meta_persists_across_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
        let session_id = session_id();

        {
            let mut store = SessionStore::open(path.to_str().unwrap()).unwrap();
            store
                .upsert_session_meta(session_id.clone(), "/repo", "cli", Some("main"))
                .unwrap();
        }

        let store = SessionStore::open(path.to_str().unwrap()).unwrap();
        let meta = store.get_session_meta(&session_id).unwrap().unwrap();
        assert_eq!(meta.repo_root, "/repo");
        assert_eq!(meta.frontend, "cli");
        assert_eq!(meta.worktree_id.as_deref(), Some("main"));
    }

    #[test]
    fn duplicate_events_deduplicate_by_hash() {
        let (_dir, mut store) = open_store(16, 1024);
        let session_id = session_id();
        seed_session(&mut store, &session_id);

        let event = NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::FileRead,
            priority: 5,
            payload: serde_json::json!({"path":"src/lib.rs","line":12}),
            created_at: Some("2026-01-01T00:00:00Z".into()),
        };

        let first = store.append_event(event.clone()).unwrap();
        let second = store.append_event(event).unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(store.list_events(&session_id).unwrap().len(), 1);
    }

    #[test]
    fn retention_evicts_lower_priority_then_older() {
        let (_dir, mut store) = open_store(2, 1024);
        let session_id = session_id();
        seed_session(&mut store, &session_id);

        for (priority, created_at, label) in [
            (1, "2026-01-01T00:00:00Z", "low-old"),
            (1, "2026-01-01T00:01:00Z", "low-new"),
            (5, "2026-01-01T00:02:00Z", "high"),
        ] {
            store
                .append_event(NewSessionEvent {
                    session_id: session_id.clone(),
                    event_type: SessionEventType::CommandRun,
                    priority,
                    payload: serde_json::json!({ "label": label }),
                    created_at: Some(created_at.into()),
                })
                .unwrap();
        }

        let events = store.list_events(&session_id).unwrap();
        let labels = events
            .iter()
            .map(|event| {
                serde_json::from_str::<Value>(&event.payload_json).unwrap()["label"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["low-new".to_string(), "high".to_string()]);
        assert!(
            store
                .get_session_meta(&session_id)
                .unwrap()
                .unwrap()
                .last_compaction_at
                .is_some()
        );
    }

    #[test]
    fn oversize_payload_rejected() {
        let (_dir, mut store) = open_store(8, 32);
        let session_id = session_id();
        seed_session(&mut store, &session_id);

        let error = store
            .append_event(NewSessionEvent {
                session_id,
                event_type: SessionEventType::CommandFail,
                priority: 10,
                payload: serde_json::json!({ "raw_output": "x".repeat(128) }),
                created_at: None,
            })
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("store raw output in content store")
        );
    }

    #[test]
    fn resume_snapshot_round_trip_and_consumption() {
        let (_dir, mut store) = open_store(16, 1024);
        let session_id = session_id();
        seed_session(&mut store, &session_id);

        store
            .put_resume_snapshot(&session_id, "{\"summary\":\"resume\"}", 7, false)
            .unwrap();
        store.mark_resume_consumed(&session_id, true).unwrap();

        let resume = store.get_resume_snapshot(&session_id).unwrap().unwrap();
        assert_eq!(resume.snapshot, "{\"summary\":\"resume\"}");
        assert_eq!(resume.event_count, 7);
        assert!(resume.consumed);
        assert!(
            store
                .get_session_meta(&session_id)
                .unwrap()
                .unwrap()
                .last_resume_at
                .is_some()
        );
    }

    #[test]
    fn open_in_repo_creates_default_session_db_path() {
        let dir = TempDir::new().unwrap();
        let session_id = session_id();

        {
            let mut store = SessionStore::open_in_repo(dir.path()).unwrap();
            seed_session(&mut store, &session_id);
        }

        let expected_path: PathBuf = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
        assert!(expected_path.exists());
    }
}
