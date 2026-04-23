use serde::{Deserialize, Serialize};

/// Below this → raw pass-through, no indexing.
const DEFAULT_SMALL_OUTPUT_BYTES: usize = 512;
/// Below this → index + return preview; above → pointer only.
const DEFAULT_PREVIEW_THRESHOLD_BYTES: usize = 4096;
/// Minimum FTS result count before trigram fallback is attempted.
const DEFAULT_FALLBACK_MIN_RESULTS: usize = 3;

/// Metadata describing an artifact being stored.
#[derive(Debug, Clone)]
pub struct SourceMeta {
    /// Caller-assigned stable id derived from a structured identity seed.
    /// File-backed seeds must use canonical repo-path identity before hashing.
    pub id: String,
    /// Session this artifact belongs to (optional).
    pub session_id: Option<String>,
    /// Category label: `"review_context"`, `"impact_result"`, `"command_output"`, etc.
    pub source_type: String,
    /// Human-readable label for display and retrieval.
    pub label: String,
    /// Repo root at time of indexing (optional, for scoped queries).
    pub repo_root: Option<String>,
    /// Identity kind used to derive `id`.
    pub identity_kind: String,
    /// Canonical identity payload used to derive `id`.
    /// For `identity_kind=repo_path`, this must already be the
    /// `CanonicalRepoPath` string form.
    pub identity_value: String,
}

/// Filters for content search.
#[derive(Debug, Default, Clone)]
pub struct SearchFilters {
    pub session_id: Option<String>,
    pub source_type: Option<String>,
    pub repo_root: Option<String>,
}

/// A single chunk result returned from search.
#[derive(Debug, Clone)]
pub struct ChunkResult {
    pub source_id: String,
    /// Stable content-derived identity for this chunk (SHA-256 hex).
    /// Because `chunk_id` is seeded from `source_id`, canonical file-backed
    /// chunk identity depends on canonical file-backed `source_id`.
    pub chunk_id: String,
    pub chunk_index: usize,
    pub title: Option<String>,
    pub content: String,
    pub content_type: String,
}

/// Routing decision for an artifact based on size.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputRouting {
    /// Small enough to return directly; not indexed.
    Raw(String),
    /// Indexed; preview (first N chars) returned inline.
    Preview { source_id: String, preview: String },
    /// Indexed; only a pointer (source_id) returned.
    Pointer { source_id: String },
}

/// Configurable size thresholds for compression routing.
#[derive(Debug, Clone)]
pub struct ContentStoreConfig {
    /// Outputs at or below this size are returned raw without indexing.
    pub small_output_bytes: usize,
    /// Outputs above this size return only a pointer (source_id) rather than a preview.
    pub preview_threshold_bytes: usize,
    /// Minimum number of FTS hits before `search_with_fallback` skips trigram search.
    pub fallback_min_results: usize,
    /// When set, oldest sources are pruned after each index operation to keep the
    /// content database below this approximate byte limit. `None` disables enforcement.
    pub max_db_bytes: Option<u64>,
}

impl Default for ContentStoreConfig {
    fn default() -> Self {
        Self {
            small_output_bytes: DEFAULT_SMALL_OUTPUT_BYTES,
            preview_threshold_bytes: DEFAULT_PREVIEW_THRESHOLD_BYTES,
            fallback_min_results: DEFAULT_FALLBACK_MIN_RESULTS,
            max_db_bytes: None,
        }
    }
}

/// In-process counters tracking how `route_output` has dispatched artifacts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutingStats {
    /// Number of outputs returned raw (not indexed).
    pub raw_count: u64,
    /// Number of outputs indexed and returned with a preview.
    pub preview_count: u64,
    /// Number of outputs indexed and returned as a pointer only.
    pub pointer_count: u64,
    /// Total bytes of output that were routed as preview or pointer.
    pub avoided_bytes: u64,
}

/// Retrieved source row from the store.
#[derive(Debug, Clone)]
pub struct SourceRow {
    pub id: String,
    pub session_id: Option<String>,
    pub source_type: String,
    pub label: String,
    pub repo_root: Option<String>,
    pub identity_kind: String,
    pub identity_value: String,
    pub created_at: String,
}

/// Lifecycle phase of the retrieval/content index for a given repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    Indexing,
    Indexed,
    IndexFailed,
}

impl IndexState {
    pub(super) fn from_str(s: &str) -> Self {
        match s {
            "indexing" => Self::Indexing,
            "index_failed" => Self::IndexFailed,
            _ => Self::Indexed,
        }
    }
}

/// Persisted status row for the retrieval index of one repo root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalIndexStatus {
    pub repo_root: String,
    pub state: IndexState,
    pub files_discovered: i64,
    pub files_indexed: i64,
    pub chunks_written: i64,
    pub chunks_reused: i64,
    pub last_indexed_at: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

/// Progress counters passed to [`ContentStore::finish_indexing`].
#[derive(Debug, Clone, Default)]
pub struct IndexingStats {
    pub files_indexed: i64,
    pub chunks_written: i64,
    pub chunks_reused: i64,
}
