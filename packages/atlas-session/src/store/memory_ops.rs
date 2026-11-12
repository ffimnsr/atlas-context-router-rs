//! VM1 — typed event–entity operator algebra for `memories`.
//!
//! VikingMem models evolving entities as deterministic operators applied to an
//! append-only event log. Atlas keeps `memories` as the single entity table and
//! applies operators through this module so numeric state updates never require
//! an LLM call, a network round trip, or ad-hoc SQL in callers.
//!
//! Operators are pure (`apply_memory_op`) plus a thin transactional wrapper
//! (`SessionStore::apply_memory_op`). `EuaPatch` delegates to
//! [`super::memory_patch`] and `TimeCompress` stamps a deterministic topic
//! rollup in metadata.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use atlas_core::{AtlasError, Clock, Result, SystemClock};

use super::SessionStore;
use super::memory::{derive_memory_id, store_memory_at};
use super::types::{MemoryImportance, MemoryScope, NewMemory, SessionEventRow, SessionEventType};
use super::util::{hex_encode, to_from_sql_error};

/// Maximum operator history entries retained per memory row.
pub const MAX_OP_HISTORY_ENTRIES: usize = 32;

/// Topic used for failing test-command events.
pub const FAILURE_TOPIC_TESTS: &str = "test-failures";

/// Topic used for failing build/compile events.
pub const FAILURE_TOPIC_BUILD: &str = "build-errors";

/// Title used to locate the single aggregated failure-log entity per topic.
const FAILURE_TITLE: &str = "Failure log";

/// Maximum distinct failure signatures retained per failure entity.
pub const FAILURE_SIGNATURE_CAP: usize = 20;

/// One typed operator applied to a memory entity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MemoryOp {
    /// Overwrite a scalar property or a `metadata.<key>` entry.
    Set { field: String, value: Value },
    /// Accumulate a numeric delta into `metadata.<field>` and `numeric_value`.
    Sum { field: String, delta: i64 },
    /// Increment an occurrence counter and stamp `last_seen`.
    Count { field: String },
    /// Retain the maximum observed numeric value or RFC 3339 timestamp.
    Max { field: String, value: Value },
    /// Retain a running average plus its sample count.
    Avg { field: String, sample: f64 },
    /// Append a unique item to a bounded JSON array field.
    AppendUnique {
        field: String,
        item: Value,
        max_items: usize,
    },
    /// Apply a VikingMem EUA search/replace patch (VM2).
    EuaPatch {
        search: String,
        replace: String,
        /// Caller-supplied threshold; default is 15% of the search length.
        #[serde(default)]
        max_edit_distance: Option<usize>,
    },
    /// Compress a topic timeline into a deterministic summary (VM2).
    TimeCompress { window_label: String },
}

impl MemoryOp {
    /// Stable operator name used in history entries and JSON output.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Set { .. } => "set",
            Self::Sum { .. } => "sum",
            Self::Count { .. } => "count",
            Self::Max { .. } => "max",
            Self::Avg { .. } => "avg",
            Self::AppendUnique { .. } => "append_unique",
            Self::EuaPatch { .. } => "eua_patch",
            Self::TimeCompress { .. } => "time_compress",
        }
    }
}

/// One bounded audit entry stored in `memories.op_history_json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryOpHistoryEntry {
    pub op: String,
    pub at: String,
    pub fields: Vec<String>,
    /// Operator-specific evidence (EUA patches record match details here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Mutable entity view loaded for operator application.
#[derive(Debug, Clone)]
pub struct MemoryEntity {
    pub id: String,
    pub repo_root: String,
    pub topic: String,
    pub title: String,
    pub body: String,
    pub importance: MemoryImportance,
    pub scope: MemoryScope,
    pub source_id: Option<String>,
    pub metadata: Value,
    pub numeric_value: Option<f64>,
    pub op_history: Vec<MemoryOpHistoryEntry>,
    pub superseded_by: Option<String>,
}

