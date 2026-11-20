//! M1 — markdown-truth storage for the shared memory surface.
//!
//! Memory records live as `.atlas/memories/<topic-slug>/<id>.md`. SQLite keeps
//! only derived state: `memory_index`/`memory_index_fts` for lexical recall,
//! `memory_access` for read-path bookkeeping, and `memory_embeddings` for
//! optional vectors. Reads never rewrite truth files; access bookkeeping is
//! recorded in `memory_access` so recall leaves file bytes and mtimes intact.
//!
//! The active set is derived from `supersedes` links at read time, so
//! consolidation only writes the new consolidated file and never rewrites the
//! members it replaces.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use atlas_core::{AtlasError, Clock, Result, SystemClock};
use atlas_store_markdown::slug::slugify;
use atlas_store_markdown::{Frontmatter, MarkdownRecord, MarkdownStore, Value};
use rusqlite::{Connection, params};
use serde_json::Value as JsonValue;
use time::OffsetDateTime;

use super::memory::{age_days, derive_memory_id, format_memory_now};
use super::memory_consolidate::OverflowSink;
use super::types::{
    MemoryConsolidationGroup, MemoryConsolidationPlan, MemoryDecayPolicy, MemoryDecayReport,
    MemoryDeleteResult, MemoryHealthCategory, MemoryHealthFinding, MemoryHealthReport,
    MemoryImportance, MemoryLink, MemoryListFilter, MemoryPruneResult, MemoryRecord,
    NOISY_TOPIC_ENTRIES, NewMemory, OVERSIZED_BODY_CHARS,
};

/// Durable memory directory, relative to the `.atlas/` store root.
pub(super) const MEMORIES_DIR: &str = "memories";

/// Relation vocabulary accepted by memory links: the memoir controlled
/// vocabulary plus `supersedes`, which marks the target as replaced.
pub(super) const MEMORY_LINK_RELATIONS: &[&str] = &[
    "depends_on",
    "part_of",
    "contradicts",
    "refines",
    "replaces",
    "caused_by",
    "fixed_by",
    "blocked_by",
    "decided_by",
    "related_to",
    "supersedes",
];

const RELATION_SUPERSEDES: &str = "supersedes";

// ── Paths ─────────────────────────────────────────────────────────────────────

/// Relative path for a record: `memories/<topic-slug>/<id>.md`.
pub(super) fn relative_path(topic: &str, id: &str) -> PathBuf {
    Path::new(MEMORIES_DIR)
        .join(slugify(topic))
        .join(format!("{id}.md"))
}

// ── Encoding ──────────────────────────────────────────────────────────────────

/// Serialize a memory record into canonical markdown (frontmatter + body).
pub(super) fn encode_record(record: &MemoryRecord) -> Result<MarkdownRecord> {
    let mut frontmatter = Frontmatter::new();
    frontmatter.set("id", Value::str(record.id.clone()));
    frontmatter.set("title", Value::str(record.title.clone()));
    frontmatter.set("topic", Value::str(record.topic.clone()));
    frontmatter.set("scope", Value::str(record.scope.as_str()));
    frontmatter.set("importance", Value::str(record.importance.as_str()));
    frontmatter.set(
        "tags",
        Value::list(record.tags.iter().cloned().map(Value::Str)),
    );
    frontmatter.set("session_id", optional_string(record.session_id.as_deref()));
    frontmatter.set("frontend", optional_string(record.frontend.as_deref()));
    frontmatter.set("created_at", Value::str(record.created_at.clone()));
    frontmatter.set("updated_at", Value::str(record.updated_at.clone()));
    frontmatter.set("source_id", optional_string(record.source_id.as_deref()));
    if record.links.is_empty() {
        frontmatter.set("links", Value::List(Vec::new()));
    } else {
        frontmatter.set(
            "links",
            Value::List(
                record
                    .links
                    .iter()
                    .map(|link| {
                        Value::Map(vec![
                            ("id".to_owned(), Value::Str(link.id.clone())),
                            ("relation".to_owned(), Value::Str(link.relation.clone())),
                        ])
                    })
                    .collect(),
            ),
        );
    }

    match metadata_as_flat_map(&record.metadata) {
        Some(map) => frontmatter.set("metadata", Value::Map(map)),
        None => {
            let json =
                serde_json::to_string(&public_metadata(&record.metadata)).map_err(|error| {
                    AtlasError::Other(format!("serialize memory metadata: {error}"))
                })?;
            frontmatter.set("metadata_json", Value::Str(json));
        }
    }

    if let Some(history) = record.metadata.get("__op_history") {
        let json = serde_json::to_string(history).map_err(|error| {
            AtlasError::Other(format!("serialize memory operator history: {error}"))
        })?;
        frontmatter.set("op_history", Value::Str(json));
    }
    if let Some(numeric) = record
        .metadata
        .get("__numeric_value")
        .and_then(JsonValue::as_f64)
    {
        frontmatter.set("numeric_value", Value::Float(numeric));
    }

    Ok(MarkdownRecord::new(frontmatter, record.body.as_str()))
}

