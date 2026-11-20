//! M2/M3 read-back: import markdown truth into the derived SQLite tables.
//!
//! `.atlas/` markdown is authoritative. This module is the inverse of
//! [`super::markdown_records`]: on store open, existing files update or insert
//! their derived rows so hand edits survive and a deleted `session.db` rebuilds
//! from markdown instead of losing continuity state.
//!
//! Import is lenient: one malformed file produces an issue with its path and
//! never blocks the remaining records. Rows are upserted, never replaced, so
//! foreign-key cascades (memoir graphs, experience deps) cannot drop derived
//! state by accident.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::{MarkdownStore, Value};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params_from_iter};

use super::memoir::derive_memoir_relation_id;

/// Summary of one import pass.
#[derive(Debug, Default, Clone)]
pub(super) struct ImportReport {
    pub records: usize,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone)]
struct ColumnInfo {
    name: String,
    not_null: bool,
    has_default: bool,
    primary_key: bool,
}

/// One simple markdown family: flat frontmatter columns plus body sections.
struct ImportSpec {
    table: &'static str,
    id_column: &'static str,
    dir: &'static str,
    /// `(column, heading)` pairs copied from plain body sections.
    body_sections: &'static [(&'static str, &'static str)],
    /// `(column, heading)` pairs copied from fenced JSON sections.
    fenced_sections: &'static [(&'static str, &'static str)],
    /// Read a checkbox status line from the body (`durable_tasks`).
    checkbox_status: bool,
}

const SPECS: &[ImportSpec] = &[
    ImportSpec {
        table: "decision_memory",
        id_column: "decision_id",
        dir: "decisions",
        body_sections: &[
            ("summary", "Summary"),
            ("rationale", "Rationale"),
            ("conclusion", "Conclusion"),
        ],
        fenced_sections: &[],
        checkbox_status: false,
    },
    ImportSpec {
        table: "feedback_records",
        id_column: "id",
        dir: "feedback",
        body_sections: &[
            ("predicted", "Predicted"),
            ("actual", "Actual"),
            ("correction", "Correction"),
        ],
        fenced_sections: &[],
        checkbox_status: false,
    },
    ImportSpec {
        table: "durable_tasks",
        id_column: "task_id",
        dir: "tasks",
        body_sections: &[("status_message", "Status message")],
        fenced_sections: &[
            ("progress_json", "Progress"),
            ("result_json", "Result"),
            ("error_json", "Error"),
        ],
        checkbox_status: true,
    },
    ImportSpec {
        table: "experience_edges",
        id_column: "id",
        dir: "experience",
        body_sections: &[("trace_summary", "Trace"), ("query_text", "Query")],
        fenced_sections: &[],
        checkbox_status: false,
    },
    ImportSpec {
        table: "overview_records",
        id_column: "id",
        dir: "overview",
        body_sections: &[
            ("summary", "Abstract (L0)"),
            ("description_preview", "Overview (L1)"),
        ],
        fenced_sections: &[
            ("responsibilities_json", "Responsibilities"),
            ("flow_json", "Flow"),
            ("inputs_json", "Inputs"),
            ("outputs_json", "Outputs"),
            ("invariants_json", "Invariants"),
            ("gotchas_json", "Gotchas"),
            ("examples_json", "Examples"),
        ],
        checkbox_status: false,
    },
];

/// Import every markdown family into the derived session tables.
pub(super) fn import_all(store: &MarkdownStore, conn: &Connection) -> Result<ImportReport> {
    let _foreign_keys = ForeignKeysGuard::disable(conn)?;
    let mut report = ImportReport::default();

    import_sessions(store, conn, &mut report);
    for spec in SPECS {
        import_table(store, conn, spec, &mut report);
    }
    repair_import_links(conn);
    rebuild_overview_fts(conn);
    import_memoir(store, conn, &mut report);

    Ok(report)
}