/// Result of applying one operator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpOutcome {
    pub memory_id: String,
    pub op: String,
    pub changed_fields: Vec<String>,
    pub numeric_value: Option<f64>,
    pub applied_at: String,
    pub history_entry: MemoryOpHistoryEntry,
}

/// Read-only operator history for one memory row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryOpHistory {
    pub memory_id: String,
    pub repo_root: String,
    pub numeric_value: Option<f64>,
    pub entries: Vec<MemoryOpHistoryEntry>,
}

// ── Pure operator application ────────────────────────────────────────────────

/// Apply one operator to an in-memory entity. Never touches the database.
pub fn apply_memory_op(entity: &mut MemoryEntity, op: &MemoryOp, now: &str) -> Result<OpOutcome> {
    let mut changed = BTreeSet::new();
    let mut details = None;

    match op {
        MemoryOp::Set { field, value } => {
            apply_set(entity, field, value)?;
            changed.insert(field.clone());
        }
        MemoryOp::Sum { field, delta } => {
            let previous = metadata_i64(entity, field)?;
            let next = previous.checked_add(*delta).ok_or_else(|| {
                op_error(format!(
                    "sum overflow for field '{field}': {previous} + {delta}"
                ))
            })?;
            let _ = metadata_object_mut(entity)?.insert(field.clone(), json!(next));
            entity.numeric_value = Some(next as f64);
            changed.insert(format!("metadata.{field}"));
            changed.insert("numeric_value".to_owned());
        }
        MemoryOp::Count { field } => {
            let previous = metadata_i64(entity, field)?;
            let next = previous.checked_add(1).ok_or_else(|| {
                op_error(format!(
                    "count overflow for field '{field}': {previous} + 1"
                ))
            })?;
            let object = metadata_object_mut(entity)?;
            object.insert(field.clone(), json!(next));
            object.insert("last_seen".to_owned(), json!(now));
            changed.insert(format!("metadata.{field}"));
            changed.insert("metadata.last_seen".to_owned());
        }
        MemoryOp::Max { field, value } => {
            let existing = metadata_value(entity, field);
            let next = match existing {
                Some(current) => max_value(field, current, value)?,
                None => value.clone(),
            };
            let _ = metadata_object_mut(entity)?.insert(field.clone(), next);
            changed.insert(format!("metadata.{field}"));
        }
        MemoryOp::Avg { field, sample } => {
            if !sample.is_finite() {
                return Err(op_error(format!(
                    "avg sample for field '{field}' must be finite"
                )));
            }
            let count_field = format!("{field}_count");
            let previous_count = metadata_i64(entity, &count_field)?;
            if previous_count < 0 {
                return Err(op_error(format!(
                    "avg sample count for field '{field}' must be non-negative"
                )));
            }
            let previous_avg = match metadata_value(entity, field) {
                None => 0.0,
                Some(value) => value.as_f64().ok_or_else(|| {
                    op_error(format!(
                        "avg field '{field}' must hold a numeric value or be absent"
                    ))
                })?,
            };
            let next_count = previous_count + 1;
            let next_avg = (previous_avg * previous_count as f64 + sample) / next_count as f64;
            if !next_avg.is_finite() {
                return Err(op_error(format!(
                    "avg for field '{field}' produced a non-finite value"
                )));
            }
            let object = metadata_object_mut(entity)?;
            object.insert(field.clone(), json!(next_avg));
            object.insert(count_field.clone(), json!(next_count));
            entity.numeric_value = Some(next_avg);
            changed.insert(format!("metadata.{field}"));
            changed.insert(format!("metadata.{count_field}"));
            changed.insert("numeric_value".to_owned());
        }
        MemoryOp::AppendUnique {
            field,
            item,
            max_items,
        } => {
            if *max_items == 0 {
                return Err(op_error(format!(
                    "append_unique for field '{field}' requires max_items >= 1"
                )));
            }
            let mut items = match metadata_value(entity, field) {
                None => Vec::new(),
                Some(Value::Array(values)) => values.clone(),
                Some(_) => {
                    return Err(op_error(format!(
                        "append_unique for field '{field}' requires an array value"
                    )));
                }
            };
            if !items.contains(item) {
                items.push(item.clone());
            }
            while items.len() > *max_items {
                items.remove(0);
            }
            let _ = metadata_object_mut(entity)?.insert(field.clone(), Value::Array(items));
            changed.insert(format!("metadata.{field}"));
        }
        MemoryOp::EuaPatch {
            search,
            replace,
            max_edit_distance,
        } => {
            let application = super::memory_patch::apply_patch_block(
                &entity.body,
                search,
                replace,
                *max_edit_distance,
            )?;
            entity.body = application.body;
            changed.insert("body".to_owned());
            details = Some(json!({
                "search_hash": application.search_hash,
                "replace_len": application.replace_len,
                "edit_distance": application.edit_distance,
                "matched_exact": application.matched_exact,
            }));
        }
        MemoryOp::TimeCompress { window_label } => {
            let label = window_label.trim();
            if label.is_empty() {
                return Err(op_error("time_compress requires a non-empty window label"));
            }
            let compressed_at = now.to_owned();
            let body_chars = entity.body.chars().count();
            let _ = metadata_object_mut(entity)?.insert(
                "time_compress".to_owned(),
                json!({
                    "window_label": label,
                    "compressed_at": compressed_at,
                    "body_chars": body_chars,
                }),
            );
            changed.insert("metadata.time_compress".to_owned());
        }
    }

    let entry = MemoryOpHistoryEntry {
        op: op.name().to_owned(),
        at: now.to_owned(),
        fields: changed.iter().cloned().collect(),
        details,
    };
    entity.op_history.push(entry.clone());
    if entity.op_history.len() > MAX_OP_HISTORY_ENTRIES {
        let overflow = entity.op_history.len() - MAX_OP_HISTORY_ENTRIES;
        entity.op_history.drain(0..overflow);
    }

    Ok(OpOutcome {
        memory_id: entity.id.clone(),
        op: op.name().to_owned(),
        changed_fields: changed.into_iter().collect(),
        numeric_value: entity.numeric_value,
        applied_at: now.to_owned(),
        history_entry: entry,
    })
}

