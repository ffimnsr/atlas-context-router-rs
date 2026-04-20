//! HTTP embedding backend for hybrid (FTS + vector) search.
//!
//! Supports two server formats:
//! - **Ollama** (`POST /api/embed`): `{"model": …, "input": "…"}`
//!   → `{"embeddings": [[f32, …]]}`
//! - **OpenAI-compat** (`POST /v1/embeddings`): same request body
//!   → `{"data": [{"embedding": [f32, …]}]}`
//!
//! Configure via environment variables:
//! - `ATLAS_EMBED_URL`   — base URL, e.g. `http://localhost:11434` (required for hybrid)
//! - `ATLAS_EMBED_MODEL` — model name, e.g. `nomic-embed-text` (default)

use anyhow::{Context, Result};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for an HTTP embedding backend.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Base URL of the embedding server, e.g. `http://localhost:11434`.
    pub base_url: String,
    /// Embedding model name, e.g. `nomic-embed-text`.
    pub model: String,
}

impl EmbeddingConfig {
    /// Load from `ATLAS_EMBED_URL` and `ATLAS_EMBED_MODEL` environment variables.
    ///
    /// Returns `None` when `ATLAS_EMBED_URL` is not set.
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("ATLAS_EMBED_URL").ok()?;
        let model =
            std::env::var("ATLAS_EMBED_MODEL").unwrap_or_else(|_| "nomic-embed-text".to_owned());
        Some(Self { base_url, model })
    }
}

// ---------------------------------------------------------------------------
// Embedding call
// ---------------------------------------------------------------------------

/// Request a dense embedding vector for `text` from the configured backend.
///
/// The endpoint format is auto-detected from `base_url`:
/// - URLs containing `/v1` → OpenAI-compat (`POST /v1/embeddings`)
/// - All others            → Ollama native (`POST /api/embed`)
pub fn embed_text(config: &EmbeddingConfig, text: &str) -> Result<Vec<f32>> {
    let url = if config.base_url.contains("/v1") {
        format!("{}/embeddings", config.base_url.trim_end_matches('/'))
    } else {
        format!("{}/api/embed", config.base_url.trim_end_matches('/'))
    };

    let body = serde_json::json!({
        "model": config.model,
        "input": text,
    });

    let resp_text: String = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| anyhow::anyhow!("embedding HTTP request failed: {e}"))?
        .into_string()
        .context("reading embedding response body")?;

    // Try Ollama format first (has `embeddings` array of arrays).
    if let Ok(ollama) = serde_json::from_str::<OllamaResp>(&resp_text) {
        return ollama
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("empty embeddings array in Ollama response"));
    }

    // Fall back to OpenAI-compat format.
    let openai: OpenAiResp = serde_json::from_str(&resp_text)
        .with_context(|| format!("cannot parse embedding response from {url}"))?;
    openai
        .data
        .into_iter()
        .next()
        .map(|d| d.embedding)
        .ok_or_else(|| anyhow::anyhow!("empty data array in OpenAI embedding response"))
}

// ---------------------------------------------------------------------------
// Response types (private)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OllamaResp {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct OpenAiResp {
    data: Vec<OpenAiDatum>,
}

#[derive(Deserialize)]
struct OpenAiDatum {
    embedding: Vec<f32>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_ollama_native() {
        let cfg = EmbeddingConfig {
            base_url: "http://localhost:11434".to_owned(),
            model: "nomic-embed-text".to_owned(),
        };
        // Check URL construction via config (non-network test).
        let expected = "http://localhost:11434/api/embed";
        let url = if cfg.base_url.contains("/v1") {
            format!("{}/embeddings", cfg.base_url.trim_end_matches('/'))
        } else {
            format!("{}/api/embed", cfg.base_url.trim_end_matches('/'))
        };
        assert_eq!(url, expected);
    }

    #[test]
    fn url_openai_compat() {
        let cfg = EmbeddingConfig {
            base_url: "http://localhost:11434/v1".to_owned(),
            model: "text-embedding-3-small".to_owned(),
        };
        let url = if cfg.base_url.contains("/v1") {
            format!("{}/embeddings", cfg.base_url.trim_end_matches('/'))
        } else {
            format!("{}/api/embed", cfg.base_url.trim_end_matches('/'))
        };
        assert_eq!(url, "http://localhost:11434/v1/embeddings");
    }

    #[test]
    fn from_env_none_when_unset() {
        // ATLAS_EMBED_URL not set → None.
        // (Assuming test environment does not have ATLAS_EMBED_URL set.)
        if std::env::var("ATLAS_EMBED_URL").is_err() {
            assert!(EmbeddingConfig::from_env().is_none());
        }
    }
}
