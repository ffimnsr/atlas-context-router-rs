//! Retrieval backend capability model (Patch R4).
//!
//! [`BackendCapabilities`] describes what the currently configured retrieval
//! backend supports. Callers use [`derive_capabilities`] to build it from an
//! optional embedding config, then pass it to [`validate_mode_request`] before
//! executing a query to get a structured error instead of silent degradation.
//!
//! The existing fallback-with-warning behaviour in `search_with_embedding` is
//! preserved for callers that do not explicitly validate; this module adds a
//! formal validation path on top of that.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::embed::EmbeddingConfig;

// ---------------------------------------------------------------------------
// Capability flags
// ---------------------------------------------------------------------------

/// Capability flags for the active retrieval backend.
///
/// All flags default to `false`; [`derive_capabilities`] sets the appropriate
/// flags based on the runtime configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCapabilities {
    /// Lexical full-text search (SQLite FTS5) is available.
    ///
    /// This is `true` for every Atlas deployment backed by `worldtree.db`
    /// because SQLite FTS5 is always compiled in.
    pub lexical_fts: bool,

    /// Dense vector search is available.
    ///
    /// Requires a configured embedding backend (`search.embedding.url` in
    /// `.atlas/config.toml`).
    pub dense_vector: bool,

    /// Hybrid lexical + dense vector fusion (Reciprocal Rank Fusion) is
    /// available.
    ///
    /// Requires both `lexical_fts` and `dense_vector`.
    pub hybrid_lexical_vector: bool,

    /// Native sparse / BM25 retrieval index is available.
    ///
    /// SQLite FTS5 approximates BM25 but is not a dedicated sparse-retrieval
    /// engine; this flag is `false` until a true sparse index is added.
    pub sparse_bm25_native: bool,

    /// Metadata filtering (kind, language, subpath) is available.
    ///
    /// Always `true` for SQLite-backed stores that support `WHERE` clauses.
    pub metadata_filtering: bool,
}

impl BackendCapabilities {
    /// Return capabilities for the standard Atlas SQLite backend.
    ///
    /// Passes `embed_cfg` to determine whether dense-vector and hybrid modes
    /// are available.
    pub fn for_sqlite(embed_cfg: Option<&EmbeddingConfig>) -> Self {
        let has_vector = embed_cfg.is_some();
        Self {
            lexical_fts: true,
            dense_vector: has_vector,
            hybrid_lexical_vector: has_vector,
            sparse_bm25_native: false,
            metadata_filtering: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Mode validation
// ---------------------------------------------------------------------------

/// Reason a requested retrieval mode is not supported by the backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeUnsupportedReason {
    /// Dense vector search is required but not configured.
    DenseVectorUnavailable,
    /// Hybrid lexical + vector fusion is required but not available.
    HybridUnavailable,
    /// Lexical FTS is required but not available on this backend.
    LexicalFtsUnavailable,
    /// Native sparse / BM25 retrieval is required but not available.
    SparseBm25Unavailable,
}

impl ModeUnsupportedReason {
    fn message(&self) -> &'static str {
        match self {
            Self::DenseVectorUnavailable => {
                "dense vector search requested but search.embedding.url is not configured; \
                 add an embedding backend to enable vector retrieval"
            }
            Self::HybridUnavailable => {
                "hybrid retrieval requested but search.embedding.url is not configured; \
                 add an embedding backend or use lexical FTS instead"
            }
            Self::LexicalFtsUnavailable => {
                "lexical FTS search requested but the backend does not support it; \
                 use a SQLite-backed store or switch to a supported retrieval mode"
            }
            Self::SparseBm25Unavailable => {
                "sparse / BM25-native retrieval is not available; \
                 use lexical FTS or hybrid retrieval instead"
            }
        }
    }
}

/// Error returned when a requested retrieval mode is unsupported.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
#[error("retrieval mode '{requested}' unsupported: {message}")]
pub struct ModeValidationError {
    /// The mode or flag that was requested.
    pub requested: String,
    /// Machine-readable reason code.
    pub reason: ModeUnsupportedReason,
    /// Human-readable explanation.
    pub message: String,
}

/// Check whether the modes requested in `query` are all supported by `caps`.
///
/// Returns `Ok(())` when every requested mode has a matching capability flag.
/// Returns `Err(ModeValidationError)` with the first unsupported mode found.
///
/// This is a strict validation path.  The existing `search_with_embedding`
/// fallback-with-warning behaviour is preserved for callers that do not
/// explicitly validate.
pub fn validate_mode_request(
    hybrid: bool,
    dense_only: bool,
    sparse: bool,
    lexical: bool,
    caps: &BackendCapabilities,
) -> Result<(), ModeValidationError> {
    if hybrid && !caps.hybrid_lexical_vector {
        let reason = ModeUnsupportedReason::HybridUnavailable;
        let message = reason.message().to_owned();
        return Err(ModeValidationError {
            requested: "hybrid".to_owned(),
            reason,
            message,
        });
    }
    if dense_only && !caps.dense_vector {
        let reason = ModeUnsupportedReason::DenseVectorUnavailable;
        let message = reason.message().to_owned();
        return Err(ModeValidationError {
            requested: "dense_vector".to_owned(),
            reason,
            message,
        });
    }
    if sparse && !caps.sparse_bm25_native {
        let reason = ModeUnsupportedReason::SparseBm25Unavailable;
        let message = reason.message().to_owned();
        return Err(ModeValidationError {
            requested: "sparse_bm25".to_owned(),
            reason,
            message,
        });
    }
    if lexical && !caps.lexical_fts {
        let reason = ModeUnsupportedReason::LexicalFtsUnavailable;
        let message = reason.message().to_owned();
        return Err(ModeValidationError {
            requested: "lexical_fts".to_owned(),
            reason,
            message,
        });
    }
    Ok(())
}

/// Derive [`BackendCapabilities`] for a standard SQLite-backed Atlas store.
///
/// Pass `embed_cfg = Some(cfg)` when an embedding backend is configured to
/// enable `dense_vector` and `hybrid_lexical_vector` flags.
pub fn derive_capabilities(embed_cfg: Option<&EmbeddingConfig>) -> BackendCapabilities {
    BackendCapabilities::for_sqlite(embed_cfg)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::EmbeddingConfig;

    fn embed_cfg() -> EmbeddingConfig {
        EmbeddingConfig::new("http://localhost:11434", "nomic-embed-text", 30, 3, 500)
    }

    // ---- derive_capabilities -----------------------------------------------

    #[test]
    fn lexical_only_backend_has_fts_and_metadata() {
        let caps = derive_capabilities(None);
        assert!(
            caps.lexical_fts,
            "lexical FTS must be available without embedding backend"
        );
        assert!(
            !caps.dense_vector,
            "dense vector must be unavailable without embedding backend"
        );
        assert!(
            !caps.hybrid_lexical_vector,
            "hybrid must be unavailable without embedding backend"
        );
        assert!(!caps.sparse_bm25_native, "sparse BM25 is not implemented");
        assert!(
            caps.metadata_filtering,
            "metadata filtering is always available"
        );
    }

    #[test]
    fn hybrid_capable_backend_has_all_standard_flags() {
        let cfg = embed_cfg();
        let caps = derive_capabilities(Some(&cfg));
        assert!(caps.lexical_fts);
        assert!(caps.dense_vector);
        assert!(caps.hybrid_lexical_vector);
        assert!(!caps.sparse_bm25_native);
        assert!(caps.metadata_filtering);
    }

    #[test]
    fn dense_only_backend_rejects_lexical_fts_request() {
        // Simulate a hypothetical dense-only backend (no FTS support).
        let dense_only_caps = BackendCapabilities {
            lexical_fts: false,
            dense_vector: true,
            hybrid_lexical_vector: false,
            sparse_bm25_native: false,
            metadata_filtering: true,
        };
        let result = validate_mode_request(false, false, false, true, &dense_only_caps);
        assert!(
            result.is_err(),
            "lexical FTS request must fail on dense-only backend"
        );
        let err = result.unwrap_err();
        assert_eq!(err.reason, ModeUnsupportedReason::LexicalFtsUnavailable);
        assert!(err.message.contains("lexical FTS"));
    }

    // ---- validate_mode_request ---------------------------------------------

    #[test]
    fn hybrid_mode_rejected_when_embedding_not_configured() {
        let caps = derive_capabilities(None);
        let result = validate_mode_request(true, false, false, false, &caps);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.requested, "hybrid");
        assert_eq!(err.reason, ModeUnsupportedReason::HybridUnavailable);
        assert!(err.message.contains("embedding backend"));
    }

