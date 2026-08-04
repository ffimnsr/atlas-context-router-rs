use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use rusqlite::params;
use serde_json::Value;
use tempfile::TempDir;

use crate::SessionId;

use super::*;

// Compile-time enforcement: `SessionStore` must not implement `Send` or `Sync`.
//
// `SessionStore` carries `PhantomData<*const ()>` which explicitly opts it out
// of `Send` and `Sync` auto-traits, enforcing thread confinement at the
// compiler level regardless of what `rusqlite::Connection` implements.
static_assertions::assert_not_impl_any!(SessionStore: Send);
static_assertions::assert_not_impl_any!(SessionStore: Sync);

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

fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA table_info('{table}')");
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

// ── ICM-A2 — memory CRUD storage layer ────────────────────────────────────────

fn cli_viewer() -> MemoryViewer {
    MemoryViewer {
        frontend: "cli".to_owned(),
        session_id: "s1".to_owned(),
    }
}

fn seed_memory(
    store: &SessionStore,
    id: &str,
    now: &str,
    body: &str,
    topic: &str,
    importance: MemoryImportance,
    scope: MemoryScope,
) -> MemoryRecord {
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        session_id: (scope == MemoryScope::Session).then(|| "s1".to_owned()),
        frontend: (scope == MemoryScope::Frontend).then(|| "codex".to_owned()),
        scope,
        topic: topic.to_owned(),
        title: format!("title-{topic}"),
        body: body.to_owned(),
        importance,
        source_id: Some(format!("src-{id}")),
        metadata: serde_json::json!({ "seed": id }),
    };
    super::memory::store_memory_at(&store.conn, &input, now, id).unwrap()
}

mod compaction;
mod concurrency;
mod durable;
mod memory_query;
mod memory_recall;
mod memory_schema;
mod memory_store;
mod quarantine;
mod resume;
mod session;
mod snapshots;