/// Temporarily disable FK enforcement for import. A deleted `session.db` can
/// leave durable decisions whose session rows were local-only, and a rebuild
/// must not drop those records.
struct ForeignKeysGuard<'a> {
    conn: &'a Connection,
}

impl<'a> ForeignKeysGuard<'a> {
    fn disable(conn: &'a Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys=OFF")
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        Ok(Self { conn })
    }
}

impl Drop for ForeignKeysGuard<'_> {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch("PRAGMA foreign_keys=ON");
    }
}

fn table_columns(conn: &Connection, table: &str) -> Vec<ColumnInfo> {
    let sql = format!("PRAGMA table_info(\"{table}\")");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |row| {
        Ok(ColumnInfo {
            name: row.get::<_, String>(1)?,
            not_null: row.get::<_, i64>(3)? != 0,
            has_default: row.get::<_, Option<String>>(4)?.is_some(),
            primary_key: row.get::<_, i64>(5)? != 0,
        })
    });
    match rows {
        Ok(rows) => rows.filter_map(std::result::Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

/// Repair foreign keys that markdown can reference after a database rebuild.
/// Decisions keep their exact text even when the local session ledger was
/// cleaned; missing session rows get a stub and dangling event pointers are
/// cleared so later writes cannot fail FK checks.
fn repair_import_links(conn: &Connection) {
    if table_exists(conn, "decision_memory") && table_exists(conn, "session_meta") {
        let _ = conn.execute_batch(
            "INSERT OR IGNORE INTO session_meta
                (session_id, repo_root, frontend, created_at, updated_at)
             SELECT DISTINCT session_id, repo_root, 'cli', created_at, updated_at
             FROM decision_memory
             WHERE session_id NOT IN (SELECT session_id FROM session_meta);
             UPDATE decision_memory SET event_id = NULL
             WHERE event_id IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM session_events WHERE session_events.id = decision_memory.event_id
               );",
        );
    }
    if table_exists(conn, "overview_records") {
        let _ = conn.execute_batch(
            "UPDATE overview_records SET supersedes_id = NULL
             WHERE supersedes_id IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM overview_records older
                   WHERE older.id = overview_records.supersedes_id
               );",
        );
    }
}

/// Overview FTS has no triggers; rebuild it after rows are imported.
fn rebuild_overview_fts(conn: &Connection) {
    if !table_exists(conn, "overview_records") || !table_exists(conn, "overview_record_fts") {
        return;
    }
    let _ = conn.execute_batch(
        "DELETE FROM overview_record_fts;
         INSERT INTO overview_record_fts
            (overview_id, title, summary, description_preview, responsibilities_json,
             flow_json, invariants_json, gotchas_json, tags_json)
         SELECT id, title, summary, description_preview, responsibilities_json,
                flow_json, invariants_json, gotchas_json, tags_json
         FROM overview_records;",
    );
}

