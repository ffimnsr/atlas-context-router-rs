//! Shared embedding generation for retrieval chunks.
//!
//! Both `atlas embed` and the MCP `embed_chunks` tool call this module so the
//! provider resolution, dimension freeze, and error accounting stay in one
//! place. Embeddings remain optional: no `[search.embedding] url` means the
//! command fails with next-step guidance instead of touching the graph.

use anyhow::{Context, Result};
use atlas_contentstore::ContentStore;
use atlas_contentstore::store::registry::provider_name_from_url;
use atlas_search::embed::{EmbeddingConfig, embed_text_blocking, ensure_model_available_blocking};
use atlas_store_sqlite::Store;
use serde::Serialize;

use crate::{Config, paths};

/// Default number of chunks embedded per run.
pub const DEFAULT_EMBED_CHUNK_LIMIT: usize = 1_000;

/// Options for one embedding run.
#[derive(Debug, Clone, Copy)]
pub struct EmbedChunkOptions {
    /// Maximum number of chunks to process in this run.
    pub limit: usize,
    /// Count pending chunks without calling the provider or writing vectors.
    pub dry_run: bool,
}

impl Default for EmbedChunkOptions {
    fn default() -> Self {
        Self {
            limit: DEFAULT_EMBED_CHUNK_LIMIT,
            dry_run: false,
        }
    }
}

/// Result of one embedding run.
#[derive(Debug, Clone, Serialize)]
pub struct EmbedChunkReport {
    /// Provider inferred from the configured backend URL.
    pub provider: String,
    /// Model sent to the embedding backend.
    pub model: String,
    /// Number of chunks selected for this run (capped by `options.limit`).
    pub pending_chunks: usize,
    /// Number of chunks whose vectors were persisted.
    pub embedded_chunks: usize,
    /// Number of chunks that failed to embed or persist.
    pub failed_chunks: usize,
    /// Frozen vector dimension observed in this run, when at least one
    /// embedding succeeded.
    pub dimension: Option<u32>,
    /// Whether Ollama reported the model missing and a pull was performed.
    pub model_pulled: bool,
    /// Whether this was a dry run.
    pub dry_run: bool,
}

/// Embed retrieval chunks that do not yet have a vector.
///
/// The configured `[search.embedding]` backend is required for real runs;
/// dry runs only count pending chunks. Dimension conflicts abort the run
/// before any new vectors are written, matching the CLI contract.
pub fn embed_pending_chunks(
    repo_root: &str,
    db_path: &str,
    options: EmbedChunkOptions,
) -> Result<EmbedChunkReport> {
    let config = Config::load(&paths::atlas_dir(repo_root)).unwrap_or_default();
    let backend = config.embedding_backend()?;
    if backend.is_none() && !options.dry_run {
        anyhow::bail!(
            "search.embedding.url not set in .atlas/config.toml — cannot generate embeddings"
        );
    }
    let model = backend
        .as_ref()
        .map(|backend| backend.model.clone())
        .unwrap_or_else(|| config.search.embedding.model.clone());
    let provider = backend
        .as_ref()
        .map(|backend| provider_name_from_url(&backend.url).to_owned())
        .unwrap_or_else(|| "unconfigured".to_owned());

    let store =
        Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let chunks = store
        .chunks_missing_embeddings(options.limit.max(1))
        .context("failed to read retrieval chunks")?;
    let pending_chunks = chunks.len();

    let mut report = EmbedChunkReport {
        provider,
        model,
        pending_chunks,
        embedded_chunks: 0,
        failed_chunks: 0,
        dimension: None,
        model_pulled: false,
        dry_run: options.dry_run,
    };

    if options.dry_run || chunks.is_empty() {
        return Ok(report);
    }

    let backend = backend.context(
        "search.embedding.url not set in .atlas/config.toml — cannot generate embeddings",
    )?;
    let embed_cfg = EmbeddingConfig::new(
        backend.url.clone(),
        backend.model.clone(),
        backend.timeout_secs,
        backend.max_retries,
        backend.retry_backoff_ms,
    );
    let content_db_path = paths::content_db_path(db_path);
    let mut content_store = ContentStore::open(&content_db_path)
        .with_context(|| format!("cannot open content store at {content_db_path}"))?;
    content_store
        .warm_dimension_cache()
        .context("failed to warm embedding dimension cache")?;

    report.model_pulled = ensure_model_available_blocking(&embed_cfg).with_context(|| {
        format!(
            "cannot prepare embedding model {} at {}",
            report.model, backend.url
        )
    })?;

    for (id, qn, text) in chunks {
        match embed_text_blocking(&embed_cfg, &text) {
            Ok(vector) => {
                let dimension = vector.len() as u32;
                if report.dimension.is_none() {
                    content_store
                        .check_embedding_dimension(&report.provider, &report.model, dimension)
                        .map_err(|error| {
                            anyhow::anyhow!(
                                "Aborting embed run: {error}\n\
                                 Run `atlas embed` again after rebuilding embeddings from scratch, \
                                 or switch back to the original provider/model."
                            )
                        })?;
                    report.dimension = Some(dimension);
                }
                match store.set_chunk_embedding(id, &vector) {
                    Ok(()) => report.embedded_chunks += 1,
                    Err(error) => {
                        tracing::warn!("store embedding failed for {qn}: {error:#}");
                        report.failed_chunks += 1;
                    }
                }
            }
            Err(error) => {
                tracing::warn!("embed failed for {qn}: {error:#}");
                report.failed_chunks += 1;
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_backend_returns_actionable_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("worldtree.db");
        let db_path = db_path.to_string_lossy().to_string();
        let repo_root = dir.path().to_string_lossy().to_string();

        let error = embed_pending_chunks(&repo_root, &db_path, EmbedChunkOptions::default())
            .expect_err("missing backend must fail");
        let message = error.to_string();

        assert!(
            message.contains("search.embedding.url"),
            "error must name the config key; got: {message}"
        );
    }

    #[test]
    fn dry_run_does_not_require_configured_backend() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("worldtree.db");
        let db_path = db_path.to_string_lossy().to_string();
        let repo_root = dir.path().to_string_lossy().to_string();

        let report = embed_pending_chunks(
            &repo_root,
            &db_path,
            EmbedChunkOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .expect("dry run");

        assert!(report.dry_run);
        assert_eq!(report.pending_chunks, 0);
        assert_eq!(report.embedded_chunks, 0);
        assert_eq!(report.provider, "unconfigured");
        assert_eq!(report.model, crate::config::DEFAULT_EMBED_MODEL);
    }
}