/// Parse a markdown record into a [`MemoryRecord`].
pub(super) fn decode_record(
    relative: &Path,
    record: &MarkdownRecord,
    repo_root: &str,
) -> Result<MemoryRecord> {
    let frontmatter = &record.frontmatter;
    let id = required_str(frontmatter, "id", relative)?;
    let scope_raw = required_str(frontmatter, "scope", relative)?;
    let importance_raw = required_str(frontmatter, "importance", relative)?;
    let scope = scope_raw.parse().map_err(|error: AtlasError| {
        invalid(relative, format!("invalid scope '{scope_raw}': {error}"))
    })?;
    let importance = importance_raw.parse().map_err(|error: AtlasError| {
        invalid(
            relative,
            format!("invalid importance '{importance_raw}': {error}"),
        )
    })?;
    if scope == super::types::MemoryScope::Session
        && frontmatter
            .get_str("session_id")
            .map(str::trim)
            .is_none_or(str::is_empty)
    {
        return Err(invalid(
            relative,
            "scope 'session' requires a session_id".to_owned(),
        ));
    }
    if scope == super::types::MemoryScope::Frontend
        && frontmatter
            .get_str("frontend")
            .map(str::trim)
            .is_none_or(str::is_empty)
    {
        return Err(invalid(
            relative,
            "scope 'frontend' requires a frontend".to_owned(),
        ));
    }

    let tags = dedupe_tags(
        frontmatter
            .get_list("tags")
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    );
    let mut links = Vec::new();
    for entry in frontmatter.get_list("links").unwrap_or_default() {
        let Some(map) = entry.as_map() else {
            return Err(invalid(relative, "links entries must be maps".to_owned()));
        };
        let mut link_id = None;
        let mut relation = None;
        for (key, value) in map {
            match key.as_str() {
                "id" => link_id = value.as_str().map(str::to_owned),
                "relation" => relation = value.as_str().map(str::to_owned),
                _ => {}
            }
        }
        let (Some(link_id), Some(relation)) = (link_id, relation) else {
            return Err(invalid(
                relative,
                "links entries require id and relation".to_owned(),
            ));
        };
        if !MEMORY_LINK_RELATIONS.contains(&relation.as_str()) {
            return Err(invalid(
                relative,
                format!("invalid link relation '{relation}'"),
            ));
        }
        links.push(MemoryLink {
            id: link_id,
            relation,
        });
    }
    links.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.relation.cmp(&right.relation))
    });
    links.dedup();

    let metadata = match frontmatter.get_str("metadata_json") {
        Some(raw) => serde_json::from_str(raw)
            .map_err(|error| invalid(relative, format!("malformed metadata_json: {error}")))?,
        None => metadata_from_map(frontmatter.get("metadata")),
    };
    let mut metadata = metadata;
    if let Some(raw) = frontmatter.get_str("op_history") {
        let history: JsonValue = serde_json::from_str(raw)
            .map_err(|error| invalid(relative, format!("malformed op_history: {error}")))?;
        if let Some(object) = metadata.as_object_mut() {
            object.insert("__op_history".to_owned(), history);
        }
    }
    if let Some(numeric) = frontmatter.get("numeric_value") {
        let numeric = match numeric {
            Value::Float(value) => Some(*value),
            Value::Int(value) => Some(*value as f64),
            _ => None,
        };
        if let Some(numeric) = numeric
            && let Some(object) = metadata.as_object_mut()
        {
            object.insert("__numeric_value".to_owned(), serde_json::json!(numeric));
        }
    }

    Ok(MemoryRecord {
        id,
        repo_root: repo_root.to_owned(),
        session_id: frontmatter.get_str("session_id").map(str::to_owned),
        frontend: frontmatter.get_str("frontend").map(str::to_owned),
        scope,
        topic: frontmatter.get_str("topic").unwrap_or_default().to_owned(),
        title: frontmatter.get_str("title").unwrap_or_default().to_owned(),
        body: record.body.clone(),
        importance,
        created_at: frontmatter
            .get_str("created_at")
            .unwrap_or_default()
            .to_owned(),
        updated_at: frontmatter
            .get_str("updated_at")
            .unwrap_or_default()
            .to_owned(),
        last_accessed_at: frontmatter
            .get_str("created_at")
            .unwrap_or_default()
            .to_owned(),
        decay_score: 0.0,
        source_id: frontmatter.get_str("source_id").map(str::to_owned),
        metadata,
        tags,
        links,
        superseded_by: None,
    })
}

fn optional_string(value: Option<&str>) -> Value {
    value.map_or(Value::Null, Value::str)
}