fn import_table(
    store: &MarkdownStore,
    conn: &Connection,
    spec: &ImportSpec,
    report: &mut ImportReport,
) {
    if !table_exists(conn, spec.table) {
        return;
    }
    let columns = table_columns(conn, spec.table);
    if columns.is_empty() {
        report
            .issues
            .push(format!("{}: no columns available", spec.table));
        return;
    }
    let column_names = columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<BTreeSet<_>>();

    for relative in list_files(store, spec.dir, report) {
        let record = match store.read_record(&relative) {
            Ok(record) => record,
            Err(error) => {
                report
                    .issues
                    .push(format!("{}: {error}", relative.display()));
                continue;
            }
        };
        let Some(id) = record.frontmatter.get_str(spec.id_column) else {
            report.issues.push(format!(
                "{}: missing required frontmatter field '{}'",
                relative.display(),
                spec.id_column
            ));
            continue;
        };
        if id.trim().is_empty() {
            report.issues.push(format!(
                "{}: empty frontmatter field '{}'",
                relative.display(),
                spec.id_column
            ));
            continue;
        }

        let mut values: BTreeMap<String, SqlValue> = BTreeMap::new();
        for (key, value) in record.frontmatter.iter() {
            if key == spec.id_column || !column_names.contains(key) {
                continue;
            }
            if let Some(value) = value_to_sql(key, value) {
                values.insert(key.to_owned(), value);
            }
        }
        for (column, heading) in spec.body_sections {
            if let Some(text) = body_section(&record.body, heading) {
                values.insert((*column).to_owned(), SqlValue::Text(text));
            }
        }
        for (column, heading) in spec.fenced_sections {
            if let Some(text) = body_section(&record.body, heading)
                && let Some(payload) = fenced_payload(&text)
            {
                values.insert((*column).to_owned(), SqlValue::Text(payload));
            }
        }
        if spec.checkbox_status
            && let Some(status) = checkbox_status(&record.body)
        {
            values.insert("status".to_owned(), SqlValue::Text(status));
        }

        match upsert_row(conn, spec.table, spec.id_column, id, &columns, &values) {
            Ok(()) => report.records += 1,
            Err(message) => report
                .issues
                .push(format!("{}: {message}", relative.display())),
        }
    }
}

fn list_files(store: &MarkdownStore, dir: &str, report: &mut ImportReport) -> Vec<PathBuf> {
    match store.list_markdown(Path::new(dir)) {
        Ok(paths) => paths,
        Err(error) => {
            // A missing directory is normal; only surface real errors.
            if store.absolute(Path::new(dir)).exists() {
                report.issues.push(format!("{dir}: {error}"));
            }
            Vec::new()
        }
    }
}

fn value_to_sql(key: &str, value: &Value) -> Option<SqlValue> {
    Some(match value {
        Value::Null => SqlValue::Null,
        Value::Bool(value) => SqlValue::Integer(i64::from(*value)),
        Value::Int(value) => SqlValue::Integer(*value),
        Value::Float(value) => SqlValue::Real(*value),
        Value::Str(value) => SqlValue::Text(value.clone()),
        Value::List(_) | Value::Map(_) => {
            if key.ends_with("_json") {
                SqlValue::Text(serde_json::to_string(&value_to_json(value)?).ok()?)
            } else {
                return None;
            }
        }
    })
}

fn upsert_row(
    conn: &Connection,
    table: &str,
    id_column: &str,
    id: &str,
    columns: &[ColumnInfo],
    values: &BTreeMap<String, SqlValue>,
) -> std::result::Result<(), String> {
    let exists: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM \"{table}\" WHERE \"{id_column}\" = ?1"),
            [id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;

    if exists > 0 {
        let assignments = columns
            .iter()
            .filter(|column| column.name != id_column && values.contains_key(column.name.as_str()))
            .map(|column| format!("\"{}\" = ?", column.name))
            .collect::<Vec<_>>();
        if assignments.is_empty() {
            return Ok(());
        }
        let mut params = columns
            .iter()
            .filter(|column| column.name != id_column && values.contains_key(column.name.as_str()))
            .filter_map(|column| values.get(column.name.as_str()).cloned())
            .collect::<Vec<_>>();
        params.push(SqlValue::Text(id.to_owned()));
        let sql = format!(
            "UPDATE \"{table}\" SET {} WHERE \"{id_column}\" = ?{}",
            assignments.join(", "),
            params.len()
        );
        conn.execute(&sql, params_from_iter(params))
            .map(|_| ())
            .map_err(|error| error.to_string())
    } else {
        let missing = columns.iter().any(|column| {
            column.not_null
                && !column.has_default
                && !column.primary_key
                && column.name != id_column
                && !values.contains_key(column.name.as_str())
        });
        if missing {
            return Err(format!(
                "cannot insert into {table}: required column missing from markdown"
            ));
        }
        let mut names = vec![id_column.to_owned()];
        let mut params = vec![SqlValue::Text(id.to_owned())];
        for column in columns {
            if column.name == id_column {
                continue;
            }
            if let Some(value) = values.get(column.name.as_str()) {
                names.push(column.name.clone());
                params.push(value.clone());
            }
        }
        let placeholders = (1..=params.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO \"{table}\" ({}) VALUES ({placeholders})",
            names
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", ")
        );
        conn.execute(&sql, params_from_iter(params))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

// ── Body parsing ─────────────────────────────────────────────────────────────

/// Extract one `## heading` section, stopping at the next `##` heading.
fn body_section(body: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}");
    let mut lines = body.lines();
    let mut found = false;
    let mut out = Vec::new();
    for line in lines.by_ref() {
        if !found {
            if line.trim_end() == marker {
                found = true;
            }
            continue;
        }
        if line.starts_with("## ") {
            break;
        }
        out.push(line);
    }
    found.then(|| {
        out.join("\n")
            .trim_start_matches('\n')
            .trim_end()
            .to_owned()
    })
}