fn apply_set(entity: &mut MemoryEntity, field: &str, value: &Value) -> Result<()> {
    if let Some(key) = field.strip_prefix("metadata.") {
        if key.is_empty() || key.contains('.') {
            return Err(op_error(format!(
                "invalid metadata field '{field}'; use metadata.<single_key>"
            )));
        }
        let _ = metadata_object_mut(entity)?.insert(key.to_owned(), value.clone());
        return Ok(());
    }

    match field {
        "topic" => entity.topic = required_string(field, value)?,
        "title" => entity.title = required_string(field, value)?,
        "body" => {
            let body = required_string(field, value)?;
            if body.trim().is_empty() {
                return Err(op_error("set body requires non-empty text"));
            }
            entity.body = body;
        }
        "importance" => {
            entity.importance = required_string(field, value)?.parse::<MemoryImportance>()?;
        }
        "scope" => {
            entity.scope = required_string(field, value)?.parse::<MemoryScope>()?;
        }
        "source_id" => {
            entity.source_id = match value {
                Value::Null => None,
                Value::String(text) if text.trim().is_empty() => None,
                Value::String(text) => Some(text.clone()),
                _ => {
                    return Err(op_error(
                        "set source_id requires a string or null value".to_owned(),
                    ));
                }
            };
        }
        other => {
            return Err(op_error(format!(
                "invalid set field '{other}'; expected topic, title, body, importance, scope, source_id, or metadata.<key>"
            )));
        }
    }
    Ok(())
}

fn max_value(field: &str, current: &Value, candidate: &Value) -> Result<Value> {
    match (current, candidate) {
        (Value::Number(left), Value::Number(right)) => {
            let left = left
                .as_f64()
                .ok_or_else(|| op_error(format!("max field '{field}' has a non-finite value")))?;
            let right = right
                .as_f64()
                .ok_or_else(|| op_error(format!("max field '{field}' has a non-finite value")))?;
            Ok(if right >= left {
                candidate.clone()
            } else {
                current.clone()
            })
        }
        (Value::String(left), Value::String(right)) => Ok(if right >= left {
            candidate.clone()
        } else {
            current.clone()
        }),
        _ => Err(op_error(format!(
            "max field '{field}' requires numeric or string values of the same kind"
        ))),
    }
}