/// Strip internal (`__`-prefixed) bookkeeping keys before serializing durable
/// metadata; operator history and numeric state have dedicated fields.
fn public_metadata(metadata: &JsonValue) -> JsonValue {
    match metadata {
        JsonValue::Object(object) => JsonValue::Object(
            object
                .iter()
                .filter(|(key, _)| !key.starts_with("__"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn required_str(frontmatter: &Frontmatter, key: &str, relative: &Path) -> Result<String> {
    frontmatter
        .get_str(key)
        .map(str::to_owned)
        .ok_or_else(|| invalid(relative, format!("missing required field '{key}'")))
}

fn invalid(relative: &Path, message: String) -> AtlasError {
    AtlasError::Other(format!(
        "memory frontmatter error at {}: {message}",
        relative.display()
    ))
}

fn metadata_as_flat_map(value: &JsonValue) -> Option<Vec<(String, Value)>> {
    let object = value.as_object()?;
    let mut out = Vec::with_capacity(object.len());
    for (key, value) in object {
        if key.starts_with("__") {
            continue;
        }
        let scalar = match value {
            JsonValue::Null => Value::Null,
            JsonValue::Bool(value) => Value::Bool(*value),
            JsonValue::Number(number) => match number.as_i64() {
                Some(value) => Value::Int(value),
                None => Value::Float(number.as_f64()?),
            },
            JsonValue::String(value) => Value::Str(value.clone()),
            JsonValue::Array(_) | JsonValue::Object(_) => return None,
        };
        out.push((key.clone(), scalar));
    }
    Some(out)
}

fn metadata_from_map(value: Option<&Value>) -> JsonValue {
    let Some(Value::Map(entries)) = value else {
        return JsonValue::Object(Default::default());
    };
    let mut object = serde_json::Map::new();
    for (key, value) in entries {
        let scalar = match value {
            Value::Null => JsonValue::Null,
            Value::Bool(value) => JsonValue::Bool(*value),
            Value::Int(value) => JsonValue::Number((*value).into()),
            Value::Float(value) => serde_json::Number::from_f64(*value)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            Value::Str(value) => JsonValue::String(value.clone()),
            Value::List(_) | Value::Map(_) => continue,
        };
        object.insert(key.clone(), scalar);
    }
    JsonValue::Object(object)
}

fn dedupe_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for tag in tags {
        let tag = tag.trim().to_owned();
        if tag.is_empty() {
            continue;
        }
        if seen.insert(tag.to_lowercase()) {
            out.push(tag);
        }
    }
    out
}

// ── Reads ─────────────────────────────────────────────────────────────────────

/// All memory records for a repo, derived active set applied, sorted newest
/// first. A malformed file fails the scan with its path in the error.
pub(super) fn scan(store: &MarkdownStore, repo_root: &str) -> Result<Vec<MemoryRecord>> {
    let mut records = Vec::new();
    for relative in store
        .list_markdown(Path::new(MEMORIES_DIR))
        .map_err(markdown_error)?
    {
        let record = store.read_record(&relative).map_err(markdown_error)?;
        records.push(decode_record(&relative, &record, repo_root)?);
    }
    sort_records(&mut records);
    apply_supersessions(&mut records);
    Ok(records)
}

/// Lenient scan used by list/health: malformed files become issues instead of
/// failing every other record in the directory.
pub(super) fn scan_lenient(
    store: &MarkdownStore,
    repo_root: &str,
) -> (Vec<MemoryRecord>, Vec<String>) {
    let mut records = Vec::new();
    let mut issues = Vec::new();
    let listed = match store.list_markdown(Path::new(MEMORIES_DIR)) {
        Ok(listed) => listed,
        Err(error) => return (records, vec![error.to_string()]),
    };
    for relative in listed {
        let record = match store.read_record(&relative) {
            Ok(record) => record,
            Err(error) => {
                issues.push(error.to_string());
                continue;
            }
        };
        match decode_record(&relative, &record, repo_root) {
            Ok(record) => records.push(record),
            Err(error) => issues.push(error.to_string()),
        }
    }
    sort_records(&mut records);
    apply_supersessions(&mut records);
    (records, issues)
}

fn sort_records(records: &mut [MemoryRecord]) {
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

/// Derive `superseded_by` from `supersedes` links. Deterministic when a target
/// is superseded more than once (lexicographically smallest new id wins);
/// cycles simply leave every member of the cycle superseded.
pub(super) fn apply_supersessions(records: &mut [MemoryRecord]) {
    let mut by_target: BTreeMap<String, String> = BTreeMap::new();
    for record in records.iter() {
        for link in record
            .links
            .iter()
            .filter(|link| link.relation == RELATION_SUPERSEDES)
        {
            by_target
                .entry(link.id.clone())
                .and_modify(|existing| {
                    if record.id < *existing {
                        *existing = record.id.clone();
                    }
                })
                .or_insert_with(|| record.id.clone());
        }
    }
    for record in records.iter_mut() {
        record.superseded_by = by_target.get(&record.id).cloned();
    }
}

/// Load one record by exact id within a repo.
pub(super) fn load_by_id(
    store: &MarkdownStore,
    repo_root: &str,
    memory_id: &str,
) -> Result<Option<MemoryRecord>> {
    let listed = store
        .list_markdown(Path::new(MEMORIES_DIR))
        .map_err(markdown_error)?;
    for relative in listed {
        let record = store.read_record(&relative).map_err(markdown_error)?;
        if record.frontmatter.get_str("id") != Some(memory_id) {
            continue;
        }
        let decoded = decode_record(&relative, &record, repo_root)?;
        let mut all = scan(store, repo_root)?;
        apply_supersessions(&mut all);
        let superseded_by = all
            .into_iter()
            .find(|candidate| candidate.id == memory_id)
            .and_then(|candidate| candidate.superseded_by);
        let mut decoded = decoded;
        decoded.superseded_by = superseded_by;
        return Ok(Some(decoded));
    }
    Ok(None)
}

fn strip_internal_metadata(metadata: &mut JsonValue) {
    if let Some(object) = metadata.as_object_mut() {
        object.retain(|key, _| !key.starts_with("__"));
    }
}

fn markdown_error(error: atlas_store_markdown::MarkdownError) -> AtlasError {
    AtlasError::Other(format!("markdown store error: {error}"))
}

/// Load a bounded set of records in one directory scan, with access
/// bookkeeping merged from the derived index.
pub(super) fn load_records(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, MemoryRecord>> {
    let (records, issues) = scan_lenient(store, repo_root);
    if records.is_empty() && !issues.is_empty() {
        return Err(AtlasError::Other(issues.join("; ")));
    }
    let mut selected = records
        .into_iter()
        .filter(|record| ids.contains(&record.id))
        .collect::<Vec<_>>();
    apply_access_index(conn, repo_root, &mut selected)?;
    for record in &mut selected {
        strip_internal_metadata(&mut record.metadata);
    }
    Ok(selected
        .into_iter()
        .map(|record| (record.id.clone(), record))
        .collect())
}

// ── Derived index ─────────────────────────────────────────────────────────────

/// Insert or replace one record in the derived index. The FTS triggers keep
/// `memory_index_fts` in sync.
pub(super) fn index_upsert(
    conn: &Connection,
    record: &MemoryRecord,
    relative: &Path,
) -> Result<()> {
    let metadata_json = serde_json::to_string(&public_metadata(&record.metadata))
        .map_err(|error| AtlasError::Other(format!("serialize memory metadata: {error}")))?;
    let tags_json = serde_json::to_string(&record.tags)
        .map_err(|error| AtlasError::Other(format!("serialize memory tags: {error}")))?;
    let links_json = serde_json::to_string(&record.links)
        .map_err(|error| AtlasError::Other(format!("serialize memory links: {error}")))?;
    let history = record
        .metadata
        .get("__op_history")
        .cloned()
        .unwrap_or_else(|| JsonValue::Array(Vec::new()));
    let history_json = serde_json::to_string(&history)
        .map_err(|error| AtlasError::Other(format!("serialize memory history: {error}")))?;
    let numeric = record
        .metadata
        .get("__numeric_value")
        .and_then(JsonValue::as_f64);
    conn.execute(
        "INSERT INTO memory_index
            (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
             created_at, updated_at, source_id, metadata_json, superseded_by, tags_json,
             links_json, op_history_json, numeric_value, path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
         ON CONFLICT(id) DO UPDATE SET
             repo_root = excluded.repo_root,
             session_id = excluded.session_id,
             frontend = excluded.frontend,
             scope = excluded.scope,
             topic = excluded.topic,
             title = excluded.title,
             body = excluded.body,
             importance = excluded.importance,
             created_at = excluded.created_at,
             updated_at = excluded.updated_at,
             source_id = excluded.source_id,
             metadata_json = excluded.metadata_json,
             superseded_by = excluded.superseded_by,
             tags_json = excluded.tags_json,
             links_json = excluded.links_json,
             op_history_json = excluded.op_history_json,
             numeric_value = excluded.numeric_value,
             path = excluded.path",
        params![
            record.id,
            record.repo_root,
            record.session_id,
            record.frontend,
            record.scope.as_str(),
            record.topic,
            record.title,
            record.body,
            record.importance.as_str(),
            record.created_at,
            record.updated_at,
            record.source_id,
            metadata_json,
            record.superseded_by,
            tags_json,
            links_json,
            history_json,
            numeric,
            relative.to_string_lossy(),
        ],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

/// Remove one record from the derived index (and its access/embedding rows).
pub(super) fn index_delete(conn: &Connection, memory_id: &str) -> Result<()> {
    conn.execute("DELETE FROM memory_index WHERE id = ?1", [memory_id])
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    conn.execute(
        "DELETE FROM memory_access WHERE memory_id = ?1",
        [memory_id],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    conn.execute(
        "DELETE FROM memory_embeddings WHERE memory_id = ?1",
        [memory_id],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

/// Recompute `superseded_by` for a repo from indexed `supersedes` links.
pub(super) fn refresh_supersession_index(conn: &Connection, repo_root: &str) -> Result<()> {
    let mut stmt = conn
        .prepare("SELECT id, links_json FROM memory_index WHERE repo_root = ?1")
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([repo_root], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;

    let mut by_target: BTreeMap<String, String> = BTreeMap::new();
    for (id, links_json) in &rows {
        let links: Vec<MemoryLink> = serde_json::from_str(links_json).unwrap_or_default();
        for link in links
            .into_iter()
            .filter(|link| link.relation == RELATION_SUPERSEDES)
        {
            by_target
                .entry(link.id)
                .and_modify(|existing| {
                    if id < existing {
                        *existing = id.clone();
                    }
                })
                .or_insert_with(|| id.clone());
        }
    }

    for (id, _) in rows {
        let superseded_by = by_target.get(&id);
        conn.execute(
            "UPDATE memory_index SET superseded_by = ?1 WHERE id = ?2",
            params![superseded_by, id],
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    }
    Ok(())
}

/// Rebuild the derived index for one repo from the markdown tree.
pub(super) fn rebuild_index(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
) -> Result<usize> {
    let records = scan(store, repo_root)?;
    conn.execute("DELETE FROM memory_index WHERE repo_root = ?1", [repo_root])
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    for record in &records {
        index_upsert(conn, record, &relative_path(&record.topic, &record.id))?;
    }
    refresh_supersession_index(conn, repo_root)?;
    Ok(records.len())
}

/// Rebuild the index when markdown exists but the derived table is empty, and
/// export legacy index rows that have no markdown file yet.
pub(super) fn reconcile_on_open(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    Ok(export_pending(store, conn, false)?.len())
}

/// Write an existing record's truth file and refresh its derived index row.
pub(super) fn write_existing_record(
    store: &MarkdownStore,
    conn: &Connection,
    record: &MemoryRecord,
) -> Result<()> {
    store
        .with_writer_lock("memories", || write_record_with_index(store, conn, record))
        .map_err(markdown_error)
}

// ── Legacy export ─────────────────────────────────────────────────────────────

/// Outcome of `atlas migrate markdown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownMigrationReport {
    pub dry_run: bool,
    pub written: Vec<String>,
    pub pending: usize,
    pub backup: Option<String>,
    /// Durable records (decisions, feedback, memoir, overview, tasks,
    /// experience) written by the same migration pass.
    pub records_written: usize,
}

/// Number of index rows that still lack a markdown file.
pub(super) fn pending_export_count(conn: &Connection) -> Result<usize> {
    conn.query_row(
        "SELECT COUNT(*) FROM memory_index WHERE path IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count.max(0) as usize)
    .map_err(|error| AtlasError::Db(error.to_string()))
}

/// Copy the session database to `session.db.bak` before the first export.
pub(super) fn write_migration_backup(db_path: &Path) -> Result<String> {
    let backup = db_path.with_extension("db.bak");
    if !backup.exists() {
        std::fs::copy(db_path, &backup).map_err(|error| {
            AtlasError::Other(format!(
                "copy migration backup {} -> {}: {error}",
                db_path.display(),
                backup.display()
            ))
        })?;
    }
    Ok(backup.to_string_lossy().into_owned())
}

/// Remove the pre-migration backup and the legacy supersession staging table.
pub(super) fn finalize_migration(
    db_path: &Path,
    conn: &Connection,
) -> Result<MarkdownMigrationReport> {
    if pending_export_count(conn)? > 0 {
        return Err(AtlasError::Other(
            "cannot finalize: pending memories still need a markdown file; run `atlas migrate markdown` first"
                .to_owned(),
        ));
    }
    conn.execute_batch("DROP TABLE IF EXISTS memory_legacy_supersessions")
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let backup = db_path.with_extension("db.bak");
    let removed = match std::fs::remove_file(&backup) {
        Ok(()) => Some(backup.to_string_lossy().into_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(AtlasError::Other(format!(
                "remove migration backup {}: {error}",
                backup.display()
            )));
        }
    };
    Ok(MarkdownMigrationReport {
        dry_run: false,
        written: Vec::new(),
        pending: 0,
        backup: removed,
        records_written: 0,
    })
}

/// Export index rows that have no markdown file yet (legacy sqlite rows and
/// interrupted imports). Returns the written relative paths.
pub(super) fn export_pending(
    store: &MarkdownStore,
    conn: &Connection,
    dry_run: bool,
) -> Result<Vec<PathBuf>> {
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_index'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, repo_root, session_id, frontend, scope, topic, title, body,
                    importance, created_at, updated_at, source_id, metadata_json,
                    superseded_by, tags_json, links_json, op_history_json, numeric_value
             FROM memory_index
             WHERE path IS NULL
             ORDER BY id",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let mut metadata: JsonValue =
                serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_default();
            let history_json: String = row.get(16)?;
            let history: JsonValue =
                serde_json::from_str(&history_json).unwrap_or(JsonValue::Array(Vec::new()));
            let numeric: Option<f64> = row.get(17)?;
            if let Some(object) = metadata.as_object_mut() {
                if history.as_array().is_some_and(|items| !items.is_empty()) {
                    object.insert("__op_history".to_owned(), history);
                }
                if let Some(numeric) = numeric {
                    object.insert("__numeric_value".to_owned(), serde_json::json!(numeric));
                }
            }
            Ok(MemoryRecord {
                id: row.get(0)?,
                repo_root: row.get(1)?,
                session_id: row.get(2)?,
                frontend: row.get(3)?,
                scope: row
                    .get::<_, String>(4)?
                    .parse()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                topic: row.get(5)?,
                title: row.get(6)?,
                body: row.get(7)?,
                importance: row
                    .get::<_, String>(8)?
                    .parse()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_accessed_at: row.get(9)?,
                decay_score: 0.0,
                source_id: row.get(11)?,
                metadata,
                tags: serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or_default(),
                links: serde_json::from_str(&row.get::<_, String>(15)?).unwrap_or_default(),
                superseded_by: row.get(13)?,
            })
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;

    let mut written = Vec::new();
    for mut record in rows {
        let mut links = legacy_supersession_links(conn, &record.id)?;
        links.append(&mut record.links);
        links.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.relation.cmp(&right.relation))
        });
        links.dedup();
        record.links = links;
        let relative = relative_path(&record.topic, &record.id);
        if dry_run {
            written.push(relative);
            continue;
        }
        let encoded = encode_record(&record)?;
        store
            .write_record(&relative, &encoded)
            .map_err(markdown_error)?;
        verify_written(store, &relative, &record)?;
        index_upsert(conn, &record, &relative)?;
        written.push(relative);
    }
    if !dry_run && !written.is_empty() {
        let repos = conn
            .prepare("SELECT DISTINCT repo_root FROM memory_index WHERE path IS NOT NULL")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| rows.filter_map(std::result::Result::ok).collect::<Vec<_>>())
            })
            .unwrap_or_default();
        for repo_root in repos {
            refresh_supersession_index(conn, &repo_root)?;
        }
    }
    Ok(written)
}

fn legacy_supersession_links(conn: &Connection, new_memory_id: &str) -> Result<Vec<MemoryLink>> {
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_legacy_supersessions'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare("SELECT old_memory_id FROM memory_legacy_supersessions WHERE new_memory_id = ?1")
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([new_memory_id], |row| row.get::<_, String>(0))
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|id| MemoryLink {
            id,
            relation: RELATION_SUPERSEDES.to_owned(),
        })
        .collect())
}

fn verify_written(store: &MarkdownStore, relative: &Path, expected: &MemoryRecord) -> Result<()> {
    let raw = store.read_record(relative).map_err(markdown_error)?;
    let decoded = decode_record(relative, &raw, &expected.repo_root)?;
    if decoded.id != expected.id
        || decoded.body != expected.body
        || decoded.scope != expected.scope
        || decoded.importance != expected.importance
        || decoded.topic != expected.topic
        || decoded.title != expected.title
        || decoded.tags != expected.tags
    {
        return Err(AtlasError::Other(format!(
            "markdown verification failed for {}",
            relative.display()
        )));
    }
    Ok(())
}

// ── CRUD ──────────────────────────────────────────────────────────────────────

/// Validate and persist a new memory, deriving its id and timestamps.
pub(super) fn store_memory(
    store: &MarkdownStore,
    conn: &Connection,
    input: &NewMemory,
) -> Result<MemoryRecord> {
    input.validate()?;
    let id = derive_memory_id(
        &input.repo_root,
        &input.body,
        SystemClock.now_utc().unix_timestamp_nanos(),
    );
    store_memory_at(store, conn, input, &format_memory_now(), &id)
}

/// Raw write used by [`store_memory`], consolidation, and deterministic tests.
pub(super) fn store_memory_at(
    store: &MarkdownStore,
    conn: &Connection,
    input: &NewMemory,
    now: &str,
    id: &str,
) -> Result<MemoryRecord> {
    input.validate()?;
    validate_links_for_write(store, &input.repo_root, id, &input.links)?;
    let mut metadata = input.metadata.clone();
    if let Some(object) = metadata.as_object_mut()
        && !object.contains_key("__op_history")
    {
        object.insert(
            "__op_history".to_owned(),
            serde_json::json!([{
                "op": "store",
                "at": now,
                "fields": ["body"],
            }]),
        );
    }
    let record = MemoryRecord {
        id: id.to_owned(),
        repo_root: input.repo_root.clone(),
        session_id: input.session_id.clone(),
        frontend: input.frontend.clone(),
        scope: input.scope,
        topic: input.topic.clone(),
        title: input.title.clone(),
        body: input.body.clone(),
        importance: input.importance,
        created_at: now.to_owned(),
        updated_at: now.to_owned(),
        last_accessed_at: now.to_owned(),
        decay_score: 0.0,
        source_id: input.source_id.clone(),
        metadata,
        tags: dedupe_tags(input.tags.clone()),
        links: normalize_links(&input.links),
        superseded_by: None,
    };
    store
        .with_writer_lock("memories", || write_record_with_index(store, conn, &record))
        .map_err(markdown_error)?;
    let mut returned = record;
    strip_internal_metadata(&mut returned.metadata);
    Ok(returned)
}

fn write_record_with_index(
    store: &MarkdownStore,
    conn: &Connection,
    record: &MemoryRecord,
) -> std::result::Result<(), atlas_store_markdown::MarkdownError> {
    let relative = relative_path(&record.topic, &record.id);
    let encoded = encode_record(record).map_err(|error| {
        atlas_store_markdown::MarkdownError::InvalidValue {
            field: "record".to_owned(),
            message: error.to_string(),
        }
    })?;
    store.write_record(&relative, &encoded)?;
    index_upsert(conn, record, &relative).map_err(|error| {
        atlas_store_markdown::MarkdownError::InvalidValue {
            field: "index".to_owned(),
            message: error.to_string(),
        }
    })?;
    refresh_supersession_index(conn, &record.repo_root).map_err(|error| {
        atlas_store_markdown::MarkdownError::InvalidValue {
            field: "index".to_owned(),
            message: error.to_string(),
        }
    })?;
    Ok(())
}

fn normalize_links(links: &[MemoryLink]) -> Vec<MemoryLink> {
    let mut out = links.to_vec();
    out.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.relation.cmp(&right.relation))
    });
    out.dedup();
    out
}

fn validate_links_for_write(
    store: &MarkdownStore,
    repo_root: &str,
    source_id: &str,
    links: &[MemoryLink],
) -> Result<()> {
    for link in links {
        if link.id == source_id {
            return Err(AtlasError::Other(
                "memory link must not target itself".to_owned(),
            ));
        }
        if !MEMORY_LINK_RELATIONS.contains(&link.relation.as_str()) {
            return Err(AtlasError::Other(format!(
                "invalid memory link relation '{}'; expected one of {}",
                link.relation,
                MEMORY_LINK_RELATIONS.join(", ")
            )));
        }
        if load_by_id(store, repo_root, &link.id)?.is_none() {
            return Err(AtlasError::Other(format!(
                "memory link target {} does not exist",
                link.id
            )));
        }
    }
    Ok(())
}

/// Create one directed link between two existing memories. Duplicate links
/// collapse, self-links are rejected, and the target must exist.
pub(super) fn add_memory_link(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
    target_id: &str,
    relation: &str,
    now: &str,
) -> Result<MemoryRecord> {
    if memory_id == target_id {
        return Err(AtlasError::Other(
            "memory link must not target itself".to_owned(),
        ));
    }
    if !MEMORY_LINK_RELATIONS.contains(&relation) {
        return Err(AtlasError::Other(format!(
            "invalid memory link relation '{relation}'; expected one of {}",
            MEMORY_LINK_RELATIONS.join(", ")
        )));
    }
    let mut record = load_by_id(store, repo_root, memory_id)?.ok_or_else(|| {
        AtlasError::Other(format!("no memory with id {memory_id} in {repo_root}"))
    })?;
    if load_by_id(store, repo_root, target_id)?.is_none() {
        return Err(AtlasError::Other(format!(
            "memory link target {target_id} does not exist"
        )));
    }
    let link = MemoryLink {
        id: target_id.to_owned(),
        relation: relation.to_owned(),
    };
    if record.links.contains(&link) {
        return Ok(record);
    }
    record.links.push(link);
    record.links = normalize_links(&record.links);
    record.updated_at = now.to_owned();
    store
        .with_writer_lock("memories", || write_record_with_index(store, conn, &record))
        .map_err(markdown_error)?;
    load_by_id(store, repo_root, memory_id)?.ok_or_else(|| {
        AtlasError::Other(format!("memory {memory_id} disappeared after link write"))
    })
}

/// Ensure the derived index has rows for a repo when markdown exists.
pub(super) fn ensure_index(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
) -> Result<()> {
    let indexed: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_index", [], |row| row.get(0))
        .unwrap_or(0);
    if indexed > 0 {
        return Ok(());
    }
    let on_disk = store
        .list_markdown(Path::new(MEMORIES_DIR))
        .map_err(markdown_error)?;
    if !on_disk.is_empty() {
        // Malformed files must not prevent the lenient list path from reporting
        // every issue; the index stays empty until the tree is repaired.
        let _ = rebuild_index(store, conn, repo_root);
    }
    Ok(())
}

fn indexed_ids(conn: &Connection, repo_root: &str) -> Result<BTreeSet<String>> {
    let mut stmt = conn
        .prepare("SELECT id FROM memory_index WHERE repo_root = ?1")
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([repo_root], |row| row.get::<_, String>(0))
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(rows.into_iter().collect())
}

fn record_belongs_to_repo(conn: &Connection, memory_id: &str, repo_root: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM memory_index WHERE id = ?1 AND repo_root = ?2",
        params![memory_id, repo_root],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

/// Record one recall access burst in the derived access index. Batched into a
/// single transaction so a recall never issues one write per returned memory.
pub(super) fn record_recall_access(
    conn: &Connection,
    repo_root: &str,
    memory_ids: &[String],
    now: &str,
) -> Result<()> {
    if memory_ids.is_empty() {
        return Ok(());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    for memory_id in memory_ids {
        tx.execute(
            "INSERT INTO memory_access (memory_id, repo_root, last_accessed_at, access_count, decay_score)
             VALUES (?1, ?2, ?3, 1, 0)
             ON CONFLICT(memory_id) DO UPDATE SET
                 last_accessed_at = excluded.last_accessed_at,
                 access_count = memory_access.access_count + 1",
            params![memory_id, repo_root, now],
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    }
    tx.commit()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

/// List memories for a repo, filtered and sorted by `updated_at DESC`.
pub(super) fn list_memories(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
) -> Result<Vec<MemoryRecord>> {
    ensure_index(store, conn, repo_root)?;
    let allowed = indexed_ids(conn, repo_root)?;
    let (mut records, issues) = scan_lenient(store, repo_root);
    if records.is_empty() && !issues.is_empty() {
        return Err(AtlasError::Other(issues.join("; ")));
    }
    apply_access_index(conn, repo_root, &mut records)?;
    records.retain(|record| allowed.contains(&record.id));
    for record in &mut records {
        strip_internal_metadata(&mut record.metadata);
    }
    records.retain(|record| matches_filter(record, filter));
    Ok(records)
}

fn matches_filter(record: &MemoryRecord, filter: &MemoryListFilter) -> bool {
    if !filter.include_superseded && record.superseded_by.is_some() {
        return false;
    }
    if let Some(topic) = &filter.topic
        && !record.topic.eq_ignore_ascii_case(topic)
    {
        return false;
    }
    if let Some(importance) = filter.importance
        && record.importance != importance
    {
        return false;
    }
    if let Some(scope) = filter.scope
        && record.scope != scope
    {
        return false;
    }
    if let Some(older_than) = &filter.older_than
        && record.updated_at >= *older_than
    {
        return false;
    }
    if let Some(newer_than) = &filter.newer_than
        && record.updated_at <= *newer_than
    {
        return false;
    }
    for tag in &filter.tags {
        if !record
            .tags
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(tag))
        {
            return false;
        }
    }
    true
}

fn apply_access_index(
    conn: &Connection,
    repo_root: &str,
    records: &mut [MemoryRecord],
) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT memory_id, last_accessed_at, decay_score
             FROM memory_access WHERE repo_root = ?1",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([repo_root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let access = rows
        .into_iter()
        .map(|(id, last, decay)| (id, (last, decay)))
        .collect::<BTreeMap<_, _>>();
    for record in records {
        if let Some((last, decay)) = access.get(&record.id) {
            record.last_accessed_at = last.clone();
            record.decay_score = *decay;
        }
    }
    Ok(())
}

/// Fetch one memory by exact id within a repo, with access bookkeeping merged
/// from the derived index.
pub(super) fn get_memory(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
) -> Result<Option<MemoryRecord>> {
    ensure_index(store, conn, repo_root)?;
    let Some(mut record) = load_by_id(store, repo_root, memory_id)? else {
        return Ok(None);
    };
    if !record_belongs_to_repo(conn, memory_id, repo_root) {
        return Ok(None);
    }
    let mut records = vec![record.clone()];
    apply_access_index(conn, repo_root, &mut records)?;
    record = records.remove(0);
    strip_internal_metadata(&mut record.metadata);
    Ok(Some(record))
}

/// Delete a memory by exact id within a repo.
pub(super) fn delete_memory(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
    dry_run: bool,
) -> Result<MemoryDeleteResult> {
    let found = {
        ensure_index(store, conn, repo_root)?;
        load_by_id(store, repo_root, memory_id)?.is_some()
            && record_belongs_to_repo(conn, memory_id, repo_root)
    };
    let deleted = if found && !dry_run {
        store
            .with_writer_lock("memories", || {
                let relative = find_relative_path(store, memory_id)?;
                let removed = match relative {
                    Some(relative) => store.delete_record(&relative)?,
                    None => false,
                };
                index_delete(conn, memory_id).map_err(|error| {
                    atlas_store_markdown::MarkdownError::InvalidValue {
                        field: "index".to_owned(),
                        message: error.to_string(),
                    }
                })?;
                refresh_supersession_index(conn, repo_root).map_err(|error| {
                    atlas_store_markdown::MarkdownError::InvalidValue {
                        field: "index".to_owned(),
                        message: error.to_string(),
                    }
                })?;
                Ok::<bool, atlas_store_markdown::MarkdownError>(removed)
            })
            .map_err(markdown_error)?
    } else {
        false
    };

    Ok(MemoryDeleteResult {
        memory_id: memory_id.to_owned(),
        found,
        deleted,
        dry_run,
    })
}

fn find_relative_path(
    store: &MarkdownStore,
    memory_id: &str,
) -> std::result::Result<Option<PathBuf>, atlas_store_markdown::MarkdownError> {
    for relative in store.list_markdown(Path::new(MEMORIES_DIR))? {
        let frontmatter = store.read_frontmatter(&relative)?;
        if frontmatter.get_str("id") == Some(memory_id) {
            return Ok(Some(relative));
        }
    }
    Ok(None)
}

// ── ICM-B — decay, stale, prune, health ───────────────────────────────────────

/// Compute (and, when not dry-running, persist) decay scores in the derived
/// access index. Truth files are never rewritten.
pub(super) fn decay_reports(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
    dry_run: bool,
) -> Result<Vec<MemoryDecayReport>> {
    if !policy.enabled {
        return Ok(Vec::new());
    }
    let mut full_filter = filter.clone();
    full_filter.include_superseded = true;
    let rows = list_memories(store, conn, repo_root, &full_filter)?;
    let mut reports = Vec::with_capacity(rows.len());
    let mut updates = Vec::new();
    for row in rows {
        let age = age_days(&row.updated_at, now);
        let score = policy.score(row.importance, age);
        let report_importance = row.importance;
        let protected = policy.is_protected(row.importance);
        let stale = !protected && score >= 1.0;
        if !dry_run {
            updates.push((row.id.clone(), score));
        }
        reports.push(MemoryDecayReport {
            memory: row,
            age_days: age,
            retention_days: policy.retention_days(report_importance),
            updated_decay_score: score,
            protected,
            stale,
        });
    }
    if !dry_run && !updates.is_empty() {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        for (id, score) in updates {
            tx.execute(
                "INSERT INTO memory_access (memory_id, repo_root, last_accessed_at, access_count, decay_score)
                 VALUES (?1, ?2, ?3, 0, ?4)
                 ON CONFLICT(memory_id) DO UPDATE SET decay_score = excluded.decay_score",
                params![id, repo_root, format_memory_now(), score],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        }
        tx.commit()
            .map_err(|error| AtlasError::Db(error.to_string()))?;
    }
    Ok(reports)
}

/// Rows past their retention window per policy.
pub(super) fn stale_memories(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
) -> Result<Vec<MemoryDecayReport>> {
    let reports = decay_reports(store, conn, repo_root, filter, policy, now, true)?;
    Ok(reports.into_iter().filter(|report| report.stale).collect())
}

/// Delete (or, in dry-run, only report) memories past retention.
#[allow(clippy::too_many_arguments)]
pub(super) fn prune_memories(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
    dry_run: bool,
    allow_critical: bool,
) -> Result<MemoryPruneResult> {
    if !policy.enabled {
        return Ok(MemoryPruneResult {
            dry_run,
            candidate_count: 0,
            deleted_count: 0,
            protected_count: 0,
            candidates: Vec::new(),
        });
    }
    if !allow_critical && filter.importance == Some(MemoryImportance::Critical) {
        return Err(AtlasError::Other(
            "critical memories are protected by memory.decay.critical_never_prune; \
             pass --allow-critical to include them in prune candidates"
                .to_owned(),
        ));
    }
    let reports = decay_reports(store, conn, repo_root, filter, policy, now, true)?;
    let mut protected_count = 0usize;
    let mut candidates = Vec::new();
    for report in reports {
        if report.protected && !allow_critical {
            protected_count += 1;
            continue;
        }
        if report.stale {
            candidates.push(report.memory);
        }
    }
    let deleted_count = if dry_run {
        0
    } else {
        let mut deleted = 0usize;
        for memory in &candidates {
            let result = delete_memory(store, conn, repo_root, &memory.id, false)?;
            deleted += usize::from(result.deleted);
        }
        deleted
    };
    Ok(MemoryPruneResult {
        dry_run,
        candidate_count: candidates.len(),
        deleted_count,
        protected_count,
        candidates,
    })
}

/// Deterministic health report, including malformed files and dangling links.
pub(super) fn health_report(
    store: &MarkdownStore,
    _conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
    source_exists: &dyn Fn(&str) -> bool,
) -> Result<MemoryHealthReport> {
    let (all_records, issues) = scan_lenient(store, repo_root);
    let rows = all_records
        .iter()
        .filter(|record| matches_filter(record, filter))
        .cloned()
        .collect::<Vec<_>>();
    let known_ids = all_records
        .iter()
        .map(|record| record.id.clone())
        .collect::<BTreeSet<_>>();
    let mut findings = Vec::new();
    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |category: MemoryHealthCategory| {
        *by_category.entry(category.as_str().to_owned()).or_insert(0) += 1;
    };

    for issue in issues {
        count(MemoryHealthCategory::Orphaned);
        findings.push(MemoryHealthFinding {
            category: MemoryHealthCategory::Orphaned,
            kind: "malformed_frontmatter".to_owned(),
            memory_id: None,
            topic: None,
            detail: issue.clone(),
            suggestion: "repair or delete the malformed memory file".to_owned(),
            command: "atlas doctor".to_owned(),
        });
    }

    for record in &rows {
        for link in &record.links {
            if !known_ids.contains(&link.id) {
                count(MemoryHealthCategory::Orphaned);
                findings.push(MemoryHealthFinding {
                    category: MemoryHealthCategory::Orphaned,
                    kind: "dangling_link".to_owned(),
                    memory_id: Some(record.id.clone()),
                    topic: (!record.topic.is_empty()).then(|| record.topic.clone()),
                    detail: format!(
                        "memory {} links ({}) to missing memory {}",
                        record.id, link.relation, link.id
                    ),
                    suggestion: "re-link or remove the dangling memory link".to_owned(),
                    command: format!("atlas memory link {} <target> {}", record.id, link.relation),
                });
            }
        }
    }

    for (index, row) in rows.iter().enumerate() {
        let age = age_days(&row.updated_at, now);
        let protected = policy.is_protected(row.importance);
        let stale = !protected && policy.score(row.importance, age) >= 1.0;
        if stale {
            count(MemoryHealthCategory::Stale);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Stale,
                kind: "stale_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} in topic '{}' is past its retention window",
                    row.id, row.topic
                ),
                suggestion: "refresh the fact or prune the row".to_owned(),
                command: prune_command(&row.topic),
            });
            continue;
        }

        let duplicated = rows[..index].iter().any(|other| {
            other.topic == row.topic
                && ((!other.title.trim().is_empty()
                    && normalize_text(&other.title) == normalize_text(&row.title))
                    || normalize_text(&other.body) == normalize_text(&row.body))
        });
        if duplicated {
            count(MemoryHealthCategory::Duplicated);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Duplicated,
                kind: "duplicated_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} repeats an earlier memory in topic '{}'",
                    row.id, row.topic
                ),
                suggestion: "merge the duplicates into one fact".to_owned(),
                command: consolidate_command(&row.topic),
            });
            continue;
        }

        let orphaned = row
            .source_id
            .as_deref()
            .is_some_and(|source_id| !source_exists(source_id));
        if orphaned {
            count(MemoryHealthCategory::Orphaned);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Orphaned,
                kind: "orphaned_source".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} references a missing saved-context artifact",
                    row.id
                ),
                suggestion: "re-link the source artifact or delete the memory".to_owned(),
                command: format!("atlas memory delete {} --dry-run", row.id),
            });
            continue;
        }

        if row.body.chars().count() > OVERSIZED_BODY_CHARS {
            count(MemoryHealthCategory::Oversized);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Oversized,
                kind: "oversized_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} exceeds {} characters",
                    row.id, OVERSIZED_BODY_CHARS
                ),
                suggestion: "split the body into one-fact memories".to_owned(),
                command: format!("atlas memory store \"<fact>\" --topic {}", row.topic),
            });
        }
    }

    let mut topic_stats: BTreeMap<&str, (usize, bool)> = BTreeMap::new();
    for row in &rows {
        if row.topic.is_empty() {
            continue;
        }
        let entry = topic_stats.entry(row.topic.as_str()).or_insert((0, false));
        entry.0 += 1;
        entry.1 |= row.importance == MemoryImportance::Critical;
    }
    for (topic, (entries, has_critical)) in topic_stats {
        if entries > NOISY_TOPIC_ENTRIES {
            count(MemoryHealthCategory::Noisy);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Noisy,
                kind: "noisy_topic".to_owned(),
                memory_id: None,
                topic: Some(topic.to_owned()),
                detail: format!(
                    "topic '{topic}' has {entries} memories (noisy above {NOISY_TOPIC_ENTRIES})"
                ),
                suggestion: "consolidate the topic into fewer facts".to_owned(),
                command: consolidate_command(topic),
            });
        }
        if !has_critical {
            count(MemoryHealthCategory::Noisy);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Noisy,
                kind: "topic_without_critical".to_owned(),
                memory_id: None,
                topic: Some(topic.to_owned()),
                detail: format!(
                    "topic '{topic}' has {entries} memories but no critical decision memory"
                ),
                suggestion: "record the key decision with critical importance".to_owned(),
                command: format!(
                    "atlas memory store \"<decision>\" --topic {topic} --importance critical"
                ),
            });
        }
    }

    Ok(MemoryHealthReport {
        total_memories: rows.len(),
        findings,
        by_category,
    })
}