/// Strip a fenced code block and return its payload.
fn fenced_payload(section: &str) -> Option<String> {
    let mut lines = section.lines();
    let first = lines.next()?.trim();
    if !first.starts_with("```") {
        return None;
    }
    let fence_len = first.chars().take_while(|ch| *ch == '`').count();
    let closing = "`".repeat(fence_len);
    let mut out = Vec::new();
    for line in lines {
        if line.trim_end() == closing {
            return Some(out.join("\n"));
        }
        out.push(line);
    }
    None
}

fn value_to_json(value: &Value) -> Option<serde_json::Value> {
    Some(match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(value) => serde_json::Value::Bool(*value),
        Value::Int(value) => serde_json::Value::Number((*value).into()),
        Value::Float(value) => {
            serde_json::Number::from_f64(*value).map(serde_json::Value::Number)?
        }
        Value::Str(value) => serde_json::Value::String(value.clone()),
        Value::List(items) => serde_json::Value::Array(
            items
                .iter()
                .map(value_to_json)
                .collect::<Option<Vec<_>>>()?,
        ),
        Value::Map(entries) => serde_json::Value::Object(
            entries
                .iter()
                .map(|(key, value)| Some((key.clone(), value_to_json(value)?)))
                .collect::<Option<serde_json::Map<_, _>>>()?,
        ),
    })
}

fn checkbox_status(body: &str) -> Option<String> {
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("- [x] ")
            .or_else(|| line.strip_prefix("- [X] "))
            .or_else(|| line.strip_prefix("- [ ] "))
        else {
            continue;
        };
        let status = rest.split_whitespace().next().unwrap_or("").trim();
        if !status.is_empty() {
            return Some(status.to_owned());
        }
    }
    None
}

// ── Sessions ─────────────────────────────────────────────────────────────────

fn import_sessions(store: &MarkdownStore, conn: &Connection, report: &mut ImportReport) {
    if !table_exists(conn, "session_meta") {
        return;
    }
    for relative in list_files(store, "sessions", report) {
        let file_name = relative.file_name().and_then(|name| name.to_str());
        match file_name {
            Some("events.md") => import_session_events(store, conn, &relative, report),
            Some("resume.md") => import_session_resume(store, conn, &relative, report),
            _ => {}
        }
    }
}

