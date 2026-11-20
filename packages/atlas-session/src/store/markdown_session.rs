//! M3 — markdown session ledger.
//!
//! Truth files:
//!
//! - `.atlas/sessions/<id>/events.md` — append-only `###` sections, one per
//!   event, each with a fenced JSON payload and an `event_hash` dedup marker.
//! - `.atlas/sessions/<id>/resume.md` — frontmatter `event_count`, `consumed`,
//!   `updated_at` plus a fenced JSON snapshot body.
//!
//! SQLite keeps the session tables as derived indexes so existing list/stats
//! paths stay byte-stable. Every event append and compaction rewrites the
//! markdown ledger, so deleting the database never loses session history.

use std::path::{Path, PathBuf};

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::{Frontmatter, MarkdownRecord, MarkdownStore, Value};
use rusqlite::params;

use super::types::{SessionEventRow, SessionMeta};

fn session_dir(session_id: &str) -> PathBuf {
    Path::new("sessions").join(safe_id(session_id))
}

fn events_path(session_id: &str) -> PathBuf {
    session_dir(session_id).join("events.md")
}

fn resume_path(session_id: &str) -> PathBuf {
    session_dir(session_id).join("resume.md")
}

fn safe_id(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() {
        "session".to_owned()
    } else {
        out
    }
}

/// Longest run of backticks plus one, minimum four, so payloads containing
/// triple backticks can never terminate their own fence.
pub(super) fn fence_for(payload: &str) -> String {
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in payload.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    "`".repeat((longest + 1).max(4))
}

fn meta_frontmatter(meta: &SessionMeta) -> Frontmatter {
    let mut frontmatter = Frontmatter::new();
    frontmatter.set("session_id", Value::str(meta.session_id.as_str()));
    frontmatter.set("repo_root", Value::str(meta.repo_root.clone()));
    frontmatter.set("frontend", Value::str(meta.frontend.clone()));
    frontmatter.set(
        "worktree_id",
        meta.worktree_id.clone().map_or(Value::Null, Value::str),
    );
    frontmatter.set("created_at", Value::str(meta.created_at.clone()));
    frontmatter.set("updated_at", Value::str(meta.updated_at.clone()));
    frontmatter.set(
        "last_compaction_at",
        meta.last_compaction_at
            .clone()
            .map_or(Value::Null, Value::str),
    );
    frontmatter
}

fn event_section(event: &SessionEventRow, fence: &str) -> String {
    format!(
        "### {} {} {} {}\n\n{fence}json\n{}\n{fence}\n\n",
        event.created_at,
        event.event_type.as_str(),
        event.priority,
        event.event_hash,
        event.payload_json
    )
}

/// Append one event to the ledger. Duplicate `event_hash` markers are skipped
/// so two processes appending the same event cannot double-write it.
pub(super) fn append_event(
    store: &MarkdownStore,
    meta: &SessionMeta,
    event: &SessionEventRow,
) -> Result<()> {
    let relative = events_path(meta.session_id.as_str());
    store
        .with_writer_lock("sessions", || {
            let (mut frontmatter, mut body) = match store.read_record(&relative) {
                Ok(record) => (record.frontmatter, record.body),
                Err(atlas_store_markdown::MarkdownError::Io { .. }) => {
                    (meta_frontmatter(meta), String::new())
                }
                Err(error) => {
                    return Err(atlas_store_markdown::MarkdownError::InvalidValue {
                        field: "events".to_owned(),
                        message: error.to_string(),
                    });
                }
            };
            if body.contains(&format!(" {}", event.event_hash)) {
                return Ok(());
            }
            frontmatter.set("updated_at", Value::str(meta.updated_at.clone()));
            let fence = fence_for(&event.payload_json);
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(&event_section(event, &fence));
            let record = MarkdownRecord::new(frontmatter, body);
            store.write_record(&relative, &record)?;
            Ok(())
        })
        .map_err(|error| AtlasError::Other(format!("session ledger: {error}")))
}

/// Rewrite the whole ledger from the derived event table. Used by compaction
/// and by explicit repair paths.
pub(super) fn rewrite_session(
    store: &MarkdownStore,
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<()> {
    let meta = match store.session_meta(conn, session_id)? {
        Some(meta) => meta,
        None => return Ok(()),
    };
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, event_type, priority, payload_json, event_hash, created_at
             FROM session_events
             WHERE session_id = ?1
             ORDER BY created_at ASC, id ASC",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let events = stmt
        .query_map([session_id], super::util::row_to_event)
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let mut body = String::new();
    for event in &events {
        let fence = fence_for(&event.payload_json);
        body.push_str(&event_section(event, &fence));
    }
    let relative = events_path(session_id);
    store
        .with_writer_lock("sessions", || {
            let record = MarkdownRecord::new(meta_frontmatter(&meta), body);
            store.write_record(&relative, &record)
        })
        .map_err(|error| AtlasError::Other(format!("session ledger rewrite: {error}")))
}

/// Write or refresh the resume snapshot markdown file.
pub(super) fn write_resume(
    store: &MarkdownStore,
    session_id: &str,
    snapshot: &str,
    event_count: i64,
    consumed: bool,
    updated_at: &str,
) -> Result<()> {
    let relative = resume_path(session_id);
    let mut frontmatter = Frontmatter::new();
    frontmatter.set("session_id", Value::str(session_id));
    frontmatter.set("event_count", Value::Int(event_count));
    frontmatter.set("consumed", Value::Bool(consumed));
    frontmatter.set("updated_at", Value::str(updated_at));
    let fence = fence_for(snapshot);
    let body = format!("{fence}json\n{snapshot}\n{fence}\n");
    let record = MarkdownRecord::new(frontmatter, body);
    store
        .with_writer_lock("sessions", || store.write_record(&relative, &record))
        .map_err(|error| AtlasError::Other(format!("session resume: {error}")))
}

/// Remove a session directory.
pub(super) fn delete_session(store: &MarkdownStore, session_id: &str) -> Result<()> {
    let dir = store.absolute(&session_dir(session_id));
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AtlasError::Other(format!(
            "remove session directory {}: {error}",
            dir.display()
        ))),
    }
}

