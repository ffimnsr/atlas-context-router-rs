use super::*;

fn open_store_with_config(config: ContentStoreConfig) -> ContentStore {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    std::mem::forget(file);
    let mut store = ContentStore::open_with_config(&path, config).unwrap();
    store.migrate().unwrap();
    store
}

/// Generate text with `n` blank-line-separated paragraphs, each producing one chunk.
fn multi_chunk_text(n: usize) -> String {
    (0..n)
        .map(|i| format!("Paragraph {i} with some unique content at index {i}."))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[test]
fn chunk_explosion_from_large_file_partial_with_warning() {
    // 20 paragraphs → 20 chunks; cap at 5 → must truncate.
    let text = multi_chunk_text(20);
    let mut store = open_store_with_config(ContentStoreConfig {
        max_chunks_per_file: 5,
        oversized_policy: OversizedPolicy::PartialWithWarning,
        ..ContentStoreConfig::default()
    });
    store
        .index_artifact(meta("large-pw"), &text, "text/plain")
        .unwrap();
    let chunks = store.get_chunks("large-pw").unwrap();
    assert!(
        chunks.len() <= 5,
        "PartialWithWarning must truncate to cap; got {}",
        chunks.len()
    );
    assert!(!chunks.is_empty(), "at least one chunk must be indexed");
}

#[test]
fn chunk_explosion_from_large_file_fail_fast() {
    let text = multi_chunk_text(20);
    let mut store = open_store_with_config(ContentStoreConfig {
        max_chunks_per_file: 5,
        oversized_policy: OversizedPolicy::FailFast,
        ..ContentStoreConfig::default()
    });
    let result = store.index_artifact(meta("large-ff"), &text, "text/plain");
    assert!(
        matches!(result, Err(atlas_core::AtlasError::ChunkCapExceeded(_))),
        "FailFast must return ChunkCapExceeded; got: {result:?}"
    );
    let chunks = store.get_chunks("large-ff").unwrap();
    assert!(chunks.is_empty(), "no chunks must be written on FailFast");
}

#[test]
fn chunk_explosion_from_large_file_skip_file() {
    let text = multi_chunk_text(20);
    let mut store = open_store_with_config(ContentStoreConfig {
        max_chunks_per_file: 5,
        oversized_policy: OversizedPolicy::SkipFile,
        ..ContentStoreConfig::default()
    });
    store
        .index_artifact(meta("large-sf"), &text, "text/plain")
        .unwrap();
    let chunks = store.get_chunks("large-sf").unwrap();
    assert!(chunks.is_empty(), "SkipFile must write no chunks");
}

#[test]
fn recursive_fallback_chunk_explosion_partial_with_warning() {
    // Markdown with many headings, each with unique body → each heading = one chunk.
    let heading_count = 30;
    let text: String = (0..heading_count)
        .map(|i| format!("# Heading {i}\nBody content for section {i}, unique text here.\n"))
        .collect();
    let mut store = open_store_with_config(ContentStoreConfig {
        max_chunks_per_file: 10,
        oversized_policy: OversizedPolicy::PartialWithWarning,
        ..ContentStoreConfig::default()
    });
    store
        .index_artifact(meta("md-explode"), &text, "text/markdown")
        .unwrap();
    let chunks = store.get_chunks("md-explode").unwrap();
    assert!(
        chunks.len() <= 10,
        "recursive markdown chunking must be capped; got {}",
        chunks.len()
    );
}

#[test]
fn partial_indexing_recovery_after_run_cap_hit() {
    // Cap the run at 3 chunks total. Index two files: first OK, second truncated.
    let small_text = "alpha beta gamma delta epsilon"; // 1 chunk
    let large_text = multi_chunk_text(10); // 10 chunks
    let mut store = open_store_with_config(ContentStoreConfig {
        max_chunks_per_index_run: 3,
        max_chunks_per_file: 1000,
        oversized_policy: OversizedPolicy::PartialWithWarning,
        ..ContentStoreConfig::default()
    });
    store.reset_run_stats();

    store
        .index_artifact(meta("run-cap-a"), small_text, "text/plain")
        .unwrap();
    store
        .index_artifact(meta("run-cap-b"), &large_text, "text/plain")
        .unwrap();

    let chunks_b = store.get_chunks("run-cap-b").unwrap();
    let stats = store.run_stats();
    // total chunks_this_run must not exceed max_chunks_per_index_run
    assert!(
        stats.chunks_this_run <= 3,
        "run cap must be respected; chunks_this_run={}",
        stats.chunks_this_run
    );
    // second file must have been truncated, not skipped entirely
    assert!(
        !chunks_b.is_empty(),
        "partial indexing: second file must still index some chunks"
    );
}

#[test]
fn run_stats_track_batch_flushes() {
    // With retrieval_batch_size=2 and 5 chunks, expect at least 2 flushes.
    let text = multi_chunk_text(5);
    let mut store = open_store_with_config(ContentStoreConfig {
        retrieval_batch_size: 2,
        max_chunks_per_file: 1000,
        ..ContentStoreConfig::default()
    });
    store.reset_run_stats();
    store
        .index_artifact(meta("batch-flush"), &text, "text/plain")
        .unwrap();
    let stats = store.run_stats();
    assert!(
        stats.batch_flush_count >= 2,
        "with batch_size=2 and >=5 chunks, must have >=2 flushes; got {}",
        stats.batch_flush_count
    );
    assert!(stats.buffered_chunk_count >= 5);
    assert!(stats.buffered_bytes > 0);
    assert!(stats.staged_vector_bytes > 0);
}

#[test]
fn run_stats_reset_clears_counters() {
    let text = multi_chunk_text(3);
    let mut store = open_store_with_config(ContentStoreConfig::default());
    store
        .index_artifact(meta("rs-reset"), &text, "text/plain")
        .unwrap();
    let before = store.run_stats();
    assert!(before.buffered_chunk_count > 0);

    store.reset_run_stats();
    let after = store.run_stats();
    assert_eq!(
        after,
        IndexRunStats::default(),
        "reset must zero all counters"
    );
}

#[test]
fn configurable_batch_and_embedding_sizes_in_config() {
    let config = ContentStoreConfig {
        retrieval_batch_size: 50,
        embedding_batch_size: 16,
        ..ContentStoreConfig::default()
    };
    assert_eq!(config.retrieval_batch_size, 50);
    assert_eq!(config.embedding_batch_size, 16);
}

// ---------------------------------------------------------------------------
// Patch R3 — Embedding dimension registry and freeze rules
// ---------------------------------------------------------------------------
