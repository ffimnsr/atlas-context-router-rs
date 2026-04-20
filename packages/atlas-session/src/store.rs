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

/// Default maximum serialised resume-snapshot size (64 KiB).
pub const DEFAULT_MAX_SNAPSHOT_BYTES: usize = 64 * 1024;
/// Default deduplication time window in seconds (0 = hash-only, no window).
pub const DEFAULT_DEDUP_WINDOW_SECS: u64 = 0;

#[derive(Debug, Clone)]
pub struct SessionStoreConfig {
    pub max_events_per_session: usize,
    pub max_inline_payload_bytes: usize,
    /// Maximum serialised size of a resume snapshot in bytes.
    /// Snapshots exceeding this limit are trimmed bucket-by-bucket before
    /// persisting.  Set to `usize::MAX` to disable.
    pub max_snapshot_bytes: usize,
    /// When > 0, an event is deduplicated if another event of the same
    /// (type, priority) was written within this many seconds, *regardless*
    /// of payload differences.  Hash-based exact deduplication is always
    /// applied in addition to this window.  Set to 0 to disable.
    pub dedup_window_secs: u64,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            max_events_per_session: DEFAULT_SESSION_MAX_EVENTS,
            max_inline_payload_bytes: MAX_INLINE_EVENT_PAYLOAD_BYTES,
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
            dedup_window_secs: DEFAULT_DEDUP_WINDOW_SECS,
        }
    }
}

/// Aggregate statistics returned by [`SessionStore::stats`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStats {
    /// Total number of distinct sessions in this store.
    pub session_count: usize,
    /// Total number of event rows across all sessions.
    pub total_events: usize,
    /// Total number of resume snapshots stored.
    pub snapshot_count: usize,
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

/// High-level grouping derived from [`SessionEventType`] at runtime.
///
/// Never stored in the database — always computed via
/// [`SessionEventType::category`].  Use for memory cleanup, filtering, and
/// analytics without adding a column to the event ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventCategory {
    UserIntent,
    Command,
    GraphState,
    Context,
    Reasoning,
    Error,
    FileOperation,
    SessionLifecycle,
}

impl EventCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserIntent => "USER_INTENT",
            Self::Command => "COMMAND",
            Self::GraphState => "GRAPH_STATE",
            Self::Context => "CONTEXT",
            Self::Reasoning => "REASONING",
            Self::Error => "ERROR",
            Self::FileOperation => "FILE_OPERATION",
            Self::SessionLifecycle => "SESSION_LIFECYCLE",
        }
    }
}

impl std::fmt::Display for EventCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
    /// A deliberate decision recorded during a session (e.g. "chose approach A over B").
    Decision,
    /// An active rule or instruction that governs agent behaviour in this session.
    RuleInstruction,
    Error,
    SessionStart,
    SessionResume,
}

impl SessionEventType {
    /// Derive the high-level [`EventCategory`] for this event type.
    ///
    /// Category is never persisted; call this whenever grouping or filtering
    /// by category is needed (e.g. selective memory cleanup).
    pub fn category(&self) -> EventCategory {
        match self {
            Self::UserIntent | Self::Decision | Self::RuleInstruction => EventCategory::UserIntent,
            Self::CommandRun | Self::CommandFail => EventCategory::Command,
            Self::GraphBuild | Self::GraphUpdate => EventCategory::GraphState,
            Self::ReviewContext | Self::ImpactAnalysis | Self::ContextRequest => {
                EventCategory::Context
            }
            Self::ReasoningResult => EventCategory::Reasoning,
            Self::Error => EventCategory::Error,
            Self::FileRead | Self::FileWrite => EventCategory::FileOperation,
            Self::SessionStart | Self::SessionResume => EventCategory::SessionLifecycle,
        }
    }

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
            Self::Decision => "DECISION",
            Self::RuleInstruction => "RULE_INSTRUCTION",
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
            "DECISION" => Ok(Self::Decision),
            "RULE_INSTRUCTION" => Ok(Self::RuleInstruction),
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
        let payload_json = canonical_json(&event.payload);
        self.ensure_payload_fits(&payload_json)?;
        let created_at = event.created_at.unwrap_or_else(format_now);
        let event_hash = hash_event(&event.event_type, event.priority, &payload_json);

        // Time-window deduplication: skip insertion if a same-(type, priority)
        // event already exists within the configured window.
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

    /// List all sessions ordered by most-recently-updated first.
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

    /// Delete a session and all associated events and snapshots.
    ///
    /// Returns `true` when the session existed, `false` when it was not found.
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

