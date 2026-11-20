//! M2 — markdown truth files for durable continuity records.
//!
//! SQLite tables stay as derived, rebuildable indexes. Writes go through this
//! module so the markdown tree under `.atlas/` is the durable record:
//!
//! - `.atlas/decisions/<id>.md`
//! - `.atlas/feedback/<id>.md`
//! - `.atlas/memoir/<graph>/_graph.md` plus concept files
//! - `.atlas/overview/<subject_kind>/<subject_id>.md`
//! - `.atlas/tasks/<id>.md` (local-only)
//! - `.atlas/experience/<id>.md` (local-only)
//!
//! Canonical serialization keeps files byte-stable: frontmatter fields follow
//! the table column order and bodies use fixed section headings.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::slug::slugify;
use atlas_store_markdown::{Frontmatter, MarkdownRecord, MarkdownStore, Value};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Row};

/// Sync every markdown-backed family from the derived indexes. Existing files
/// are only rewritten when their canonical bytes change, so repeated opens are
/// cheap and preserve mtimes for untouched records.
pub(super) fn sync_all(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    let mut written = 0usize;
    written += sync_decisions(store, conn)?;
    written += sync_feedback(store, conn)?;
    written += sync_tasks(store, conn)?;
    written += sync_experience(store, conn)?;
    written += sync_overview(store, conn)?;
    written += sync_memoir(store, conn)?;
    Ok(written)
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

fn sync_table(
    store: &MarkdownStore,
    conn: &Connection,
    table: &str,
    query: &str,
    path_for: impl Fn(&Row<'_>) -> Result<PathBuf>,
    body_for: impl Fn(&Row<'_>) -> Result<String>,
    skip_columns: &[&str],
) -> Result<usize> {
    if !table_exists(conn, table) {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(query)
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let columns = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut rows = stmt
        .query([])
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let mut written = 0usize;
    while let Some(row) = rows
        .next()
        .map_err(|error| AtlasError::Db(error.to_string()))?
    {
        let relative = path_for(row)?;
        let mut frontmatter = Frontmatter::new();
        for (index, column) in columns.iter().enumerate() {
            if skip_columns.contains(&column.as_str()) {
                continue;
            }
            let value = match row
                .get_ref(index)
                .map_err(|error| AtlasError::Db(error.to_string()))?
            {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(value) => Value::Int(value),
                ValueRef::Real(value) => Value::Float(value),
                ValueRef::Text(bytes) => Value::Str(String::from_utf8_lossy(bytes).into_owned()),
                ValueRef::Blob(_) => continue,
            };
            frontmatter.set(column.clone(), value);
        }
        let body = body_for(row)?;
        let record = MarkdownRecord::new(frontmatter, body);
        let bytes = record
            .to_bytes(&relative)
            .map_err(|error| AtlasError::Other(format!("serialize markdown: {error}")))?;
        if write_if_changed(store, &relative, &bytes)? {
            written += 1;
        }
    }
    Ok(written)
}

fn write_if_changed(store: &MarkdownStore, relative: &Path, bytes: &[u8]) -> Result<bool> {
    let absolute = store.absolute(relative);
    if let Ok(existing) = std::fs::read(&absolute)
        && existing == bytes
    {
        return Ok(false);
    }
    store
        .write_bytes(relative, bytes)
        .map_err(|error| AtlasError::Other(format!("write markdown: {error}")))?;
    Ok(true)
}

fn text(row: &Row<'_>, column: &str) -> Result<String> {
    row.get::<_, Option<String>>(column)
        .map_err(|error| AtlasError::Db(error.to_string()))
        .map(Option::unwrap_or_default)
}

fn optional_text(row: &Row<'_>, column: &str) -> Result<Option<String>> {
    row.get::<_, Option<String>>(column)
        .map_err(|error| AtlasError::Db(error.to_string()))
}

fn render_section(title: &str, body: &str) -> String {
    if body.trim().is_empty() {
        String::new()
    } else {
        format!("## {title}\n\n{}\n\n", body.trim_end())
    }
}

fn fenced(label: &str, body: &str) -> String {
    if body.trim().is_empty() || body.trim() == "null" {
        return String::new();
    }
    let fence = "`".repeat(4);
    format!("## {label}\n\n{fence}json\n{body}\n{fence}\n\n")
}

// ── Decisions ────────────────────────────────────────────────────────────────

pub(super) fn sync_decisions(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    sync_table(
        store,
        conn,
        "decision_memory",
        "SELECT * FROM decision_memory ORDER BY decision_id",
        |row| Ok(Path::new("decisions").join(format!("{}.md", safe_id(&text(row, "decision_id")?)))),
        |row| {
            let mut body = String::new();
            body.push_str(&render_section("Summary", &text(row, "summary")?));
            body.push_str(&render_section(
                "Rationale",
                &optional_text(row, "rationale")?.unwrap_or_default(),
            ));
            body.push_str(&render_section(
                "Conclusion",
                &optional_text(row, "conclusion")?.unwrap_or_default(),
            ));
            Ok(body)
        },
        &[],
    )
}

// ── Feedback ─────────────────────────────────────────────────────────────────

pub(super) fn sync_feedback(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    sync_table(
        store,
        conn,
        "feedback_records",
        "SELECT * FROM feedback_records ORDER BY id",
        |row| Ok(Path::new("feedback").join(format!("{}.md", safe_id(&text(row, "id")?)))),
        |row| {
            let mut body = String::new();
            body.push_str(&render_section("Predicted", &text(row, "predicted")?));
            body.push_str(&render_section("Actual", &text(row, "actual")?));
            body.push_str(&render_section("Correction", &text(row, "correction")?));
            Ok(body)
        },
        &[],
    )
}

// ── Durable tasks ────────────────────────────────────────────────────────────

pub(super) fn sync_tasks(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    sync_table(
        store,
        conn,
        "durable_tasks",
        "SELECT * FROM durable_tasks ORDER BY task_id",
        |row| Ok(Path::new("tasks").join(format!("{}.md", safe_id(&text(row, "task_id")?)))),
        |row| {
            let status = text(row, "status")?;
            let checked = if status == "completed" { "x" } else { " " };
            let mut body = format!("- [{checked}] {status}\n\n");
            if let Some(message) = optional_text(row, "status_message")? {
                body.push_str(&render_section("Status message", &message));
            }
            if let Some(progress) = optional_text(row, "progress_json")? {
                body.push_str(&fenced("Progress", &progress));
            }
            if let Some(result) = optional_text(row, "result_json")? {
                body.push_str(&fenced("Result", &result));
            }
            if let Some(error) = optional_text(row, "error_json")? {
                body.push_str(&fenced("Error", &error));
            }
            Ok(body)
        },
        &[],
    )
}

/// Remove a durable task markdown file.
pub(super) fn delete_task(store: &MarkdownStore, task_id: &str) -> Result<()> {
    delete_file(
        store,
        &Path::new("tasks").join(format!("{}.md", safe_id(task_id))),
    )
}

// ── Experience edges ─────────────────────────────────────────────────────────

pub(super) fn sync_experience(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    sync_table(
        store,
        conn,
        "experience_edges",
        "SELECT * FROM experience_edges ORDER BY id",
        |row| Ok(Path::new("experience").join(format!("{}.md", safe_id(&text(row, "id")?)))),
        |row| {
            let mut body = render_section("Trace", &text(row, "trace_summary")?);
            body.push_str(&render_section("Query", &text(row, "query_text")?));
            body.push_str(&render_section(
                "Edge",
                &format!(
                    "{} -> {}",
                    text(row, "source_uri")?,
                    text(row, "target_uri")?
                ),
            ));
            Ok(body)
        },
        &["query_embedding"],
    )
}

// ── Overview ─────────────────────────────────────────────────────────────────

pub(super) fn sync_overview(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    sync_table(
        store,
        conn,
        "overview_records",
        "SELECT * FROM overview_records ORDER BY id",
        |row| {
            let kind = safe_id(&text(row, "subject_kind")?);
            let subject = safe_id(&text(row, "subject_id")?);
            Ok(Path::new("overview")
                .join(kind)
                .join(format!("{subject}.md")))
        },
        |row| {
            let mut body = render_section("Abstract (L0)", &text(row, "summary")?);
            body.push_str(&render_section(
                "Overview (L1)",
                &text(row, "description_preview")?,
            ));
            for (column, title) in [
                ("responsibilities_json", "Responsibilities"),
                ("flow_json", "Flow"),
                ("inputs_json", "Inputs"),
                ("outputs_json", "Outputs"),
                ("invariants_json", "Invariants"),
                ("gotchas_json", "Gotchas"),
                ("examples_json", "Examples"),
            ] {
                if let Some(value) = optional_text(row, column)? {
                    body.push_str(&fenced(title, &value));
                }
            }
            Ok(body)
        },
        &[],
    )
}

/// Remove one overview record file by id.
pub(super) fn delete_overview(store: &MarkdownStore, record_id: &str) -> Result<()> {
    // The file path is derived from subject kind and subject id, so remove any
    // matching overview file rather than guessing the pair.
    for relative in store
        .list_markdown(Path::new("overview"))
        .map_err(|error| AtlasError::Other(format!("list overview markdown: {error}")))?
    {
        let frontmatter = store
            .read_frontmatter(&relative)
            .map_err(|error| AtlasError::Other(format!("read overview markdown: {error}")))?;
        if frontmatter.get_str("id") == Some(record_id) {
            delete_file(store, &relative)?;
        }
    }
    Ok(())
}

// ── Memoir ───────────────────────────────────────────────────────────────────

pub(super) fn sync_memoir(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    let mut written = 0usize;
    written += sync_table(
        store,
        conn,
        "memoir_graphs",
        "SELECT * FROM memoir_graphs ORDER BY id",
        |row| {
            Ok(Path::new("memoir")
                .join(slugify(&text(row, "name")?))
                .join("_graph.md"))
        },
        |row| Ok(render_section("Description", &text(row, "description")?)),
        &[],
    )?;

    if !table_exists(conn, "memoir_concepts") {
        return Ok(written);
    }
    let mut stmt = conn
        .prepare(
            "SELECT c.*, g.name AS graph_name
             FROM memoir_concepts c
             JOIN memoir_graphs g ON g.id = c.graph_id
             ORDER BY c.id",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let columns = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    let mut rows = stmt
        .query([])
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| AtlasError::Db(error.to_string()))?
    {
        let graph_slug = slugify(&text(row, "graph_name")?);
        let relative = Path::new("memoir")
            .join(graph_slug)
            .join(format!("{}.md", slugify(&text(row, "name")?)));
        let relations = concept_relations(conn, &text(row, "id")?)?;
        let mut frontmatter = Frontmatter::new();
        for (index, column) in columns.iter().enumerate() {
            if column == "graph_name" {
                continue;
            }
            let value = match row
                .get_ref(index)
                .map_err(|error| AtlasError::Db(error.to_string()))?
            {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(value) => Value::Int(value),
                ValueRef::Real(value) => Value::Float(value),
                ValueRef::Text(bytes) => Value::Str(String::from_utf8_lossy(bytes).into_owned()),
                ValueRef::Blob(_) => continue,
            };
            frontmatter.set(column.clone(), value);
        }
        frontmatter.set(
            "relations",
            Value::List(
                relations
                    .into_iter()
                    .map(|relation| {
                        Value::Map(vec![
                            ("relation".to_owned(), Value::Str(relation.relation)),
                            ("target".to_owned(), Value::Str(relation.target)),
                            ("confidence".to_owned(), Value::Float(relation.confidence)),
                        ])
                    })
                    .collect(),
            ),
        );
        let record = MarkdownRecord::new(
            frontmatter,
            render_section("Description", &text(row, "description")?),
        );
        let bytes = record
            .to_bytes(&relative)
            .map_err(|error| AtlasError::Other(format!("serialize memoir concept: {error}")))?;
        if write_if_changed(store, &relative, &bytes)? {
            written += 1;
        }
    }
    Ok(written)
}

struct ConceptRelation {
    target: String,
    relation: String,
    confidence: f64,
}

fn concept_relations(conn: &Connection, concept_id: &str) -> Result<Vec<ConceptRelation>> {
    let mut stmt = conn
        .prepare(
            "SELECT target.name, r.relation_type, r.confidence
             FROM memoir_relations r
             JOIN memoir_concepts target ON target.id = r.target_concept_id
             WHERE r.source_concept_id = ?1
             ORDER BY target.name, r.relation_type",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([concept_id], |row| {
            Ok(ConceptRelation {
                target: row.get(0)?,
                relation: row.get(1)?,
                confidence: row.get(2)?,
            })
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(rows)
}

/// Remove a memoir graph directory by graph name.
pub(super) fn delete_memoir_graph_dir(store: &MarkdownStore, graph_name: &str) -> Result<()> {
    let dir = store.absolute(&Path::new("memoir").join(slugify(graph_name)));
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AtlasError::Other(format!(
            "remove memoir directory {}: {error}",
            dir.display()
        ))),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn delete_file(store: &MarkdownStore, relative: &Path) -> Result<()> {
    store
        .delete_record(relative)
        .map(|_| ())
        .map_err(|error| AtlasError::Other(format!("delete markdown: {error}")))
}

/// Keep record ids filesystem-safe without changing ordinary hex/uuid ids.
fn safe_id(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "untitled".to_owned();
    }
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_owned();
    if out.is_empty() {
        "untitled".to_owned()
    } else {
        out
    }
}

/// Remove markdown files whose record id no longer exists in the derived
/// table. Used after prune/delete paths that can remove many rows at once.
pub(super) fn prune_orphans(
    store: &MarkdownStore,
    conn: &Connection,
    table: &str,
    id_column: &str,
    dir: &str,
    file_id_field: &str,
) -> Result<usize> {
    if !table_exists(conn, table) {
        return Ok(0);
    }
    let mut stmt = conn
        .prepare(&format!("SELECT {id_column} FROM {table}"))
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let mut removed = 0usize;
    for relative in store
        .list_markdown(Path::new(dir))
        .map_err(|error| AtlasError::Other(format!("list {dir} markdown: {error}")))?
    {
        let Ok(frontmatter) = store.read_frontmatter(&relative) else {
            continue;
        };
        let Some(id) = frontmatter.get_str(file_id_field) else {
            continue;
        };
        if !ids.contains(id) {
            delete_file(store, &relative)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Malformed frontmatter in durable record families, reported per file path so
/// `db_check` surfaces the exact file without failing the whole check.
pub(super) fn integrity_issues(store: &MarkdownStore) -> Vec<String> {
    let mut issues = Vec::new();
    for dir in [
        "decisions",
        "feedback",
        "tasks",
        "experience",
        "overview",
        "memoir",
    ] {
        let listed = match store.list_markdown(Path::new(dir)) {
            Ok(listed) => listed,
            Err(error) => {
                issues.push(format!("{dir}: {error}"));
                continue;
            }
        };
        for relative in listed {
            if let Err(error) = store.read_frontmatter(&relative) {
                issues.push(format!("{}: {error}", relative.display()));
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        DurableTaskStatus, MemoryScope, NewDurableTask, NewExperienceEdge, NewFeedback,
        NewMemoirConcept, NewMemoirGraph, NewMemoirRelation, NewOverviewRecord, NewSessionEvent,
        OverviewFreshness, OverviewSourceKind, OverviewSubject, OverviewSubjectKind,
        SessionEventType, SessionId, SessionStore,
    };
    use tempfile::TempDir;

    fn open_store() -> (TempDir, SessionStore) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".atlas").join("session.db");
        let store = SessionStore::open(path.to_str().unwrap()).unwrap();
        (dir, store)
    }

    fn read(store: &SessionStore, relative: &str) -> MarkdownRecord {
        store
            .markdown
            .read_record(Path::new(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"))
    }

    #[test]
    fn feedback_writes_predicted_actual_correction_sections() {
        let (_dir, mut store) = open_store();
        let record = store
            .store_feedback(&NewFeedback {
                repo_root: "/repo".to_owned(),
                session_id: None,
                tool_name: "cli".to_owned(),
                analysis_kind: "dead_code".to_owned(),
                predicted: "helper is dead".to_owned(),
                actual: "helper is used".to_owned(),
                correction: "keep helper".to_owned(),
                related_symbol: None,
                related_file: None,
                source_id: None,
                metadata: serde_json::json!({}),
            })
            .unwrap();
        let file = read(&store, &format!("feedback/{}.md", record.id));
        assert_eq!(file.frontmatter.get_str("tool_name"), Some("cli"));
        assert!(file.body.contains("## Predicted"));
        assert!(file.body.contains("## Actual"));
        assert!(file.body.contains("## Correction"));
    }

    #[test]
    fn durable_task_writes_checkbox_body_and_delete_removes_file() {
        let (_dir, mut store) = open_store();
        let task = store
            .create_durable_task(&NewDurableTask {
                task_id: "task-md".to_owned(),
                originating_method: "tools/call".to_owned(),
                request_id: None,
                tool_name: Some("doctor".to_owned()),
                transport_kind: Some("stdio".to_owned()),
                session_id: None,
                status: DurableTaskStatus::Completed,
                status_message: Some("done".to_owned()),
                ttl_ms: None,
            })
            .unwrap();
        let file = read(&store, &format!("tasks/{}.md", task.task_id));
        assert!(file.body.starts_with("- [x] completed"));
        assert!(store.delete_durable_task("task-md").unwrap());
        assert!(
            !store
                .markdown
                .absolute(Path::new("tasks/task-md.md"))
                .exists()
        );
    }

    #[test]
    fn memoir_writes_graph_concept_and_relation_frontmatter() {
        let (_dir, mut store) = open_store();
        store
            .create_memoir_graph(&NewMemoirGraph {
                repo_root: "/repo".to_owned(),
                scope: MemoryScope::Project,
                name: "md-graph".to_owned(),
                description: "graph description".to_owned(),
                metadata: serde_json::json!({}),
            })
            .unwrap();
        for name in ["alpha", "beta"] {
            store
                .add_memoir_concept(&NewMemoirConcept {
                    repo_root: "/repo".to_owned(),
                    scope: MemoryScope::Project,
                    graph: "md-graph".to_owned(),
                    name: name.to_owned(),
                    description: format!("{name} description"),
                    kind: "concept".to_owned(),
                    source_id: None,
                    metadata: serde_json::json!({}),
                })
                .unwrap();
        }
        store
            .link_memoir_concepts(&NewMemoirRelation {
                repo_root: "/repo".to_owned(),
                scope: MemoryScope::Project,
                graph: "md-graph".to_owned(),
                source: "alpha".to_owned(),
                target: "beta".to_owned(),
                relation: "depends_on".to_owned(),
                confidence: 0.8,
                source_id: None,
                metadata: serde_json::json!({}),
                create_missing: false,
            })
            .unwrap();

        assert!(
            store
                .markdown
                .absolute(Path::new("memoir/md-graph/_graph.md"))
                .exists()
        );
        let concept = read(&store, "memoir/md-graph/alpha.md");
        let relations = concept.frontmatter.get_list("relations").unwrap();
        assert_eq!(relations.len(), 1);
        let relation = relations[0].as_map().unwrap();
        assert_eq!(
            relation
                .iter()
                .find(|(key, _)| key == "target")
                .and_then(|(_, value)| value.as_str()),
            Some("beta")
        );
    }

    #[test]
    fn experience_and_decision_records_write_markdown() {
        let (_dir, mut store) = open_store();
        store
            .record_experience_edge(&NewExperienceEdge {
                repo_root: "/repo".to_owned(),
                source_uri: "atlas://symbol/a".to_owned(),
                target_uri: "atlas://file/b.rs".to_owned(),
                query_text: "find a".to_owned(),
                trace_summary: "a depends on b".to_owned(),
                commit_sha: "abc".to_owned(),
                source_file_hash: None,
                target_file_hash: None,
                confidence: 0.9,
                query_embedding: None,
            })
            .unwrap();
        let experience = std::fs::read_dir(store.markdown.absolute(Path::new("experience")))
            .unwrap()
            .count();
        assert_eq!(experience, 1);

        let session = SessionId::derive("/repo", "main", "cli");
        store
            .upsert_session_meta(session.clone(), "/repo", "cli", None)
            .unwrap();
        store
            .append_event(NewSessionEvent {
                session_id: session,
                event_type: SessionEventType::Decision,
                priority: 50,
                payload: serde_json::json!({
                    "decision": "markdown truth",
                    "summary": "markdown truth",
                    "rationale": "version control",
                    "conclusion": "adopt"
                }),
                created_at: None,
            })
            .unwrap();
        let decisions = std::fs::read_dir(store.markdown.absolute(Path::new("decisions")))
            .unwrap()
            .count();
        assert_eq!(decisions, 1);
    }

    #[test]
    fn overview_record_writes_level_sections() {
        let (_dir, mut store) = open_store();
        let subject = OverviewSubject {
            kind: OverviewSubjectKind::File,
            repo_root: "/repo".to_owned(),
            commit_sha: "abc".to_owned(),
            package_name: None,
            module_path: None,
            canonical_file_path: Some("src/lib.rs".to_owned()),
            qualified_name: None,
            node_kind: None,
            line_start: None,
            line_end: None,
            content_hash: None,
        };
        let record = store
            .insert_overview_record(&NewOverviewRecord {
                subject,
                title: "lib".to_owned(),
                summary: "one sentence".to_owned(),
                description_preview: "overview body".to_owned(),
                responsibilities: vec!["parse".to_owned()],
                flow: Vec::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                invariants: Vec::new(),
                gotchas: Vec::new(),
                examples: Vec::new(),
                tags: Vec::new(),
                source_kind: OverviewSourceKind::Manual,
                analyzer_name: None,
                analyzer_version: None,
                model_name: None,
                graph_last_indexed_at: None,
                freshness: Some(OverviewFreshness::Fresh),
                confidence: 0.9,
                source_id: None,
                supersedes_id: None,
                metadata: serde_json::json!({}),
            })
            .unwrap();
        let relative = format!("overview/file/{}.md", record.subject_id);
        let file = read(&store, &relative);
        assert!(file.body.contains("## Abstract (L0)"));
        assert!(file.body.contains("## Overview (L1)"));
        assert!(file.body.contains("one sentence"));
    }
}
