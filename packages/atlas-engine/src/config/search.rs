//! Search-phase configuration (hybrid retrieval + embedding backend).

use atlas_core::BudgetPolicy;
use serde::{Deserialize, Serialize};

/// Fastest CPU-friendly default embedding model.
///
/// `all-minilm` is the Ollama build of `sentence-transformers/all-MiniLM-L6-v2`:
/// ~23M parameters, 384 dimensions, and low latency on CPU-only hosts. Users
/// can override it with `[search.embedding] model = "..."`.
pub const DEFAULT_EMBED_MODEL: &str = "all-minilm";
pub const DEFAULT_EMBED_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_EMBED_MAX_RETRIES: u32 = 3;
pub const DEFAULT_EMBED_RETRY_BACKOFF_MS: u64 = 500;
/// Default chunk-count threshold for enabling the in-memory TrieHI index.
pub const DEFAULT_TRIEHI_MIN_CHUNKS: usize = 10_000;

/// Retrieval execution configuration (VR2 stage 2).
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RetrievalConfig {
    /// Minimum `retrieval_chunks` row count before the TrieHI in-memory
    /// directory index is built lazily for scoped vector queries.
    pub triehi_min_chunks: usize,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            triehi_min_chunks: DEFAULT_TRIEHI_MIN_CHUNKS,
        }
    }
}

impl RetrievalConfig {
    /// Validate the threshold; `0` would force a trie build on every query.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.triehi_min_chunks == 0 {
            anyhow::bail!("retrieval.triehi_min_chunks must be >= 1");
        }
        Ok(())
    }
}

/// Search-phase configuration.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Enable hybrid (FTS + vector) retrieval when an embedding backend is configured.
    /// Falls back to FTS-only when no backend is available regardless of this flag.
    pub hybrid_enabled: bool,
    /// FTS candidate pool size fetched before Reciprocal Rank Fusion merge.
    pub top_k_fts: usize,
    /// Vector candidate pool size fetched before Reciprocal Rank Fusion merge.
    pub top_k_vector: usize,
    /// RRF k constant (higher = less rank-position sensitivity, default 60).
    pub rrf_k: u32,
    /// Maximum seed candidates accepted before graph expansion or semantic rerank.
    pub max_query_candidates: usize,
    /// Maximum wall time for one query path before Atlas reports a budget hit.
    pub max_query_wall_time_ms: u64,
    /// HTTP embedding backend configuration used for hybrid retrieval.
    pub embedding: SearchEmbeddingConfig,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            hybrid_enabled: false,
            top_k_fts: 60,
            top_k_vector: 60,
            rrf_k: 60,
            max_query_candidates: BudgetPolicy::default()
                .query_candidates_and_seeds
                .candidates
                .default_limit,
            max_query_wall_time_ms: BudgetPolicy::default()
                .query_candidates_and_seeds
                .wall_time_ms
                .default_limit as u64,
            embedding: SearchEmbeddingConfig::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchEmbeddingConfig {
    /// Base URL for embedding requests. When unset, hybrid retrieval falls back to FTS.
    pub url: Option<String>,
    /// Embedding model name sent to the backend.
    pub model: String,
    /// Per-request timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum retry attempts on transient backend failures.
    pub max_retries: u32,
    /// Initial retry backoff in milliseconds.
    pub retry_backoff_ms: u64,
}

impl Default for SearchEmbeddingConfig {
    fn default() -> Self {
        Self {
            url: None,
            model: DEFAULT_EMBED_MODEL.to_owned(),
            timeout_secs: DEFAULT_EMBED_TIMEOUT_SECS,
            max_retries: DEFAULT_EMBED_MAX_RETRIES,
            retry_backoff_ms: DEFAULT_EMBED_RETRY_BACKOFF_MS,
        }
    }
}