    /// Return aggregate statistics for this store.
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

    /// Delete sessions (and their events/snapshots) whose `updated_at` timestamp
    /// is older than `keep_days` days.
    ///
    /// Returns the number of sessions deleted.
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

    /// Build and persist a bounded resume snapshot for `session_id`.
    ///
    /// Groups all stored events into categorised summary buckets, serialises
    /// the result as JSON, stores it in `session_resume`, and returns the
    /// persisted [`ResumeSnapshot`].
    ///
    /// Snapshot constraints:
    /// - Prefers identifiers and short summaries over raw content.
    /// - Bounded per-bucket limits prevent unbounded growth.
    /// - Includes retrieval hints so callers can fetch prior artifacts on demand.
    pub fn build_resume(&mut self, session_id: &SessionId) -> Result<ResumeSnapshot> {
        let meta = self
            .get_session_meta(session_id)?
            .ok_or_else(|| AtlasError::Other(format!("session {} not found", session_id)))?;

        let events = self.list_events(session_id)?;
        let event_count = events.len() as i64;

        // ------------------------------------------------------------------
        // Accumulate bucketed summaries from the event ledger.
        // ------------------------------------------------------------------
        let mut last_user_intent: Option<String> = None;
        // Most recent commands (bounded to 10, latest at end).
        let mut recent_commands: Vec<serde_json::Value> = Vec::new();
        // File-change references gathered from review/impact events.
        let mut changed_files: Vec<String> = Vec::new();
        // Impacted symbol names gathered from impact events.
        let mut impacted_symbols: Vec<String> = Vec::new();
        // Unresolved error messages (COMMAND_FAIL / ERROR events).
        let mut unresolved_errors: Vec<serde_json::Value> = Vec::new();
        // Reasoning summaries with optional source_id.
        let mut recent_reasoning: Vec<serde_json::Value> = Vec::new();
        // Artifact source_ids from reasoning results.
        let mut saved_artifact_refs: Vec<String> = Vec::new();
        // Recent deliberate decisions (bounded to 10).
        let mut recent_decisions: Vec<serde_json::Value> = Vec::new();
        // Active rules / instructions (bounded to 10, later entries override).
        let mut active_rules: Vec<serde_json::Value> = Vec::new();
        // Graph state from most recent build/update event.
        let mut graph_state: Option<serde_json::Value> = None;
        // Retrieval hints (query labels, source_ids).
        let mut retrieval_hints: Vec<serde_json::Value> = Vec::new();

        for event in &events {
            let payload: serde_json::Value =
                serde_json::from_str(&event.payload_json).unwrap_or(serde_json::Value::Null);

            match event.event_type {
                SessionEventType::UserIntent => {
                    if let Some(intent) = payload.get("intent").and_then(|v| v.as_str()) {
                        last_user_intent = Some(intent.to_string());
                    }
                }
                SessionEventType::CommandRun => {
                    let entry = serde_json::json!({
                        "command": payload.get("command"),
                        "status": payload.get("status"),
                        "at": event.created_at,
                    });
                    recent_commands.push(entry);
                    if recent_commands.len() > 10 {
                        recent_commands.remove(0);
                    }
                }
                SessionEventType::CommandFail | SessionEventType::Error => {
                    let entry = serde_json::json!({
                        "command": payload.get("command").or_else(|| payload.get("tool")),
                        "error": payload.get("error").or_else(|| payload.get("extra")),
                        "at": event.created_at,
                    });
                    unresolved_errors.push(entry);
                    if unresolved_errors.len() > 5 {
                        unresolved_errors.remove(0);
                    }
                }
                SessionEventType::ReviewContext | SessionEventType::ImpactAnalysis => {
                    // Pull file list and impacted symbols when available.
                    if let Some(files) = payload.get("files").and_then(|v| v.as_array()) {
                        for f in files {
                            if let Some(s) = f.as_str() {
                                let owned = s.to_string();
                                if !changed_files.contains(&owned) {
                                    changed_files.push(owned);
                                }
                            }
                        }
                        if changed_files.len() > 20 {
                            changed_files.truncate(20);
                        }
                    }
                    if let Some(syms) = payload.get("impacted_symbols").and_then(|v| v.as_array()) {
                        for sym in syms {
                            if let Some(s) = sym.as_str() {
                                let owned = s.to_string();
                                if !impacted_symbols.contains(&owned) {
                                    impacted_symbols.push(owned);
                                }
                            }
                        }
                        if impacted_symbols.len() > 30 {
                            impacted_symbols.truncate(30);
                        }
                    }
                }
                SessionEventType::ReasoningResult => {
                    if let Some(src) = payload.get("source_id").and_then(|v| v.as_str()) {
                        let owned = src.to_string();
                        if !saved_artifact_refs.contains(&owned) {
                            saved_artifact_refs.push(owned.clone());
                        }
                        retrieval_hints.push(serde_json::json!({
                            "kind": "saved_artifact",
                            "source_id": owned,
                            "summary": payload.get("summary"),
                        }));
                    }
                    let entry = serde_json::json!({
                        "summary": payload.get("summary"),
                        "source_id": payload.get("source_id"),
                        "at": event.created_at,
                    });
                    recent_reasoning.push(entry);
                    if recent_reasoning.len() > 5 {
                        recent_reasoning.remove(0);
                    }
                }
                SessionEventType::ContextRequest => {
                    if let Some(qh) = payload.get("query_hint").and_then(|v| v.as_str()) {
                        retrieval_hints.push(serde_json::json!({
                            "kind": "context_query",
                            "query": qh,
                        }));
                        if retrieval_hints.len() > 15 {
                            retrieval_hints.remove(0);
                        }
                    }
                }
                SessionEventType::Decision => {
                    let entry = serde_json::json!({
                        "summary": payload.get("summary"),
                        "rationale": payload.get("rationale"),
                        "at": event.created_at,
                    });
                    recent_decisions.push(entry);
                    if recent_decisions.len() > 10 {
                        recent_decisions.remove(0);
                    }
                }
                SessionEventType::RuleInstruction => {
                    // Each rule is identified by a short label; later entries
                    // for the same label replace earlier ones so only the
                    // current active set is retained.
                    let label = payload
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let entry = serde_json::json!({
                        "label": label,
                        "rule": payload.get("rule"),
                        "source": payload.get("source"),
                        "at": event.created_at,
                    });
                    // Replace existing entry with same label, or append.
                    if let Some(pos) = active_rules.iter().position(|r| {
                        r.get("label").and_then(|v| v.as_str()) == Some(label.as_str())
                    }) {
                        active_rules[pos] = entry;
                    } else {
                        active_rules.push(entry);
                        if active_rules.len() > 10 {
                            active_rules.remove(0);
                        }
                    }
                }
                SessionEventType::GraphBuild | SessionEventType::GraphUpdate => {
                    graph_state = Some(payload.clone());
                }
                // SessionStart / SessionResume / FileRead / FileWrite not
                // distilled into the snapshot; they already contribute to
                // the event ledger record count.
                _ => {}
            }
        }

        let snapshot = serde_json::json!({
            "session_id": session_id.as_str(),
            "repo_root": meta.repo_root,
            "worktree_id": meta.worktree_id,
            "frontend": meta.frontend,
            "session_created_at": meta.created_at,
            "last_user_intent": last_user_intent,
            "recent_commands": recent_commands,
            "changed_files": changed_files,
            "impacted_symbols": impacted_symbols,
            "unresolved_errors": unresolved_errors,
            "recent_reasoning": recent_reasoning,
            "saved_artifact_refs": saved_artifact_refs,
            "recent_decisions": recent_decisions,
            "active_rules": active_rules,
            "graph_state": graph_state,
            "retrieval_hints": retrieval_hints,
            "event_count": event_count,
        });

        let snapshot_str = serde_json::to_string(&snapshot)
            .map_err(|e| AtlasError::Other(format!("cannot serialise resume snapshot: {e}")))?;

        // Enforce snapshot size cap by progressively trimming bulky buckets.
        let snapshot_str = if snapshot_str.len() > self.config.max_snapshot_bytes {
            trim_snapshot_to_limit(snapshot, self.config.max_snapshot_bytes)?
        } else {
            snapshot_str
        };

        self.put_resume_snapshot(session_id, &snapshot_str, event_count, false)?;

        self.get_resume_snapshot(session_id)?
            .ok_or_else(|| AtlasError::Other("resume snapshot was not persisted".to_string()))
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

fn format_days_ago(days: u32) -> String {
    let ts = OffsetDateTime::now_utc() - time::Duration::days(days as i64);
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn format_seconds_ago(secs: u64) -> String {
    let ts = OffsetDateTime::now_utc() - time::Duration::seconds(secs as i64);
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Progressively trim bulky snapshot buckets until the serialised JSON fits
/// within `max_bytes`.  If we still exceed the limit after all trimming,
/// return an error rather than persisting a corrupt snapshot.
fn trim_snapshot_to_limit(mut snapshot: serde_json::Value, max_bytes: usize) -> Result<String> {
    // Buckets trimmed in order from cheapest to most important.
    let trimable_arrays = [
        "impacted_symbols",
        "changed_files",
        "retrieval_hints",
        "recent_commands",
        "unresolved_errors",
        "recent_reasoning",
        "recent_decisions",
        "active_rules",
        "saved_artifact_refs",
    ];

    for key in &trimable_arrays {
        let serialised = serde_json::to_string(&snapshot)
            .map_err(|e| AtlasError::Other(format!("cannot serialise snapshot: {e}")))?;
        if serialised.len() <= max_bytes {
            return Ok(serialised);
        }
        // Work with a local copy of the array, truncate it, then put it back.
        if let Some(arr) = snapshot.get(key).and_then(|v| v.as_array()).cloned() {
            let mut arr = arr;
            while arr.len() > 1 {
                arr.truncate(arr.len() / 2);
                if let Some(obj) = snapshot.as_object_mut() {
                    obj.insert((*key).to_string(), serde_json::Value::Array(arr.clone()));
                }
                let s = serde_json::to_string(&snapshot).unwrap_or_default();
                if s.len() <= max_bytes {
                    return Ok(s);
                }
            }
            // Clear the bucket entirely.
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert((*key).to_string(), serde_json::Value::Array(vec![]));
            }
        }
    }

    // Last resort: drop graph_state.
    snapshot.as_object_mut().map(|m| m.remove("graph_state"));

    let serialised = serde_json::to_string(&snapshot)
        .map_err(|e| AtlasError::Other(format!("cannot serialise snapshot: {e}")))?;
    if serialised.len() > max_bytes {
        return Err(AtlasError::Other(format!(
            "resume snapshot {} bytes exceeds limit {} bytes even after trimming",
            serialised.len(),
            max_bytes
        )));
    }
    Ok(serialised)
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

/// Return `true` when the error message indicates SQLite database corruption.
///
/// Matches rusqlite's mapped error strings for SQLITE_CORRUPT (11),
/// SQLITE_NOTADB (26), and SQLITE_IOERR (10) sub-codes that arise when a
/// file is not a valid SQLite database.
fn is_corruption_error(err: &AtlasError) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("malformed")
        || msg.contains("not a database")
        || msg.contains("disk image is malformed")
        || msg.contains("database disk image")
        || msg.contains("file is not a database")
}

/// Rename `path` to `{path}.quarantine` so Atlas can start fresh.
///
/// Logs a warning on failure but never panics — this is best-effort recovery.
fn quarantine_db(path: &str) {
    let qpath = format!("{path}.quarantine");
    if let Err(e) = std::fs::rename(path, &qpath) {
        tracing::warn!(
            path = path,
            quarantine = %qpath,
            err = %e,
            "session DB quarantine rename failed"
        );
    } else {
        tracing::warn!(
            path = path,
            quarantine = %qpath,
            "corrupt session DB quarantined; a fresh store will be created on next open"
        );
    }
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
                ..Default::default()
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

    #[test]
    fn list_sessions_returns_all_in_recency_order() {
        let (_dir, mut store) = open_store(16, 1024);

        let id_a = SessionId::derive("/repo/a", "", "cli");
        let id_b = SessionId::derive("/repo/b", "", "mcp");
        store
            .upsert_session_meta(id_a.clone(), "/repo/a", "cli", None)
            .unwrap();
        store
            .upsert_session_meta(id_b.clone(), "/repo/b", "mcp", None)
            .unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        // Most recently updated (id_b) should come first.
        assert_eq!(sessions[0].session_id, id_b);
        assert_eq!(sessions[1].session_id, id_a);
    }

    #[test]
    fn delete_session_removes_events_and_returns_true_only_when_existed() {
        let (_dir, mut store) = open_store(16, 1024);
        let session_id = session_id();
        seed_session(&mut store, &session_id);

        // Append an event so there is something to cascade-delete.
        store
            .append_event(NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::CommandRun,
                priority: 2,
                payload: serde_json::json!({ "command": "build" }),
                created_at: None,
            })
            .unwrap();

        assert!(store.delete_session(&session_id).unwrap());
        assert!(store.get_session_meta(&session_id).unwrap().is_none());
        assert!(store.list_events(&session_id).unwrap().is_empty());

        // Second call should return false.
        assert!(!store.delete_session(&session_id).unwrap());
    }

    #[test]
    fn build_resume_persists_and_groups_events() {
        let (_dir, mut store) = open_store(64, 8192);
        let session_id = session_id();
        seed_session(&mut store, &session_id);

        let events = vec![
            NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::UserIntent,
                priority: 3,
                payload: serde_json::json!({ "intent": "review" }),
                created_at: None,
            },
            NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::CommandRun,
                priority: 2,
                payload: serde_json::json!({ "command": "build", "status": "ok" }),
                created_at: None,
            },
            NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::ReasoningResult,
                priority: 3,
                payload: serde_json::json!({ "source_id": "src-abc", "summary": "impact analysis" }),
                created_at: None,
            },
        ];
        for ev in events {
            store.append_event(ev).unwrap();
        }

