use tempfile::NamedTempFile;

use super::util::{is_corruption_error, rrf_merge};
use super::*;

// Compile-time enforcement: `ContentStore` must not implement `Send` or `Sync`.
//
// `ContentStore` carries `PhantomData<*const ()>` which explicitly opts it out
// of `Send` and `Sync` auto-traits, enforcing thread confinement at the
// compiler level regardless of what `rusqlite::Connection` implements.
static_assertions::assert_not_impl_any!(ContentStore: Send);
static_assertions::assert_not_impl_any!(ContentStore: Sync);

fn open_store() -> ContentStore {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    std::mem::forget(file);
    let mut store = ContentStore::open(&path).unwrap();
    store.migrate().unwrap();
    store
}

fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA table_info('{table}')");
    let mut stmt = conn.prepare(&sql).unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

fn meta(id: &str) -> SourceMeta {
    SourceMeta {
        id: id.to_string(),
        session_id: Some("sess1".into()),
        agent_id: Some("agent-alpha".into()),
        source_type: "review_context".into(),
        label: "test artifact".into(),
        repo_root: Some("/repo".into()),
        repo_roots: vec!["/repo".into()],
        repo_id: None,
        repo_ids: vec![],
        identity_kind: "artifact_label".into(),
        identity_value: "test artifact".into(),
    }
}

mod chunking;
mod embeddings;
mod indexing;
mod lifecycle;
mod scope;
mod search;
