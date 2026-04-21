-- Migration 003: retrieval index lifecycle state tracking (Patch R1).
--
-- One row per repo_root tracks whether the content store for that repository
-- is currently being indexed, fully indexed and searchable, or in a failed
-- state.  This is the single source of truth for "is this repo searchable now?"
--
-- States:
--   indexing     — an indexing run is in progress (or was interrupted)
--   indexed      — last run completed successfully; content is searchable
--   index_failed — last run failed; last_error contains the reason
--
-- If a row's state is still 'indexing' at startup, the previous run was
-- interrupted.  Callers should call begin_indexing() to restart cleanly.

CREATE TABLE IF NOT EXISTS retrieval_index_state (
    repo_root        TEXT    PRIMARY KEY,
    state            TEXT    NOT NULL DEFAULT 'indexed',
    files_discovered INTEGER NOT NULL DEFAULT 0,
    files_indexed    INTEGER NOT NULL DEFAULT 0,
    chunks_written   INTEGER NOT NULL DEFAULT 0,
    chunks_reused    INTEGER NOT NULL DEFAULT 0,
    last_indexed_at  TEXT,
    last_error       TEXT,
    updated_at       TEXT    NOT NULL
);