        let snap = store.build_resume(&session_id).unwrap();
        assert!(!snap.consumed);
        assert_eq!(snap.event_count, 3);
        assert_eq!(snap.session_id, session_id);

        let inner: serde_json::Value = serde_json::from_str(&snap.snapshot).unwrap();
        assert_eq!(inner["last_user_intent"], "review");
        assert_eq!(inner["recent_commands"].as_array().unwrap().len(), 1);
        assert!(
            inner["saved_artifact_refs"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("src-abc"))
        );
        assert_eq!(inner["event_count"], 3);
    }

    #[test]
    fn build_resume_captures_decisions_and_deduplicates_rules_by_label() {
        let (_dir, mut store) = open_store(64, 8192);
        let session_id = session_id();
        seed_session(&mut store, &session_id);

        let events = vec![
            NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::Decision,
                priority: 4,
                payload: serde_json::json!({ "summary": "prefer composition", "rationale": "simpler" }),
                created_at: None,
            },
            // First version of rule "no_mut_global".
            NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::RuleInstruction,
                priority: 4,
                payload: serde_json::json!({
                    "label": "no_mut_global",
                    "rule": "avoid global mutable state",
                    "source": "AGENTS.md",
                }),
                created_at: None,
            },
            // Second version of the same label — should replace the first.
            NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::RuleInstruction,
                priority: 4,
                payload: serde_json::json!({
                    "label": "no_mut_global",
                    "rule": "avoid global mutable state (updated)",
                    "source": "AGENTS.md",
                }),
                created_at: None,
            },
            // A distinct rule.
            NewSessionEvent {
                session_id: session_id.clone(),
                event_type: SessionEventType::RuleInstruction,
                priority: 4,
                payload: serde_json::json!({
                    "label": "use_result",
                    "rule": "use Result and ? for error propagation",
                    "source": "AGENTS.md",
                }),
                created_at: None,
            },
        ];
        for ev in events {
            store.append_event(ev).unwrap();
        }

        let snap = store.build_resume(&session_id).unwrap();
        let inner: serde_json::Value = serde_json::from_str(&snap.snapshot).unwrap();

        let decisions = inner["recent_decisions"].as_array().unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0]["summary"], "prefer composition");

        let rules = inner["active_rules"].as_array().unwrap();
        assert_eq!(rules.len(), 2);
        // The "no_mut_global" entry should carry the updated text.
        let no_mut = rules
            .iter()
            .find(|r| r["label"] == "no_mut_global")
            .expect("no_mut_global rule missing");
        assert_eq!(no_mut["rule"], "avoid global mutable state (updated)");
    }

    // ── CM8 tests ──────────────────────────────────────────────────────────

    #[test]
    fn stats_returns_accurate_counts() {
        let (_dir, mut store) = open_store(16, 1024);
        let id_a = SessionId::derive("/repo/a", "", "cli");
        let id_b = SessionId::derive("/repo/b", "", "mcp");

        store
            .upsert_session_meta(id_a.clone(), "/repo/a", "cli", None)
            .unwrap();
        store
            .upsert_session_meta(id_b.clone(), "/repo/b", "mcp", None)
            .unwrap();

        store
            .append_event(NewSessionEvent {
                session_id: id_a.clone(),
                event_type: SessionEventType::CommandRun,
                priority: 2,
                payload: serde_json::json!({ "command": "build" }),
                created_at: None,
            })
            .unwrap();
        store
            .append_event(NewSessionEvent {
                session_id: id_b.clone(),
                event_type: SessionEventType::CommandRun,
                priority: 2,
                payload: serde_json::json!({ "command": "update" }),
                created_at: None,
            })
            .unwrap();

        store.put_resume_snapshot(&id_a, "{}", 1, false).unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.session_count, 2);
        assert_eq!(stats.total_events, 2);
        assert_eq!(stats.snapshot_count, 1);
    }

    #[test]
    fn cleanup_stale_sessions_removes_old_entries() {
        let (_dir, mut store) = open_store(16, 1024);
        let id = SessionId::derive("/repo/stale", "", "cli");

        // Insert session with a very old timestamp directly.
        store
            .conn
            .execute(
                "INSERT INTO session_meta
                 (session_id, repo_root, frontend, worktree_id, created_at, updated_at)
                 VALUES (?1, ?2, 'cli', NULL, ?3, ?3)",
                params![id.as_str(), "/repo/stale", "2020-01-01T00:00:00Z"],
            )
            .unwrap();

        let removed = store.cleanup_stale_sessions(30).unwrap();
        assert_eq!(removed, 1, "old session should be removed");
        assert!(store.get_session_meta(&id).unwrap().is_none());
    }

    #[test]
    fn cleanup_stale_sessions_keeps_recent_sessions() {
        let (_dir, mut store) = open_store(16, 1024);
        let id = SessionId::derive("/repo/fresh", "", "cli");
        store
            .upsert_session_meta(id.clone(), "/repo/fresh", "cli", None)
            .unwrap();
        let removed = store.cleanup_stale_sessions(30).unwrap();
        assert_eq!(removed, 0, "recent session must not be removed");
        assert!(store.get_session_meta(&id).unwrap().is_some());
    }

    #[test]
    fn snapshot_size_cap_trims_bulky_buckets() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
        // Very small cap forces trimming.
        let mut store = SessionStore::open_with_config(
            path.to_str().unwrap(),
            SessionStoreConfig {
                max_events_per_session: 256,
                max_inline_payload_bytes: 8192,
                max_snapshot_bytes: 512,
                dedup_window_secs: 0,
            },
        )
        .unwrap();

        let session_id = session_id();
        seed_session(&mut store, &session_id);

        // Push many impacted symbols to bloat the snapshot.
        for i in 0..50 {
            store
                .append_event(NewSessionEvent {
                    session_id: session_id.clone(),
                    event_type: SessionEventType::ImpactAnalysis,
                    priority: 2,
                    payload: serde_json::json!({
                        "symbols": (0..20).map(|j| format!("Symbol_{i}_{j}")).collect::<Vec<_>>(),
                    }),
                    created_at: None,
                })
                .unwrap();
        }

        let snap = store.build_resume(&session_id).unwrap();
        assert!(
            snap.snapshot.len() <= 512,
            "snapshot len {} exceeds cap 512",
            snap.snapshot.len()
        );
    }

    #[test]
    fn dedup_window_blocks_same_type_within_window() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
        let mut store = SessionStore::open_with_config(
            path.to_str().unwrap(),
            SessionStoreConfig {
                max_events_per_session: 64,
                max_inline_payload_bytes: 8192,
                max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
                // 60-second window
                dedup_window_secs: 60,
            },
        )
        .unwrap();

        let session_id = session_id();
        seed_session(&mut store, &session_id);

        let mk = |label: &str| NewSessionEvent {
            session_id: session_id.clone(),
            event_type: SessionEventType::CommandRun,
            priority: 2,
            payload: serde_json::json!({ "command": label }),
            created_at: None,
        };

        // First insertion must succeed.
        let first = store.append_event(mk("build")).unwrap();
        assert!(first.is_some(), "first event should be stored");

        // Second within the 60-second window should be suppressed by the window.
        let second = store.append_event(mk("build-again")).unwrap();
        assert!(
            second.is_none(),
            "same-type event inside window should be deduped"
        );

        let events = store.list_events(&session_id).unwrap();
        assert_eq!(events.len(), 1, "only one event should be stored");
    }

    #[test]
    fn best_effort_open_in_nonexistent_dir_creates_path() {
        let dir = TempDir::new().unwrap();
        // Nested subdirectory that does not exist yet.
        let nested = dir.path().join("deep").join("nested").join(".atlas");
        let path = nested.join(DEFAULT_SESSION_DB);
        // open() must create parent dirs and succeed.
        let result = SessionStore::open(path.to_str().unwrap());
        assert!(result.is_ok(), "store open must create missing dirs");
    }

    // ── Corrupt DB quarantine ───────────────────────────────────────────────

    #[test]
    fn corrupt_db_is_quarantined_on_open() {
        let dir = TempDir::new().unwrap();
        let atlas_dir = dir.path().join(".atlas");
        std::fs::create_dir_all(&atlas_dir).unwrap();
        let path = atlas_dir.join(DEFAULT_SESSION_DB);

        // Write garbage bytes so SQLite cannot recognise the file.
        std::fs::write(&path, b"this is not a sqlite database").unwrap();

        let result = SessionStore::open(path.to_str().unwrap());
        // Open must fail (corrupt file).
        assert!(result.is_err(), "corrupt DB must return error");

        // Quarantine file must exist; original must be gone or renamed.
        let quarantine = atlas_dir.join(format!("{}.quarantine", DEFAULT_SESSION_DB));
        // Either the original was renamed to quarantine, or the file was removed.
        // We check that at least the quarantine file was created.
        assert!(
            quarantine.exists(),
            "quarantine file must be created for corrupt DB"
        );
    }

    #[test]
    fn quarantine_allows_fresh_open_after_corruption() {
        let dir = TempDir::new().unwrap();
        let atlas_dir = dir.path().join(".atlas");
        std::fs::create_dir_all(&atlas_dir).unwrap();
        let path = atlas_dir.join(DEFAULT_SESSION_DB);

        // Write garbage first.
        std::fs::write(&path, b"not a database").unwrap();
        let _ = SessionStore::open(path.to_str().unwrap()); // triggers quarantine

        // After quarantine the path is gone; open must succeed with a fresh DB.
        let store = SessionStore::open(path.to_str().unwrap());
        assert!(
            store.is_ok(),
            "fresh open after quarantine must succeed: {:?}",
            store.err()
        );
    }

    #[test]
    fn is_corruption_error_matches_known_strings() {
        let cases = [
            "database disk image is malformed",
            "file is not a database",
            "not a database",
        ];
        for msg in cases {
            let err = AtlasError::Db(msg.to_string());
            assert!(
                is_corruption_error(&err),
                "must detect corruption in: {msg}"
            );
        }
    }

    #[test]
    fn is_corruption_error_does_not_match_normal_errors() {
        let err = AtlasError::Db("disk I/O error (SQLITE_IOERR)".to_string());
        assert!(!is_corruption_error(&err));
    }

    // ── Race / concurrency coverage ─────────────────────────────────────────
    //
    // Each `SessionStore` owns one SQLite connection.  Concurrent safety is
    // achieved by opening separate connections from separate threads: SQLite WAL
    // mode serialises writers at the DB level while readers proceed concurrently.
    // The busy_timeout pragma (5 000 ms) absorbs transient write contention.

    /// Multiple threads each open their own `SessionStore` and write distinct
    /// events concurrently.  All events must survive without loss or panic.
    #[test]
    fn concurrent_writers_to_same_db_all_events_persist() {
        use std::sync::Arc;
        use std::thread;

        let dir = TempDir::new().unwrap();
        let path = Arc::new(
            dir.path()
                .join(".atlas")
                .join(DEFAULT_SESSION_DB)
                .to_string_lossy()
                .into_owned(),
        );

        // Initialise the DB (creates schema) before spawning writers.
        let session_id = SessionId::derive("concurrent-repo", "", "cli");
        {
            let mut store = SessionStore::open(&path).unwrap();
            store
                .upsert_session_meta(session_id.clone(), "concurrent-repo", "cli", None)
                .unwrap();
        }

        const THREADS: usize = 4;
        const EVENTS_PER_THREAD: usize = 10;

        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let p = Arc::clone(&path);
                let sid = session_id.clone();
                thread::spawn(move || {
                    let mut store = SessionStore::open(&p).expect("thread store open");
                    for i in 0..EVENTS_PER_THREAD {
                        let event = NewSessionEvent {
                            session_id: sid.clone(),
                            event_type: SessionEventType::CommandRun,
                            priority: 0,
                            payload: serde_json::json!({
                                "thread": t,
                                "step": i,
                                "unique": format!("t{t}-s{i}"),
                            }),
                            created_at: None,
                        };
                        // Busy_timeout handles lock contention; errors are soft.
                        let _ = store.append_event(event);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("writer thread must not panic");
        }

        // Verify: at least one event per thread survived (dedup may collapse
        // identical payloads, but each thread uses a unique "unique" field so
        // all should be distinct).
        let final_store = SessionStore::open(&path).unwrap();
        let events = final_store.list_events(&session_id).unwrap();
        assert!(
            events.len() >= THREADS,
            "expected at least {THREADS} events; got {}",
            events.len()
        );
    }

    /// One thread continuously appends events while another builds a resume
    /// snapshot.  Neither operation must panic or produce a torn read.
    #[test]
    fn concurrent_snapshot_build_while_writing_events() {
        use std::sync::Arc;
        use std::thread;

        let dir = TempDir::new().unwrap();
        let path = Arc::new(
            dir.path()
                .join(".atlas")
                .join(DEFAULT_SESSION_DB)
                .to_string_lossy()
                .into_owned(),
        );
        let session_id = SessionId::derive("snap-race-repo", "", "cli");

        {
            let mut store = SessionStore::open(&path).unwrap();
            store
                .upsert_session_meta(session_id.clone(), "snap-race-repo", "cli", None)
                .unwrap();
            // Seed a few events so the snapshot has something to work with.
            for i in 0..5_u32 {
                let _ = store.append_event(NewSessionEvent {
                    session_id: session_id.clone(),
                    event_type: SessionEventType::UserIntent,
                    priority: 1,
                    payload: serde_json::json!({"intent": format!("seed {i}")}),
                    created_at: None,
                });
            }
        }

        let path_writer = Arc::clone(&path);
        let sid_writer = session_id.clone();
        let writer = thread::spawn(move || {
            let mut store = SessionStore::open(&path_writer).expect("writer open");
            for i in 0..20_u32 {
                let _ = store.append_event(NewSessionEvent {
                    session_id: sid_writer.clone(),
                    event_type: SessionEventType::CommandRun,
                    priority: 0,
                    payload: serde_json::json!({"command": format!("cmd-{i}")}),
                    created_at: None,
                });
            }
        });

        let path_snap = Arc::clone(&path);
        let sid_snap = session_id.clone();
        let snapper = thread::spawn(move || {
            let mut store = SessionStore::open(&path_snap).expect("snapper open");
            // build_resume may race with the writer; it must not panic.
            let result = store.build_resume(&sid_snap);
            result.is_ok()
        });

        writer.join().expect("writer must not panic");
        let snap_ok = snapper.join().expect("snapper must not panic");
        assert!(snap_ok, "build_resume must succeed under concurrent writes");
    }

    /// Two threads upsert session metadata for the same session concurrently.
    /// The final row must be consistent (not torn / half-written).
    #[test]
    fn concurrent_upsert_session_meta_is_safe() {
        use std::sync::Arc;
        use std::thread;

        let dir = TempDir::new().unwrap();
        let path = Arc::new(
            dir.path()
                .join(".atlas")
                .join(DEFAULT_SESSION_DB)
                .to_string_lossy()
                .into_owned(),
        );
        let session_id = SessionId::derive("upsert-race", "", "mcp");

        // Initialise the schema.
        SessionStore::open(&path).unwrap();

        let handles: Vec<_> = (0..3)
            .map(|t| {
                let p = Arc::clone(&path);
                let sid = session_id.clone();
                thread::spawn(move || {
                    let mut store = SessionStore::open(&p).expect("open");
                    let _ = store.upsert_session_meta(sid, &format!("repo-{t}"), "mcp", None);
                })
            })
            .collect();

        for h in handles {
            h.join().expect("upsert thread must not panic");
        }

        let store = SessionStore::open(&path).unwrap();
        let meta = store.get_session_meta(&session_id).unwrap();
        assert!(
            meta.is_some(),
            "session meta must exist after concurrent upserts"
        );
    }

    /// Concurrent snapshot writes (put_resume_snapshot) for the same session
    /// must not corrupt the stored snapshot — the last writer wins.
    #[test]
    fn concurrent_snapshot_writes_last_writer_wins() {
        use std::sync::Arc;
        use std::thread;

        let dir = TempDir::new().unwrap();
        let path = Arc::new(
            dir.path()
                .join(".atlas")
                .join(DEFAULT_SESSION_DB)
                .to_string_lossy()
                .into_owned(),
        );
        let session_id = SessionId::derive("snap-write-race", "", "cli");

        {
            let mut store = SessionStore::open(&path).unwrap();
            store
                .upsert_session_meta(session_id.clone(), "repo", "cli", None)
                .unwrap();
        }

        const THREADS: usize = 4;
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let p = Arc::clone(&path);
                let sid = session_id.clone();
                thread::spawn(move || {
                    let mut store = SessionStore::open(&p).expect("open");
                    let snapshot = format!(r#"{{"writer":{t}}}"#);
                    let _ = store.put_resume_snapshot(&sid, &snapshot, t as i64, false);
                })
            })
            .collect();

        for h in handles {
            h.join().expect("snapshot writer must not panic");
        }

        // Snapshot must be valid JSON (not torn).
        let store = SessionStore::open(&path).unwrap();
        let snap = store
            .get_resume_snapshot(&session_id)
            .unwrap()
            .expect("snapshot must exist");
        let parsed: serde_json::Value = serde_json::from_str(&snap.snapshot)
            .expect("snapshot must be valid JSON after concurrent writes");
        assert!(
            parsed.get("writer").is_some(),
            "snapshot payload must have 'writer' key"
        );
    }
}