fn import_session_events(
    store: &MarkdownStore,
    conn: &Connection,
    relative: &Path,
    report: &mut ImportReport,
) {
    let record = match store.read_record(relative) {
        Ok(record) => record,
        Err(error) => {
            report
                .issues
                .push(format!("{}: {error}", relative.display()));
            return;
        }
    };
    let Some(session_id) = record.frontmatter.get_str("session_id") else {
        report.issues.push(format!(
            "{}: missing session_id frontmatter",
            relative.display()
        ));
        return;
    };
    let repo_root = record.frontmatter.get_str("repo_root").unwrap_or("");
    let frontend = record.frontmatter.get_str("frontend").unwrap_or("cli");
    let created_at = record
        .frontmatter
        .get_str("created_at")
        .unwrap_or("1970-01-01T00:00:00Z");
    let updated_at = record
        .frontmatter
        .get_str("updated_at")
        .unwrap_or(created_at);
    if let Err(error) = conn.execute(
        "INSERT INTO session_meta
            (session_id, repo_root, frontend, worktree_id, created_at, updated_at,
             last_resume_at, last_compaction_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)
         ON CONFLICT(session_id) DO UPDATE SET
             repo_root = excluded.repo_root,
             frontend = excluded.frontend,
             worktree_id = excluded.worktree_id,
             created_at = excluded.created_at,
             updated_at = excluded.updated_at,
             last_compaction_at = excluded.last_compaction_at",
        rusqlite::params![
            session_id,
            repo_root,
            frontend,
            record.frontmatter.get_str("worktree_id"),
            created_at,
            updated_at,
            record.frontmatter.get_str("last_compaction_at"),
        ],
    ) {
        report
            .issues
            .push(format!("{}: {error}", relative.display()));
        return;
    }

    let existing: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_events WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if existing > 0 {
        return;
    }

    for event in parse_events(&record.body) {
        if let Err(error) = conn.execute(
            "INSERT OR IGNORE INTO session_events
                (session_id, event_type, priority, payload_json, event_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                session_id,
                event.event_type,
                event.priority,
                event.payload,
                event.event_hash,
                event.created_at,
            ],
        ) {
            report
                .issues
                .push(format!("{}: {error}", relative.display()));
        }
    }
}

struct ParsedEvent {
    created_at: String,
    event_type: String,
    priority: i64,
    event_hash: String,
    payload: String,
}

fn parse_events(body: &str) -> Vec<ParsedEvent> {
    let mut out = Vec::new();
    let mut lines = body.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(header) = line.strip_prefix("### ") else {
            continue;
        };
        let parts = header.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 4 {
            continue;
        }
        let Ok(priority) = parts[2].parse::<i64>() else {
            continue;
        };
        // Find the opening fenced JSON block for this event.
        let mut fence_len = 0usize;
        let mut started = false;
        let mut payload = Vec::new();
        for next in lines.by_ref() {
            let trimmed = next.trim();
            if !started {
                if trimmed.starts_with("```") {
                    let candidate = trimmed.chars().take_while(|ch| *ch == '`').count();
                    if trimmed[candidate..].trim_start().starts_with("json") {
                        fence_len = candidate;
                        started = true;
                    }
                }
                continue;
            }
            let closing = "`".repeat(fence_len);
            if trimmed == closing {
                break;
            }
            payload.push(next);
        }
        if !started {
            continue;
        }
        out.push(ParsedEvent {
            created_at: parts[0].to_owned(),
            event_type: parts[1].to_owned(),
            priority,
            event_hash: parts[3].to_owned(),
            payload: payload.join("\n"),
        });
    }
    out
}

