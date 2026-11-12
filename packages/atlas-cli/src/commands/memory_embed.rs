//! ICM-E2 — optional memory embeddings.
//!
//! Lexical recall never needs this module: it only resolves the opt-in
//! `[memory.embedding]` backend, embeds memory bodies or query text through the
//! shared `atlas-search` HTTP client, and hands vectors to the session store.
//! No embedding provider means no embeddings and no semantic path.

use anyhow::{Context, Result, bail};

use atlas_engine::EmbeddingBackendConfig;
use atlas_search::embed::{EmbeddingConfig, embed_text_blocking};
use atlas_session::{
    MemoryEmbeddingCoverage, MemoryEmbeddingPolicy, MemoryListFilter, MemoryRecord, SessionStore,
};

/// Build the `atlas-search` HTTP client for the configured memory backend.
pub(super) fn embedding_client(backend: &EmbeddingBackendConfig) -> EmbeddingConfig {
    EmbeddingConfig::new(
        backend.url.clone(),
        backend.model.clone(),
        backend.timeout_secs,
        backend.max_retries,
        backend.retry_backoff_ms,
    )
}

/// Resolve the configured memory embedding backend or fail with next steps.
pub(super) fn require_backend(config: &atlas_engine::Config) -> Result<EmbeddingBackendConfig> {
    match config.memory_embedding_backend()? {
        Some(backend) => Ok(backend),
        None => bail!(
            "memory.embedding.enabled is false; set [memory.embedding] enabled = true with provider = \"search\" and search.embedding.url to use semantic recall"
        ),
    }
}

/// Hybrid fusion weights mirrored from `[memory.search]` (ICM-E2).
pub(super) fn embedding_policy(config: &atlas_engine::Config) -> MemoryEmbeddingPolicy {
    MemoryEmbeddingPolicy {
        weight_fts: config.memory.search.hybrid_weight_fts,
        weight_vector: config.memory.search.hybrid_weight_vector,
        rrf_k: atlas_session::MEMORY_HYBRID_RRF_K,
        min_similarity: config.memory.search.min_vector_similarity,
    }
}

/// Embed one piece of text through the configured backend, enforcing dimension.
fn embed_one(backend: &EmbeddingBackendConfig, dimension: usize, text: &str) -> Result<Vec<f32>> {
    let client = embedding_client(backend);
    let vector = embed_text_blocking(&client, text)
        .with_context(|| format!("cannot embed text with backend {}", backend.url))?;
    if vector.len() != dimension {
        bail!(
            "memory.embedding.dimension is {dimension} but embedding provider returned {} values; update memory.embedding.dimension or the model",
            vector.len()
        );
    }
    Ok(vector)
}

/// Embed the recall query so hybrid fusion can score stored vectors.
pub(super) fn query_embedding(config: &atlas_engine::Config, query: &str) -> Result<Vec<f32>> {
    let backend = require_backend(config)?;
    embed_one(&backend, config.memory.embedding.dimension, query)
}

/// `atlas memory embed` — fill embedding gaps for recallable memories.
pub(super) fn run_embed(
    repo: &str,
    store: &mut SessionStore,
    config: &atlas_engine::Config,
    filter: &MemoryListFilter,
    limit: usize,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let backend = require_backend(config)?;
    let dimension = config.memory.embedding.dimension;
    let model = config.memory.embedding.model.trim().to_owned();

    let pending = store.memories_missing_embeddings(repo, &model, dimension, filter, limit)?;
    if dry_run {
        let coverage = store.memory_embedding_coverage(repo, &model, dimension)?;
        if json {
            super::print_json(
                "memory.embed",
                serde_json::json!({
                    "dry_run": true,
                    "embedding_model": model,
                    "dimension": dimension,
                    "pending": pending,
                    "pending_count": pending.len(),
                    "coverage": coverage_json(&coverage),
                }),
            )?;
        } else {
            println!(
                "Would embed {} memories with {model} ({dimension}d)",
                pending.len()
            );
            for memory in &pending {
                println!("  {} [{}/{}]", memory.id, memory.topic, memory.importance);
            }
        }
        return Ok(());
    }

    let mut embedded = 0_usize;
    let mut failures: Vec<serde_json::Value> = Vec::new();
    for memory in &pending {
        let text = embedding_text(memory);
        match embed_one(&backend, dimension, &text) {
            Ok(vector) => {
                store.store_memory_embedding(&memory.id, &model, dimension, &vector)?;
                embedded += 1;
            }
            Err(error) => failures.push(serde_json::json!({
                "memory_id": memory.id,
                "error": error.to_string(),
            })),
        }
    }

    let coverage = store.memory_embedding_coverage(repo, &model, dimension)?;
    if json {
        super::print_json(
            "memory.embed",
            serde_json::json!({
                "dry_run": false,
                "embedding_model": model,
                "dimension": dimension,
                "embedded": embedded,
                "failed": failures.len(),
                "failures": failures,
                "coverage": coverage_json(&coverage),
            }),
        )?;
    } else {
        println!("Embedded {embedded} memories with {model} ({dimension}d)");
        println!(
            "Coverage   : {}/{} embedded",
            coverage.embedded,
            coverage.embedded + coverage.missing
        );
        for failure in &failures {
            eprintln!(
                "failed {}: {}",
                failure["memory_id"].as_str().unwrap_or_default(),
                failure["error"].as_str().unwrap_or_default()
            );
        }
    }

    if !failures.is_empty() {
        bail!("{} memories could not be embedded", failures.len());
    }
    Ok(())
}

fn coverage_json(coverage: &MemoryEmbeddingCoverage) -> serde_json::Value {
    serde_json::json!({
        "embedding_model": coverage.embedding_model,
        "dimension": coverage.dimension,
        "embedded": coverage.embedded,
        "missing": coverage.missing,
    })
}

/// Embedding input: topic, title, and body so short memories keep context.
fn embedding_text(memory: &MemoryRecord) -> String {
    let mut text = String::with_capacity(memory.body.len() + memory.topic.len() + 8);
    if !memory.topic.trim().is_empty() {
        text.push_str(memory.topic.trim());
        text.push('\n');
    }
    if !memory.title.trim().is_empty() {
        text.push_str(memory.title.trim());
        text.push('\n');
    }
    text.push_str(&memory.body);
    text
}