// ── Consolidation ─────────────────────────────────────────────────────────────

/// Deterministic consolidation planner/applier. Apply mode writes one new
/// consolidated file whose `supersedes` links replace the members; member
/// files are never rewritten.
pub(super) fn consolidation_plan(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    dry_run: bool,
    mut overflow: Option<&mut OverflowSink<'_>>,
) -> Result<MemoryConsolidationPlan> {
    let rows = list_memories(store, conn, repo_root, filter)?;

    #[derive(Clone)]
    struct MemberKeys {
        title: Option<String>,
        body: String,
        source: Option<String>,
        category: Option<String>,
    }

    struct Group {
        members: Vec<MemoryRecord>,
        keys: Vec<MemberKeys>,
    }

    let mut groups: Vec<Group> = Vec::new();
    for row in rows {
        let keys = MemberKeys {
            title: (!row.title.trim().is_empty()).then(|| normalize_text(&row.title)),
            body: normalize_text(&row.body),
            source: row.source_id.clone(),
            category: memory_category(&row.metadata),
        };
        let matches = |other: &MemberKeys| -> bool {
            (keys.title.is_some() && other.title.is_some() && keys.title == other.title)
                || keys.body == other.body
                || (keys.source.is_some() && keys.source == other.source)
                || (keys.category.is_some() && keys.category == other.category)
        };
        let mut matched: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group.members[0].topic == row.topic && group.keys.iter().any(&matches)
            })
            .map(|(index, _)| index)
            .collect();
        if matched.is_empty() {
            groups.push(Group {
                members: vec![row],
                keys: vec![keys],
            });
        } else {
            matched.sort_unstable();
            let first = matched.remove(0);
            for extra in matched.into_iter().rev() {
                let merged = groups.remove(extra);
                groups[first].members.extend(merged.members);
                groups[first].keys.extend(merged.keys);
            }
            groups[first].members.push(row);
            groups[first].keys.push(keys);
        }
    }

    let _now = format_memory_now();
    let mut plan_groups = Vec::new();
    let mut kept_ids = Vec::new();
    let mut merged_ids = Vec::new();

    for group in groups.into_iter().filter(|group| group.members.len() >= 2) {
        let members = group.members;
        let kept = &members[0];
        let group_key = consolidation_group_key(&members);
        let source_ids = members
            .iter()
            .filter_map(|member| member.source_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let merged = members[1..]
            .iter()
            .map(|member| member.id.clone())
            .collect::<Vec<_>>();

        let synthesis = super::memory_consolidate::synthesize_consolidated_body(&members);
        let member_ids = members
            .iter()
            .map(|member| member.id.clone())
            .collect::<Vec<_>>();
        let planned_overflow_id = synthesis
            .overflow
            .then(|| super::memory_consolidate::overflow_source_id(repo_root, &member_ids));
        let unrouted_overflow = synthesis.overflow && !dry_run && overflow.is_none();
        let overflow_id = if unrouted_overflow {
            None
        } else {
            planned_overflow_id
        };
        let row_body = if unrouted_overflow {
            synthesis
                .full_body
                .clone()
                .unwrap_or_else(|| synthesis.body.clone())
        } else {
            synthesis.body.clone()
        };

        let importance = members
            .iter()
            .map(|member| importance_rank(member.importance))
            .min()
            .map(importance_from_rank)
            .unwrap_or(kept.importance);
        let mut metadata = kept.metadata.clone();
        metadata["consolidated"] = serde_json::json!(true);
        metadata["merged_ids"] = serde_json::to_value(&merged).unwrap_or_default();
        metadata["merged_source_ids"] = serde_json::to_value(&source_ids).unwrap_or_default();
        metadata["merged_unit_count"] = serde_json::json!(synthesis.distinct_units);
        metadata["overflow_source_id"] = serde_json::to_value(&overflow_id).unwrap_or_default();
        let metadata_json = serde_json::to_string(&metadata).map_err(|error| {
            AtlasError::Other(format!("serialize consolidated metadata: {error}"))
        })?;

        let consolidated_id = if dry_run {
            None
        } else {
            if let (Some(full_body), Some(sink)) =
                (synthesis.full_body.as_deref(), overflow.as_deref_mut())
            {
                super::memory_consolidate::route_overflow_body(
                    sink,
                    repo_root,
                    &member_ids,
                    kept.session_id.as_deref(),
                    full_body,
                )?;
            }
            let input = NewMemory {
                repo_root: repo_root.to_owned(),
                session_id: kept.session_id.clone(),
                frontend: kept.frontend.clone(),
                scope: kept.scope,
                topic: kept.topic.clone(),
                title: kept.title.clone(),
                body: row_body.clone(),
                importance,
                source_id: kept.source_id.clone(),
                tags: kept.tags.clone(),
                metadata,
                links: member_ids
                    .iter()
                    .map(|id| MemoryLink {
                        id: id.clone(),
                        relation: RELATION_SUPERSEDES.to_owned(),
                    })
                    .collect(),
            };
            let record = store_memory(store, conn, &input)?;
            Some(record.id)
        };

        kept_ids.push(kept.id.clone());
        merged_ids.extend(merged.iter().cloned());
        plan_groups.push(MemoryConsolidationGroup {
            topic: kept.topic.clone(),
            group_key,
            kept_memory_id: kept.id.clone(),
            merged_memory_ids: merged,
            source_ids: source_ids.clone(),
            body: row_body,
            metadata_json,
            merged_source_ids: source_ids,
            overflow_source_id: overflow_id,
            consolidated_id,
        });
    }

    Ok(MemoryConsolidationPlan {
        dry_run,
        groups: plan_groups,
        kept_ids,
        merged_ids,
    })
}