    #[test]
    fn hybrid_mode_accepted_when_embedding_configured() {
        let cfg = embed_cfg();
        let caps = derive_capabilities(Some(&cfg));
        assert!(validate_mode_request(true, false, false, false, &caps).is_ok());
    }

    #[test]
    fn dense_only_mode_rejected_without_vector_backend() {
        let caps = derive_capabilities(None);
        let result = validate_mode_request(false, true, false, false, &caps);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.reason, ModeUnsupportedReason::DenseVectorUnavailable);
    }

    #[test]
    fn dense_only_mode_accepted_with_vector_backend() {
        let cfg = embed_cfg();
        let caps = derive_capabilities(Some(&cfg));
        assert!(validate_mode_request(false, true, false, false, &caps).is_ok());
    }

    #[test]
    fn sparse_bm25_always_rejected_on_current_backend() {
        // Sparse BM25 is not implemented — always rejected.
        let caps_no_embed = derive_capabilities(None);
        let caps_with_embed = derive_capabilities(Some(&embed_cfg()));
        assert!(validate_mode_request(false, false, true, false, &caps_no_embed).is_err());
        assert!(validate_mode_request(false, false, true, false, &caps_with_embed).is_err());
    }

    #[test]
    fn unsupported_mode_error_is_descriptive() {
        let caps = derive_capabilities(None);
        let err = validate_mode_request(true, false, false, false, &caps).unwrap_err();
        // Error message must explain what to do.
        assert!(err.message.contains("embedding backend") || err.message.contains("configured"));
        // Display impl should include both requested mode and message.
        let display = err.to_string();
        assert!(display.contains("hybrid"));
    }

    #[test]
    fn no_special_modes_always_valid() {
        let caps = derive_capabilities(None);
        assert!(validate_mode_request(false, false, false, false, &caps).is_ok());
        assert!(validate_mode_request(false, false, false, true, &caps).is_ok());
    }

    #[test]
    fn serde_round_trip() {
        let cfg = embed_cfg();
        let caps = derive_capabilities(Some(&cfg));
        let json = serde_json::to_string(&caps).unwrap();
        let decoded: BackendCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(caps, decoded);
    }
}