fn metadata_value<'a>(entity: &'a MemoryEntity, field: &str) -> Option<&'a Value> {
    entity
        .metadata
        .as_object()
        .and_then(|object| object.get(field))
}

fn metadata_i64(entity: &MemoryEntity, field: &str) -> Result<i64> {
    match metadata_value(entity, field) {
        None => Ok(0),
        Some(Value::Number(number)) => number
            .as_i64()
            .ok_or_else(|| op_error(format!("metadata.{field} must be an integer value"))),
        Some(_) => Err(op_error(format!(
            "metadata.{field} must be an integer value"
        ))),
    }
}

fn metadata_object_mut(entity: &mut MemoryEntity) -> Result<&mut Map<String, Value>> {
    if !entity.metadata.is_object() {
        entity.metadata = Value::Object(Map::new());
    }
    entity
        .metadata
        .as_object_mut()
        .ok_or_else(|| op_error("memory metadata must be a JSON object"))
}

fn required_string(field: &str, value: &Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| op_error(format!("set field '{field}' requires a string value")))
}

fn op_error(message: impl Into<String>) -> AtlasError {
    AtlasError::Other(format!("memory operation: {}", message.into()))
}

// ── Persistence ──────────────────────────────────────────────────────────────

/// Apply one operator to a stored memory inside the caller's connection.
pub(super) fn apply_memory_op_on(
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
    op: &MemoryOp,
    now: &str,
) -> Result<OpOutcome> {
    let mut entity = load_entity(conn, repo_root, memory_id)?
        .ok_or_else(|| op_error(format!("no memory with id {memory_id} in {repo_root}")))?;
    if let Some(superseded_by) = &entity.superseded_by {
        return Err(op_error(format!(
            "memory {memory_id} is superseded by {superseded_by}; operators apply to active rows only"
        )));
    }
    let outcome = apply_memory_op(&mut entity, op, now)?;
    persist_entity(conn, &entity, now)?;
    Ok(outcome)
}

fn load_entity(
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
) -> Result<Option<MemoryEntity>> {
    conn.query_row(
        "SELECT id, repo_root, topic, title, body, importance, scope, source_id,
                metadata_json, numeric_value, op_history_json, superseded_by
         FROM memories
         WHERE id = ?1 AND repo_root = ?2",
        params![memory_id, repo_root],
        |row| {
            let importance: String = row.get(5)?;
            let scope: String = row.get(6)?;
            let metadata_json: String = row.get(8)?;
            let metadata = serde_json::from_str(&metadata_json).map_err(|error| {
                to_from_sql_error(op_error(format!(
                    "malformed metadata_json for memory {memory_id}: {error}"
                )))
            })?;
            let history_json: String = row.get(10)?;
            let op_history = parse_history(&history_json, memory_id).map_err(to_from_sql_error)?;
            Ok(MemoryEntity {
                id: row.get(0)?,
                repo_root: row.get(1)?,
                topic: row.get(2)?,
                title: row.get(3)?,
                body: row.get(4)?,
                importance: importance.parse().map_err(to_from_sql_error)?,
                scope: scope.parse().map_err(to_from_sql_error)?,
                source_id: row.get(7)?,
                metadata,
                numeric_value: row.get(9)?,
                op_history,
                superseded_by: row.get(11)?,
            })
        },
    )
    .optional()
    .map_err(|error| AtlasError::Db(error.to_string()))
}