// A free function keeps `MarkdownStore` helpers accessible from the module
// without widening the public surface.
trait StoreSessionMeta {
    fn session_meta(
        &self,
        conn: &rusqlite::Connection,
        session_id: &str,
    ) -> Result<Option<SessionMeta>>;
}

impl StoreSessionMeta for MarkdownStore {
    fn session_meta(
        &self,
        conn: &rusqlite::Connection,
        session_id: &str,
    ) -> Result<Option<SessionMeta>> {
        let meta = conn
            .query_row(
                "SELECT session_id, repo_root, frontend, worktree_id, created_at, updated_at,
                        last_resume_at, last_compaction_at
                 FROM session_meta WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(SessionMeta {
                        session_id: super::SessionId(row.get(0)?),
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
            .ok();
        Ok(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewSessionEvent, SessionEventType, SessionId, SessionStore};
    use tempfile::TempDir;

    fn open_store() -> (TempDir, SessionStore, SessionId) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".atlas").join("session.db");
        let mut store = SessionStore::open(path.to_str().unwrap()).unwrap();
        let session = SessionId::derive("/repo", "main", "cli");
        store
            .upsert_session_meta(session.clone(), "/repo", "cli", None)
            .unwrap();
        (dir, store, session)
    }

    #[test]
    fn triple_backticks_get_a_longer_fence() {
        let (_dir, mut store, session) = open_store();
        store
            .append_event(NewSessionEvent {
                session_id: session.clone(),
                event_type: SessionEventType::ReasoningResult,
                priority: 10,
                payload: serde_json::json!({ "output": "code ``` inside" }),
                created_at: None,
            })
            .unwrap();

        let path = store.markdown.absolute(&events_path(session.as_str()));
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("````json"), "expected a four-backtick fence");
        assert!(text.contains("code ``` inside"));
        assert!(text.contains("###"));
    }

    #[test]
    fn duplicate_event_hash_appends_once() {
        let (_dir, store, session) = open_store();
        let meta = store
            .get_session_meta(&session)
            .unwrap()
            .expect("session meta");
        let event = SessionEventRow {
            id: 1,
            session_id: session.clone(),
            event_type: SessionEventType::CommandRun,
            priority: 10,
            payload_json: "{\"command\":\"ls\"}".to_owned(),
            event_hash: "abc123".to_owned(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        append_event(&store.markdown, &meta, &event).unwrap();
        append_event(&store.markdown, &meta, &event).unwrap();

        let text = std::fs::read_to_string(store.markdown.absolute(&events_path(session.as_str())))
            .unwrap();
        assert_eq!(text.matches("abc123").count(), 1);
    }

    #[test]
    fn resume_file_tracks_consumed_state() {
        let (_dir, mut store, session) = open_store();
        store
            .put_resume_snapshot(&session, "{\"summary\":\"s\"}", 3, false)
            .unwrap();
        let path = store.markdown.absolute(&resume_path(session.as_str()));
        let record = store
            .markdown
            .read_record(&resume_path(session.as_str()))
            .unwrap();
        assert_eq!(record.frontmatter.get_i64("event_count"), Some(3));
        assert_eq!(record.frontmatter.get_bool("consumed"), Some(false));
        assert!(path.exists());

        store.mark_resume_consumed(&session, true).unwrap();
        let record = store
            .markdown
            .read_record(&resume_path(session.as_str()))
            .unwrap();
        assert_eq!(record.frontmatter.get_bool("consumed"), Some(true));
    }

    #[test]
    fn compaction_rewrites_the_ledger_from_curated_events() {
        let (_dir, mut store, session) = open_store();
        for index in 0..4 {
            store
                .append_event(NewSessionEvent {
                    session_id: session.clone(),
                    event_type: SessionEventType::CommandRun,
                    priority: 10,
                    payload: serde_json::json!({ "command": format!("cmd {index}") }),
                    created_at: Some(format!("2026-01-01T00:00:0{index}Z")),
                })
                .unwrap();
        }
        store.compact_session(&session).unwrap();
        let events = store.list_events(&session).unwrap();
        let text = std::fs::read_to_string(store.markdown.absolute(&events_path(session.as_str())))
            .unwrap();
        assert_eq!(text.matches("### ").count(), events.len());
    }
}
