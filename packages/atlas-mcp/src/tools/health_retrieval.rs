//! Retrieval-index summary probe for `status`.
//!
//! Kept out of `health.rs` so the tool module stays below the repository's
//! 1,000-line modification threshold while sharing the same content-store
//! schema probe the CLI uses.

use serde::Serialize;

#[derive(Serialize)]
pub(super) struct RetrievalIndexSummary {
    pub(super) available: bool,
    pub(super) searchable: bool,
    pub(super) state: Option<String>,
    pub(super) files_discovered: i64,
    pub(super) files_indexed: i64,
    pub(super) chunks_written: i64,
    pub(super) chunks_reused: i64,
    pub(super) last_indexed_at: Option<String>,
    pub(super) last_error: Option<String>,
    pub(super) content_db_path: String,
    pub(super) content_schema_version: Option<i32>,
    pub(super) content_schema_supported: i32,
    pub(super) content_schema_ok: bool,
}

pub(super) fn collect_retrieval_index_summary(
    repo_root: &str,
    db_path: &str,
) -> RetrievalIndexSummary {
    use atlas_contentstore::{ContentStore, IndexState};

    let content_db_path = atlas_engine::paths::content_db_path(db_path);
    // Mirror the CLI status payload so schema skew is visible on both
    // surfaces instead of only in `atlas status`.
    let schema = atlas_engine::doctor::probe_content_schema(&content_db_path);
    match ContentStore::open(&content_db_path) {
        Ok(mut store) => {
            let _ = store.migrate();
            match store.get_index_status(repo_root) {
                Ok(Some(status)) => {
                    let searchable = status.state == IndexState::Indexed;
                    let state = match status.state {
                        IndexState::Indexed => "indexed",
                        IndexState::Indexing => "indexing",
                        IndexState::IndexFailed => "index_failed",
                    };
                    RetrievalIndexSummary {
                        available: true,
                        searchable,
                        state: Some(state.to_owned()),
                        files_discovered: status.files_discovered,
                        files_indexed: status.files_indexed,
                        chunks_written: status.chunks_written,
                        chunks_reused: status.chunks_reused,
                        last_indexed_at: status.last_indexed_at,
                        last_error: status.last_error,
                        content_db_path,
                        content_schema_version: schema.schema_version,
                        content_schema_supported: schema.supported_version,
                        content_schema_ok: schema.ok,
                    }
                }
                Ok(None) => RetrievalIndexSummary {
                    available: false,
                    searchable: false,
                    state: None,
                    files_discovered: 0,
                    files_indexed: 0,
                    chunks_written: 0,
                    chunks_reused: 0,
                    last_indexed_at: None,
                    last_error: Some(
                        "content store has no retrieval index state for this repo".to_owned(),
                    ),
                    content_db_path,
                    content_schema_version: schema.schema_version,
                    content_schema_supported: schema.supported_version,
                    content_schema_ok: schema.ok,
                },
                Err(error) => RetrievalIndexSummary {
                    available: false,
                    searchable: false,
                    state: None,
                    files_discovered: 0,
                    files_indexed: 0,
                    chunks_written: 0,
                    chunks_reused: 0,
                    last_indexed_at: None,
                    last_error: Some(error.to_string()),
                    content_db_path,
                    content_schema_version: schema.schema_version,
                    content_schema_supported: schema.supported_version,
                    content_schema_ok: schema.ok,
                },
            }
        }
        Err(error) => RetrievalIndexSummary {
            available: false,
            searchable: false,
            state: None,
            files_discovered: 0,
            files_indexed: 0,
            chunks_written: 0,
            chunks_reused: 0,
            last_indexed_at: None,
            last_error: Some(error.to_string()),
            content_db_path,
            content_schema_version: schema.schema_version,
            content_schema_supported: schema.supported_version,
            content_schema_ok: schema.ok,
        },
    }
}