/// Deterministic grouping reason, in fixed priority order.
fn consolidation_group_key(members: &[MemoryRecord]) -> String {
    for (a, b) in pairs(members) {
        if !a.title.trim().is_empty()
            && !b.title.trim().is_empty()
            && normalize_text(&a.title) == normalize_text(&b.title)
        {
            return "same_title".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if normalize_text(&a.body) == normalize_text(&b.body) {
            return "same_body".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if a.source_id.is_some() && a.source_id == b.source_id {
            return "same_source".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if memory_category(&a.metadata).is_some()
            && memory_category(&a.metadata) == memory_category(&b.metadata)
        {
            return "same_category".to_owned();
        }
    }
    "same_body".to_owned()
}

fn pairs<T>(items: &[T]) -> impl Iterator<Item = (&T, &T)> {
    (0..items.len()).flat_map(move |i| (i + 1..items.len()).map(move |j| (&items[i], &items[j])))
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn memory_category(metadata: &JsonValue) -> Option<String> {
    metadata
        .get("category")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn importance_rank(importance: MemoryImportance) -> u8 {
    match importance {
        MemoryImportance::Critical => 0,
        MemoryImportance::High => 1,
        MemoryImportance::Normal => 2,
        MemoryImportance::Low => 3,
    }
}

fn importance_from_rank(rank: u8) -> MemoryImportance {
    match rank {
        0 => MemoryImportance::Critical,
        1 => MemoryImportance::High,
        2 => MemoryImportance::Normal,
        _ => MemoryImportance::Low,
    }
}

fn prune_command(topic: &str) -> String {
    if topic.is_empty() {
        "atlas memory prune --dry-run".to_owned()
    } else {
        format!("atlas memory prune --topic {topic} --dry-run")
    }
}

fn consolidate_command(topic: &str) -> String {
    if topic.is_empty() {
        "atlas memory consolidate --dry-run".to_owned()
    } else {
        format!("atlas memory consolidate --topic {topic} --dry-run")
    }
}

// ── Integrity ─────────────────────────────────────────────────────────────────

/// Markdown-tree integrity issues surfaced by `atlas db check`: malformed
/// frontmatter and dangling links, reported per file/id.
pub(super) fn tree_integrity(store: &MarkdownStore, repo_root: &str) -> Vec<String> {
    let (records, issues) = scan_lenient(store, repo_root);
    let mut out = issues;
    let known = records
        .iter()
        .map(|record| record.id.as_str())
        .collect::<BTreeSet<_>>();
    for record in &records {
        for link in &record.links {
            if !known.contains(link.id.as_str()) {
                out.push(format!(
                    "dangling memory link: {} -> {} ({})",
                    record.id, link.id, link.relation
                ));
            }
        }
    }
    out
}
