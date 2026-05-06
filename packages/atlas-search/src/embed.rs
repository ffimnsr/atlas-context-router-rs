//! HTTP embedding backend for hybrid (FTS + vector) search.
//!
//! Supports two server formats:
//! - **Ollama** (`POST /api/embed`): `{"model": …, "input": "…"}`
//!   → `{"embeddings": [[f32, …]]}`
//! - **OpenAI-compat** (`POST /v1/embeddings`): same request body
//!   → `{"data": [{"embedding": [f32, …]}]}`
//!
//! Configure via `.atlas/config.toml` `[search.embedding]` fields:
//! - `url`              — base URL, e.g. `http://localhost:11434` (required for hybrid)
//! - `model`            — model name, e.g. `nomic-embed-text` (default)
//! - `timeout_secs`     — per-request timeout in seconds (default 30)
//! - `max_retries`      — max retry attempts on transient errors (default 3)
//! - `retry_backoff_ms` — initial backoff between retries in ms, doubles each attempt (default 500)

use std::time::Duration;

use thiserror::Error;

/// Typed error for HTTP embedding backend operations.
#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("embedding HTTP request failed: {0}")]
    Http(String),
    #[error("empty response from embedding server")]
    EmptyResponse,
    #[error("failed to parse embedding response: {0}")]
    Parse(String),
    #[error("failed to build tokio runtime: {0}")]
    Runtime(String),
}

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
    /// Per-request HTTP timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum number of retry attempts on transient HTTP errors.
    pub max_retries: u32,
    /// Initial backoff between retries in milliseconds; doubles each attempt.
    pub retry_backoff_ms: u64,
    /// Pre-built HTTP client scoped to this config's timeout.
    client: reqwest::Client,
}

impl EmbeddingConfig {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        timeout_secs: u64,
        max_retries: u32,
        retry_backoff_ms: u64,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_default();

        Self {
            base_url: base_url.into(),
            model: model.into(),
            timeout_secs,
            max_retries,
            retry_backoff_ms,
            client,
        }
    }

    fn endpoint_url(&self) -> String {
        if self.base_url.contains("/v1") {
            format!("{}/embeddings", self.base_url.trim_end_matches('/'))
        } else {
            format!("{}/api/embed", self.base_url.trim_end_matches('/'))
        }
    }
}

// ---------------------------------------------------------------------------
// Async embedding call
// ---------------------------------------------------------------------------

/// Request a dense embedding vector for `text` from the configured backend.
///
/// The endpoint format is auto-detected from `base_url`:
/// - URLs containing `/v1` → OpenAI-compat (`POST /v1/embeddings`)
/// - All others            → Ollama native (`POST /api/embed`)
///
/// Retries up to `config.max_retries` times on transient HTTP errors with
/// exponential backoff starting at `config.retry_backoff_ms`.
pub async fn embed_text(config: &EmbeddingConfig, text: &str) -> Result<Vec<f32>, EmbedError> {
    let url = config.endpoint_url();
    let body = serde_json::json!({
        "model": config.model,
        "input": text,
    });

    let mut last_err = EmbedError::Http("no attempts made".to_owned());
    let mut backoff_ms = config.retry_backoff_ms;

    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = backoff_ms.saturating_mul(2);
        }

        let response = config
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        let resp = match response {
            Ok(r) => r,
            Err(e) => {
                last_err = EmbedError::Http(e.to_string());
                continue;
            }
        };

        if !resp.status().is_success() {
            last_err = EmbedError::Http(format!(
                "embedding server returned status {} for {}",
                resp.status(),
                url
            ));
            // Only retry on 429 / 5xx; bail immediately on 4xx client errors.
            if resp.status().is_client_error() && resp.status().as_u16() != 429 {
                return Err(last_err);
            }
            continue;
        }

        let resp_text = resp
            .text()
            .await
            .map_err(|e| EmbedError::Parse(format!("reading embedding response body: {e}")))?;

        // Try Ollama format first (has `embeddings` array of arrays).
        if let Ok(ollama) = serde_json::from_str::<OllamaResp>(&resp_text) {
            return ollama
                .embeddings
                .into_iter()
                .next()
                .ok_or(EmbedError::EmptyResponse);
        }

        // Fall back to OpenAI-compat format.
        let openai: OpenAiResp = serde_json::from_str(&resp_text).map_err(|e| {
            EmbedError::Parse(format!("cannot parse embedding response from {url}: {e}"))
        })?;
        return openai
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or(EmbedError::EmptyResponse);
    }

    Err(last_err)
}

// ---------------------------------------------------------------------------
// Sync bridge
// ---------------------------------------------------------------------------

/// Blocking wrapper around [`embed_text`].
///
/// - Inside an existing Tokio runtime (e.g. `spawn_blocking` tasks in the MCP
///   server): drives the future using the current runtime's handle so no extra
///   thread is spawned.
/// - Outside any runtime (e.g. CLI commands): creates a temporary
///   `current_thread` runtime for the duration of the call.
pub fn embed_text_blocking(config: &EmbeddingConfig, text: &str) -> Result<Vec<f32>, EmbedError> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(embed_text(config, text)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| EmbedError::Runtime(e.to_string()))?
            .block_on(embed_text(config, text)),
    }
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
        let cfg = EmbeddingConfig::new("http://localhost:11434", "nomic-embed-text", 30, 3, 500);
        assert_eq!(cfg.endpoint_url(), "http://localhost:11434/api/embed");
    }

    #[test]
    fn url_openai_compat() {
        let cfg = EmbeddingConfig::new(
            "http://localhost:11434/v1",
            "text-embedding-3-small",
            30,
            3,
            500,
        );
        assert_eq!(cfg.endpoint_url(), "http://localhost:11434/v1/embeddings");
    }

    #[test]
    fn new_preserves_explicit_settings() {
        let cfg = EmbeddingConfig::new("http://embed.test", "embed-model", 45, 5, 750);

        assert_eq!(cfg.base_url, "http://embed.test");
        assert_eq!(cfg.model, "embed-model");
        assert_eq!(cfg.timeout_secs, 45);
        assert_eq!(cfg.max_retries, 5);
        assert_eq!(cfg.retry_backoff_ms, 750);
    }

    #[test]
    fn new_builds_client_from_timeout() {
        let cfg = EmbeddingConfig::new("http://embed.test", "embed-model", 30, 3, 500);

        assert_eq!(cfg.timeout_secs, 30);
        let cloned = cfg.client.clone();
        drop(cloned);
    }
}
