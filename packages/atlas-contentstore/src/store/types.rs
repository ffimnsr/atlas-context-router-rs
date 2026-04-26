use serde::{Deserialize, Serialize};

/// Below this → raw pass-through, no indexing.
const DEFAULT_SMALL_OUTPUT_BYTES: usize = 512;
/// Below this → index + return preview; above → pointer only.
const DEFAULT_PREVIEW_THRESHOLD_BYTES: usize = 4096;
/// Minimum FTS result count before trigram fallback is attempted.
const DEFAULT_FALLBACK_MIN_RESULTS: usize = 3;
/// Default max chunks processed per retrieval/index flush batch.
const DEFAULT_RETRIEVAL_BATCH_SIZE: usize = 100;
/// Default max chunks sent to an embedding provider per batch.
const DEFAULT_EMBEDDING_BATCH_SIZE: usize = 32;
/// Hard cap on total chunks written across one indexing run.
const DEFAULT_MAX_CHUNKS_PER_INDEX_RUN: usize = 50_000;
/// Hard cap on chunks produced from a single file.
const DEFAULT_MAX_CHUNKS_PER_FILE: usize = 500;

/// Policy applied when an artifact or run exceeds a chunk cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OversizedPolicy {
    /// Return an error immediately; no chunks are written.
    FailFast,
    /// Truncate chunks to the cap, emit a `warn!` log, and continue.
    PartialWithWarning,
    /// Skip the file entirely with a `warn!` log; no chunks are written.
    SkipFile,
}

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
    /// Maximum chunks processed per flush batch during indexing.
    /// Controls transaction granularity and `batch_flush_count` tracking.
    pub retrieval_batch_size: usize,
    /// Maximum chunks sent to an embedding provider per batch call.
    pub embedding_batch_size: usize,
    /// Hard cap on total chunks written across one indexing run.
    /// Tracked cumulatively via `ContentStore::run_stats()`.
    pub max_chunks_per_index_run: usize,
    /// Hard cap on chunks produced from a single file/artifact.
    pub max_chunks_per_file: usize,
    /// Policy applied when a cap is exceeded.
    pub oversized_policy: OversizedPolicy,
}

impl Default for ContentStoreConfig {
    fn default() -> Self {
        Self {
            small_output_bytes: DEFAULT_SMALL_OUTPUT_BYTES,
            preview_threshold_bytes: DEFAULT_PREVIEW_THRESHOLD_BYTES,
            fallback_min_results: DEFAULT_FALLBACK_MIN_RESULTS,
            max_db_bytes: None,
            retrieval_batch_size: DEFAULT_RETRIEVAL_BATCH_SIZE,
            embedding_batch_size: DEFAULT_EMBEDDING_BATCH_SIZE,
            max_chunks_per_index_run: DEFAULT_MAX_CHUNKS_PER_INDEX_RUN,
            max_chunks_per_file: DEFAULT_MAX_CHUNKS_PER_FILE,
            oversized_policy: OversizedPolicy::PartialWithWarning,
        }
    }
}

/// In-process counters tracking chunk throughput for the current indexing run.
///
/// Reset by `ContentStore::reset_run_stats()` at the start of each run.
/// Read via `ContentStore::run_stats()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexRunStats {
    /// Total chunks buffered (queued for flush) since last reset.
    pub buffered_chunk_count: u64,
    /// Total bytes of chunk content buffered since last reset.
    pub buffered_bytes: u64,
    /// Total bytes of chunk content staged for vector/embedding indexing since last reset.
    pub staged_vector_bytes: u64,
    /// Number of batch flushes committed since last reset.
    pub batch_flush_count: u64,
    /// Cumulative chunks written to the store this run (used to enforce `max_chunks_per_index_run`).
    pub chunks_this_run: u64,
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

/// Progress counters passed to `ContentStore::finish_indexing()`.
#[derive(Debug, Clone, Default)]
pub struct IndexingStats {
    pub files_indexed: i64,
    pub chunks_written: i64,
    pub chunks_reused: i64,
}
