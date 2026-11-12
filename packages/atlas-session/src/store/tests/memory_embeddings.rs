//! ICM-E2 — optional vector storage and hybrid recall behavior.

use super::*;

fn policy() -> MemoryRecallPolicy {
    MemoryRecallPolicy::default()
}

fn embedding_policy() -> MemoryEmbeddingPolicy {
    MemoryEmbeddingPolicy::default()
}

fn unit_vector(dimension: usize, hot: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; dimension];
    vector[hot % dimension] = 1.0;
    vector
}

fn store_embedding(store: &mut SessionStore, memory_id: &str, vector: &[f32]) {
    store
        .store_memory_embedding(memory_id, "test-embed", vector.len(), vector)
        .expect("store embedding");
}

#[test]
fn embedding_round_trip_and_dimension_mismatch_rejected() {
    let (_dir, mut store) = open_store(100, 4096);
    let memory = seed_memory(
        &store,
        "m1",
        "2025-01-01T00:00:00Z",
        "postgres pool sizing",
        "postgres",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    let vector = vec![0.25_f32, -0.5, 1.0];
    store_embedding(&mut store, &memory.id, &vector);

    let stored = store
        .memory_embedding(&memory.id)
        .expect("read embedding")
        .expect("embedding row");
    assert_eq!(stored.0, "test-embed");
    assert_eq!(stored.1, 3);
    assert_eq!(stored.2, vector);

    // A different configured dimension for the same memory is rejected.
    let error = store
        .store_memory_embedding(&memory.id, "test-embed", 4, &[1.0, 0.0, 0.0, 0.0])
        .expect_err("dimension mismatch must fail");
    assert!(
        error
            .to_string()
            .contains("memory_embedding_dimension_mismatch"),
        "unexpected error: {error}"
    );

    // A vector whose length disagrees with the configured dimension is rejected.
    let error = store
        .store_memory_embedding(&memory.id, "test-embed", 3, &[1.0, 0.0])
        .expect_err("vector length mismatch must fail");
    assert!(
        error
            .to_string()
            .contains("memory_embedding_dimension_mismatch"),
        "unexpected error: {error}"
    );
}

#[test]
fn embedding_coverage_counts_recallable_rows_only() {
    let (_dir, mut store) = open_store(100, 4096);
    let embedded = seed_memory(
        &store,
        "m1",
        "2025-01-01T00:00:00Z",
        "first body",
        "topic-a",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    let missing = seed_memory(
        &store,
        "m2",
        "2025-01-02T00:00:00Z",
        "second body",
        "topic-b",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    let _superseded = seed_memory(
        &store,
        "m3",
        "2025-01-03T00:00:00Z",
        "third body",
        "topic-c",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    store
        .conn
        .execute(
            "UPDATE memories SET superseded_by = ?2 WHERE id = ?1",
            params!["m3", "m1"],
        )
        .expect("mark superseded");

    store_embedding(&mut store, &embedded.id, &unit_vector(2, 0));

    let coverage = store
        .memory_embedding_coverage("/repo", "test-embed", 2)
        .expect("coverage");
    assert_eq!(coverage.embedded, 1);
    assert_eq!(coverage.missing, 1, "superseded rows are not recallable");

    let pending = store
        .memories_missing_embeddings("/repo", "test-embed", 2, &MemoryListFilter::default(), 10)
        .expect("missing embeddings");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, missing.id);
}

#[test]
fn hybrid_recall_keeps_lexical_hits_ahead_of_vector_only_matches() {
    let (_dir, mut store) = open_store(100, 4096);
    let lexical = seed_memory(
        &store,
        "m1",
        "2025-01-01T00:00:00Z",
        "postgres connection pool sizing notes",
        "postgres",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    let vector_only = seed_memory_full(
        &store,
        "m2",
        "2025-01-02T00:00:00Z",
        "database client reuse strategy decisions",
        "database",
        MemoryImportance::Normal,
        MemoryScope::Project,
        None,
        None,
        None,
    );

    store_embedding(&mut store, &lexical.id, &unit_vector(2, 0));
    store_embedding(&mut store, &vector_only.id, &unit_vector(2, 0));

    let hits = store
        .recall_memories_hybrid_with_policy(
            "/repo",
            "postgres connection pool",
            &unit_vector(2, 0),
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            5,
            &policy(),
            &embedding_policy(),
        )
        .expect("hybrid recall");

    assert_eq!(hits.len(), 2, "both paths contribute");
    assert_eq!(hits[0].memory.id, lexical.id);
    assert_eq!(hits[0].ranking.recall_path, "primary");
    assert!(hits[0].ranking.vector_score.is_some());
    assert!(hits[0].ranking.hybrid_score.is_some());

    let tail = hits.last().expect("vector-only tail");
    assert_eq!(tail.memory.id, vector_only.id);
    assert_eq!(tail.ranking.recall_path, "vector");
    assert_eq!(tail.relevance_score, 3);
    assert!(tail.ranking.hybrid_score.unwrap_or(0.0) > 0.0);
}

#[test]
fn hybrid_recall_respects_visibility_and_min_similarity() {
    let (_dir, mut store) = open_store(100, 4096);
    let private = seed_memory_full(
        &store,
        "m1",
        "2025-01-01T00:00:00Z",
        "unrelated wording about caching tiers",
        "caching",
        MemoryImportance::Normal,
        MemoryScope::Frontend,
        None,
        Some("codex"),
        None,
    );
    store_embedding(&mut store, &private.id, &unit_vector(2, 0));

    let hits = store
        .recall_memories_hybrid_with_policy(
            "/repo",
            "postgres connection pool",
            &unit_vector(2, 0),
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            5,
            &policy(),
            &embedding_policy(),
        )
        .expect("hybrid recall");
    assert!(
        hits.is_empty(),
        "another frontend's private memory must never surface"
    );

    // Same vector, but below the configured similarity floor.
    let strict = MemoryEmbeddingPolicy {
        min_similarity: 0.99,
        ..MemoryEmbeddingPolicy::default()
    };
    let lexical = seed_memory(
        &store,
        "m2",
        "2025-01-01T00:00:00Z",
        "postgres connection pool notes",
        "postgres",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    store_embedding(&mut store, &lexical.id, &unit_vector(4, 1));
    let hits = store
        .recall_memories_hybrid_with_policy(
            "/repo",
            "postgres connection pool",
            &unit_vector(4, 0),
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            5,
            &policy(),
            &strict,
        )
        .expect("hybrid recall");
    assert_eq!(hits.len(), 1, "orthogonal vector is below the floor");
    assert_eq!(hits[0].memory.id, lexical.id);
    assert!(hits[0].ranking.vector_score.is_none());
}

#[test]
fn hybrid_recall_without_embeddings_falls_back_to_lexical() {
    let (_dir, store) = open_store(100, 4096);
    let memory = seed_memory(
        &store,
        "m1",
        "2025-01-01T00:00:00Z",
        "postgres pool sizing",
        "postgres",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    let hits = store
        .recall_memories_hybrid_with_policy(
            "/repo",
            "postgres pool",
            &unit_vector(2, 0),
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            5,
            &policy(),
            &embedding_policy(),
        )
        .expect("hybrid recall");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.id, memory.id);
    assert_eq!(hits[0].ranking.recall_path, "primary");
    assert!(hits[0].ranking.vector_score.is_none());
    assert!(hits[0].ranking.hybrid_score.is_some());
}