fn import_session_resume(
    store: &MarkdownStore,
    conn: &Connection,
    relative: &Path,
    report: &mut ImportReport,
) {
    let record = match store.read_record(relative) {
        Ok(record) => record,
        Err(error) => {
            report
                .issues
                .push(format!("{}: {error}", relative.display()));
            return;
        }
    };
    let Some(session_id) = record.frontmatter.get_str("session_id") else {
        return;
    };
    let Some(snapshot) = fenced_payload(record.body.trim()) else {
        return;
    };
    let event_count = record.frontmatter.get_i64("event_count").unwrap_or(0);
    let consumed = record.frontmatter.get_bool("consumed").unwrap_or(false);
    let updated_at = record
        .frontmatter
        .get_str("updated_at")
        .unwrap_or("1970-01-01T00:00:00Z");
    if let Err(error) = conn.execute(
        "INSERT INTO session_resume
            (session_id, snapshot, event_count, consumed, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(session_id) DO UPDATE SET
             snapshot = excluded.snapshot,
             event_count = excluded.event_count,
             consumed = excluded.consumed,
             updated_at = excluded.updated_at",
        rusqlite::params![
            session_id,
            snapshot,
            event_count,
            i64::from(consumed),
            updated_at
        ],
    ) {
        report
            .issues
            .push(format!("{}: {error}", relative.display()));
    }
}

// ── Memoir ───────────────────────────────────────────────────────────────────

fn import_memoir(store: &MarkdownStore, conn: &Connection, report: &mut ImportReport) {
    if !table_exists(conn, "memoir_graphs") || !table_exists(conn, "memoir_concepts") {
        return;
    }
    let records = list_files(store, "memoir", report);
    let mut graph_by_dir: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut graph_records = Vec::new();
    let mut concept_records = Vec::new();

    for relative in records {
        let record = match store.read_record(&relative) {
            Ok(record) => record,
            Err(error) => {
                report
                    .issues
                    .push(format!("{}: {error}", relative.display()));
                continue;
            }
        };
        if relative.file_name().and_then(|name| name.to_str()) == Some("_graph.md") {
            if let Some(id) = record.frontmatter.get_str("id")
                && let Some(parent) = relative.parent()
            {
                graph_by_dir.insert(parent.to_path_buf(), id.to_owned());
            }
            graph_records.push((relative, record));
        } else {
            concept_records.push((relative, record));
        }
    }

    let graph_columns = table_columns(conn, "memoir_graphs");
    for (relative, record) in graph_records {
        match import_memoir_graph(conn, &graph_columns, &record) {
            Ok(()) => report.records += 1,
            Err(message) => report
                .issues
                .push(format!("{}: {message}", relative.display())),
        }
    }

    let concept_columns = table_columns(conn, "memoir_concepts");
    let relation_columns = table_columns(conn, "memoir_relations");
    let mut pending_relations = Vec::new();
    for (relative, record) in concept_records {
        let graph_id = record
            .frontmatter
            .get_str("graph_id")
            .map(str::to_owned)
            .or_else(|| {
                relative
                    .parent()
                    .and_then(|parent| graph_by_dir.get(parent))
                    .cloned()
            });
        let Some(graph_id) = graph_id else {
            report.issues.push(format!(
                "{}: cannot resolve graph for memoir concept",
                relative.display()
            ));
            continue;
        };
        let Some(concept_id) = record.frontmatter.get_str("id").map(str::to_owned) else {
            report
                .issues
                .push(format!("{}: memoir concept missing id", relative.display()));
            continue;
        };
        let mut values = BTreeMap::new();
        values.insert("graph_id".to_owned(), SqlValue::Text(graph_id.clone()));
        for (key, value) in record.frontmatter.iter() {
            if key == "id" || key == "relations" || key == "graph_name" {
                continue;
            }
            if let Some(value) = value_to_sql(key, value) {
                values.insert(key.to_owned(), value);
            }
        }
        match upsert_row(
            conn,
            "memoir_concepts",
            "id",
            &concept_id,
            &concept_columns,
            &values,
        ) {
            Ok(()) => report.records += 1,
            Err(message) => {
                report
                    .issues
                    .push(format!("{}: {message}", relative.display()));
                continue;
            }
        }
        pending_relations.push((
            relative,
            graph_id,
            concept_id,
            record
                .frontmatter
                .get_list("relations")
                .unwrap_or_default()
                .to_vec(),
            record
                .frontmatter
                .get_str("created_at")
                .unwrap_or("")
                .to_owned(),
        ));
    }

    // Relations resolve targets by concept name, so every concept must be in
    // the derived table before any relation is linked.
    for (relative, graph_id, concept_id, relations, created_at) in pending_relations {
        import_memoir_relations(
            conn,
            &relation_columns,
            &graph_id,
            &concept_id,
            &relations,
            &created_at,
            report,
            &relative,
        );
    }
}

fn import_memoir_graph(
    conn: &Connection,
    columns: &[ColumnInfo],
    record: &atlas_store_markdown::MarkdownRecord,
) -> std::result::Result<(), String> {
    let Some(id) = record.frontmatter.get_str("id") else {
        return Err("memoir graph missing id".to_owned());
    };
    let mut values = BTreeMap::new();
    for (key, value) in record.frontmatter.iter() {
        if key == "id" {
            continue;
        }
        if let Some(value) = value_to_sql(key, value) {
            values.insert(key.to_owned(), value);
        }
    }
    upsert_row(conn, "memoir_graphs", "id", id, columns, &values)
}

#[allow(clippy::too_many_arguments)]
fn import_memoir_relations(
    conn: &Connection,
    columns: &[ColumnInfo],
    graph_id: &str,
    source_id: &str,
    relations: &[Value],
    created_at: &str,
    report: &mut ImportReport,
    relative: &Path,
) {
    if columns.is_empty() {
        return;
    }
    let mut expected = BTreeSet::new();
    for entry in relations {
        let Some(map) = entry.as_map() else {
            continue;
        };
        let mut target = None;
        let mut relation = None;
        let mut confidence = 1.0f64;
        for (key, value) in map {
            match key.as_str() {
                "target" => target = value.as_str().map(str::to_owned),
                "relation" => relation = value.as_str().map(str::to_owned),
                "confidence" => {
                    confidence = match value {
                        Value::Float(value) => *value,
                        Value::Int(value) => *value as f64,
                        _ => value
                            .as_str()
                            .and_then(|raw| raw.parse().ok())
                            .unwrap_or(1.0),
                    };
                }
                _ => {}
            }
        }
        let (Some(target), Some(relation)) = (target, relation) else {
            continue;
        };
        if confidence.is_nan() || !(0.0..=1.0).contains(&confidence) {
            continue;
        }
        if !super::memoir::MEMOIR_RELATION_TYPES.contains(&relation.as_str()) {
            continue;
        }
        let target_id: Option<String> = conn
            .query_row(
                "SELECT id FROM memoir_concepts WHERE graph_id = ?1 AND name = ?2 COLLATE NOCASE",
                rusqlite::params![graph_id, target],
                |row| row.get(0),
            )
            .ok();
        let Some(target_id) = target_id else {
            report.issues.push(format!(
                "{}: relation target '{target}' not found",
                relative.display()
            ));
            continue;
        };
        let relation_id = derive_memoir_relation_id(graph_id, source_id, &target_id, &relation);
        expected.insert(relation_id.clone());
        let repo_root = conn
            .query_row(
                "SELECT repo_root FROM memoir_concepts WHERE id = ?1",
                [source_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default();
        if let Err(error) = conn.execute(
            "INSERT INTO memoir_relations
                (id, graph_id, repo_root, source_concept_id, target_concept_id,
                 relation_type, confidence, source_id, created_at, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, '{}')
             ON CONFLICT(id) DO UPDATE SET confidence = excluded.confidence",
            rusqlite::params![
                relation_id,
                graph_id,
                repo_root,
                source_id,
                target_id,
                relation,
                confidence,
                if created_at.is_empty() {
                    "1970-01-01T00:00:00Z"
                } else {
                    created_at
                }
            ],
        ) {
            report.issues.push(format!(
                "{}: memoir relation insert failed: {error}",
                relative.display()
            ));
        }
    }

    // Remove relations hand-deleted from the concept file.
    let existing = conn
        .prepare("SELECT id FROM memoir_relations WHERE source_concept_id = ?1")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([source_id], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(std::result::Result::ok).collect::<Vec<_>>())
        })
        .unwrap_or_default();
    for relation_id in existing {
        if !expected.contains(&relation_id) {
            let _ = conn.execute("DELETE FROM memoir_relations WHERE id = ?1", [relation_id]);
        }
    }
}