fn persist_entity(conn: &Connection, entity: &MemoryEntity, now: &str) -> Result<()> {
    let metadata_json = serde_json::to_string(&entity.metadata)
        .map_err(|error| AtlasError::Other(format!("serialize memory metadata: {error}")))?;
    let history_json = serde_json::to_string(&entity.op_history)
        .map_err(|error| AtlasError::Other(format!("serialize operator history: {error}")))?;
    conn.execute(
        "UPDATE memories
         SET topic = ?1, title = ?2, body = ?3, importance = ?4, scope = ?5,
             source_id = ?6, metadata_json = ?7, numeric_value = ?8,
             op_history_json = ?9, updated_at = ?10
         WHERE id = ?11 AND repo_root = ?12",
        params![
            entity.topic,
            entity.title,
            entity.body,
            entity.importance.as_str(),
            entity.scope.as_str(),
            entity.source_id,
            metadata_json,
            entity.numeric_value,
            history_json,
            now,
            entity.id,
            entity.repo_root,
        ],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

fn parse_history(raw: &str, memory_id: &str) -> Result<Vec<MemoryOpHistoryEntry>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(raw).map_err(|error| {
        op_error(format!(
            "malformed op_history_json for memory {memory_id}: {error}"
        ))
    })
}

pub(super) fn memory_op_now() -> String {
    let timestamp = SystemClock
        .now_utc()
        .replace_nanosecond(0)
        .expect("0 nanoseconds is always valid");
    atlas_core::format_rfc3339(timestamp)
}

impl SessionStore {
    /// Apply one typed operator to a memory row in a single transaction.
    pub fn apply_memory_op(
        &mut self,
        repo_root: &str,
        memory_id: &str,
        op: &MemoryOp,
    ) -> Result<OpOutcome> {
        let now = memory_op_now();
        let tx = self
            .conn
            .transaction()
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        let outcome = apply_memory_op_on(&tx, repo_root, memory_id, op, &now)?;
        tx.commit()
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        Ok(outcome)
    }

    /// Read the bounded operator history for one memory row.
    pub fn memory_op_history(
        &self,
        repo_root: &str,
        memory_id: &str,
    ) -> Result<Option<MemoryOpHistory>> {
        let Some(entity) = load_entity(&self.conn, repo_root, memory_id)? else {
            return Ok(None);
        };
        Ok(Some(MemoryOpHistory {
            memory_id: entity.id,
            repo_root: entity.repo_root,
            numeric_value: entity.numeric_value,
            entries: entity.op_history,
        }))
    }
}

// ── Event-to-entity routing (Patch X command failures) ───────────────────────

/// Aggregate `COMMAND_FAIL` events into deterministic topic entities.
///
/// One failure-log memory is kept per `(repo_root, topic)`; each event applies
/// `Count(fail_count)`, `Max(last_failed_at)`, and a bounded
/// `AppendUnique(error_signatures)` update. Signatures are SHA-256 hashes of the
/// normalized failure text so raw stderr never lands in durable memory state.
pub(super) fn record_failure_events(
    conn: &Connection,
    repo_root: &str,
    events: &[SessionEventRow],
    now: &str,
) -> Result<()> {
    let mut by_topic: std::collections::BTreeMap<&'static str, Vec<(String, String)>> =
        std::collections::BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.event_type == SessionEventType::CommandFail)
    {
        let command = payload_string(&event.payload_json, &["command", "cmd"])
            .or_else(|| nested_payload_string(&event.payload_json, "extra", "command"))
            .unwrap_or_default();
        let topic = failure_topic(&command);
        let signature = failure_signature(&event.payload_json, &command);
        by_topic
            .entry(topic)
            .or_default()
            .push((signature, event.created_at.clone()));
    }
    if by_topic.is_empty() {
        return Ok(());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    for (topic, failures) in by_topic {
        let memory_id = ensure_failure_entity(&tx, repo_root, topic, now)?;
        for (signature, observed_at) in failures {
            apply_memory_op_on(
                &tx,
                repo_root,
                &memory_id,
                &MemoryOp::Count {
                    field: "fail_count".to_owned(),
                },
                now,
            )?;
            apply_memory_op_on(
                &tx,
                repo_root,
                &memory_id,
                &MemoryOp::Max {
                    field: "last_failed_at".to_owned(),
                    value: json!(observed_at),
                },
                now,
            )?;
            apply_memory_op_on(
                &tx,
                repo_root,
                &memory_id,
                &MemoryOp::AppendUnique {
                    field: "error_signatures".to_owned(),
                    item: json!(signature),
                    max_items: FAILURE_SIGNATURE_CAP,
                },
                now,
            )?;
        }
    }
    tx.commit()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

fn ensure_failure_entity(
    conn: &Connection,
    repo_root: &str,
    topic: &str,
    now: &str,
) -> Result<String> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM memories
             WHERE repo_root = ?1 AND topic = ?2 AND title = ?3 AND superseded_by IS NULL
             ORDER BY created_at ASC, id ASC
             LIMIT 1",
            params![repo_root, topic, FAILURE_TITLE],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    if let Some(id) = existing {
        return Ok(id);
    }

    let body = format!(
        "Aggregated runtime failures for topic '{topic}'. Counts and failure signatures are tracked in memory metadata."
    );
    let id = derive_memory_id(repo_root, &body, 0);
    let input = NewMemory {
        repo_root: repo_root.to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: topic.to_owned(),
        title: FAILURE_TITLE.to_owned(),
        body,
        importance: MemoryImportance::Normal,
        source_id: None,
        metadata: json!({ "entity_key": "failure_log" }),
    };
    store_memory_at(conn, &input, now, &id)?;
    Ok(id)
}

pub(super) fn failure_topic(command: &str) -> &'static str {
    let command = command.to_ascii_lowercase();
    const TEST_MARKERS: &[&str] = &[
        "cargo test",
        "cargo nextest",
        "nextest",
        "pytest",
        "go test",
        "npm test",
        "pnpm test",
        "yarn test",
        "bun test",
        "mvn test",
        "gradle test",
        "rspec",
    ];
    if TEST_MARKERS.iter().any(|marker| command.contains(marker)) {
        FAILURE_TOPIC_TESTS
    } else {
        FAILURE_TOPIC_BUILD
    }
}

