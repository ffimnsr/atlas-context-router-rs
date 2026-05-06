-- Migration 007: embedding provider registry and dimension freeze (Patch R3).
--
-- One row per (provider_name, model_name) pair tracks the expected embedding
-- dimension for that combination.  The dimension is frozen the first time a
-- vector is produced by that provider+model pair.  Subsequent insert and
-- search operations MUST present vectors of the same dimension or be rejected.
--
-- `provider_name` is derived from the base_url heuristic used by EmbeddingConfig:
--   URLs containing `/v1` → "openai"
--   All others            → "ollama"
--
-- `index_schema_version` is bumped if the storage format for embeddings changes
-- (e.g. byte encoding, normalisation).  A mismatch between stored schema version
-- and the current runtime version requires a forced rebuild of all embeddings.

CREATE TABLE IF NOT EXISTS embedding_provider_registry (
    id                   INTEGER PRIMARY KEY,
    provider_name        TEXT    NOT NULL,
    model_name           TEXT    NOT NULL,
    dimension            INTEGER NOT NULL CHECK (dimension > 0),
    discovered_at        TEXT    NOT NULL,
    index_schema_version INTEGER NOT NULL DEFAULT 1,
    UNIQUE(provider_name, model_name)
);
