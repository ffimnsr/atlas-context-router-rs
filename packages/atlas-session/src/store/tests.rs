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
    let source_id = format!("src-{id}");
    seed_memory_full(
        store,
        id,
        now,
        body,
        topic,
        importance,
        scope,
        (scope == MemoryScope::Session).then_some("s1"),
        (scope == MemoryScope::Frontend).then_some("codex"),
        Some(&source_id),
    )
}

#[allow(clippy::too_many_arguments)]
fn seed_memory_full(
    store: &SessionStore,
    id: &str,
    now: &str,
    body: &str,
    topic: &str,
    importance: MemoryImportance,
    scope: MemoryScope,
    session_id: Option<&str>,
    frontend: Option<&str>,
    source_id: Option<&str>,
) -> MemoryRecord {
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        session_id: session_id.map(str::to_owned),
        frontend: frontend.map(str::to_owned),
        scope,
        topic: topic.to_owned(),
        title: format!("title-{topic}"),
        body: body.to_owned(),
        importance,
        source_id: source_id.map(str::to_owned),
        tags: Vec::new(),
        links: Vec::new(),
        metadata: serde_json::json!({ "seed": id }),
    };
    super::markdown_memory::store_memory_at(&store.markdown, &store.conn, &input, now, id).unwrap()
}

/// Rewrite one markdown record through the truth store.
fn update_memory(store: &SessionStore, id: &str, apply: impl FnOnce(&mut MemoryRecord)) {
    let mut record = super::markdown_memory::load_by_id(&store.markdown, "/repo", id)
        .unwrap()
        .expect("memory exists");
    apply(&mut record);
    super::markdown_memory::write_existing_record(&store.markdown, &store.conn, &record).unwrap();
}

/// Rewrite every memory record for the seeded repo.
fn rewrite_all_memories(store: &SessionStore, apply: impl Fn(&mut MemoryRecord)) {
    let records = super::markdown_memory::scan(&store.markdown, "/repo").unwrap();
    for mut record in records {
        apply(&mut record);
        super::markdown_memory::write_existing_record(&store.markdown, &store.conn, &record)
            .unwrap();
    }
}

/// Mark `id` as superseded by a `supersedes` link from `by`, creating the
/// marker record when a test only needs the effect.
fn mark_superseded(store: &SessionStore, id: &str, by: &str) {
    if super::markdown_memory::load_by_id(&store.markdown, "/repo", by)
        .unwrap()
        .is_none()
    {
        seed_memory_full(
            store,
            by,
            "2000-01-01T00:00:00Z",
            "supersession marker",
            "markers",
            MemoryImportance::Normal,
            MemoryScope::Project,
            None,
            None,
            None,
        );
    }
    update_memory(store, by, |record| {
        record.links.push(MemoryLink {
            id: id.to_owned(),
            relation: "supersedes".to_owned(),
        });
    });
}

mod compaction;
mod concurrency;
mod curation_compress;
mod durable;
mod feedback;
mod indexes;
mod markdown_import;
mod memoir;
mod memory_consolidate;
mod memory_curation;
mod memory_embeddings;
mod memory_markdown;
mod memory_ops;
mod memory_ops_operators;
mod memory_patch;
mod memory_query;
mod memory_recall;
mod memory_recall_quota;
mod memory_schema;
mod memory_store;
mod quarantine;
mod resume;
mod session;
mod snapshots;