pub(super) fn failure_signature(payload_json: &str, command: &str) -> String {
    let raw = payload_string(
        payload_json,
        &["error_signature", "error", "stderr", "message", "status"],
    )
    .or_else(|| nested_payload_string(payload_json, "extra", "error"))
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| {
        if command.trim().is_empty() {
            "unknown-failure".to_owned()
        } else {
            command.to_owned()
        }
    });
    let normalized = raw
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex_encode(&hasher.finalize())
}

fn payload_string(payload_json: &str, keys: &[&str]) -> Option<String> {
    let value: Value = serde_json::from_str(payload_json).ok()?;
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            return Some(text.to_owned());
        }
    }
    None
}

fn nested_payload_string(payload_json: &str, outer: &str, inner: &str) -> Option<String> {
    let value: Value = serde_json::from_str(payload_json).ok()?;
    value
        .get(outer)
        .and_then(|nested| nested.get(inner))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

// ── Schema validation (`atlas db check`) ────────────────────────────────────

/// Validate the VM1 additions: operator columns, FTS table, and triggers.
pub(super) fn schema_issues(conn: &Connection) -> Vec<String> {
    let mut issues = Vec::new();

    let fts_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if fts_exists == 0 {
        issues.push("missing table: memories_fts".to_owned());
    } else {
        let memories_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .unwrap_or(0);
        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories_fts_docsize", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        if memories_count != fts_count {
            issues.push(format!(
                "memories_fts is out of sync with memories: {fts_count} indexed rows for {memories_count} memories"
            ));
        }
    }

    for trigger in ["memories_ai", "memories_ad", "memories_au"] {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                [trigger],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            issues.push(format!("missing trigger: {trigger}"));
        }
    }

    if let Ok(mut stmt) = conn.prepare("SELECT id, op_history_json FROM memories")
        && let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
    {
        for row in rows.flatten() {
            if let Err(error) = parse_history(&row.1, &row.0) {
                issues.push(error.to_string());
            }
        }
    }

    issues
}
