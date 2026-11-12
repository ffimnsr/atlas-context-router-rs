-- ICM-E2 — optional dense-vector recall for the shared memory surface.
--
-- Embeddings never become required: lexical `memories_fts` recall plus origin,
-- freshness, and importance scoring stays the default path. This table only
-- stores opt-in vectors keyed by memory id so hybrid recall can fuse a second
-- ranking path without changing memory identity, scope, or visibility rules.
--
-- `dimension` is persisted next to the blob so a config/model change cannot
-- silently reinterpret a stale vector. `embedding_model` is part of the
-- compatibility check because two models can share a dimension.

CREATE TABLE IF NOT EXISTS memory_embeddings (
    memory_id       TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
    embedding_model TEXT NOT NULL,
    dimension       INTEGER NOT NULL CHECK (dimension > 0),
    vector_blob     BLOB NOT NULL,
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_embeddings_model
    ON memory_embeddings(embedding_model, dimension);
