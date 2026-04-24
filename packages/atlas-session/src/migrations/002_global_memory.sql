-- CM11 — Cross-Session Intelligence: global memory tables.
--
-- These tables aggregate access patterns across all sessions for a repo so
-- Atlas can recall frequently-accessed symbols, files, and recurring workflows
-- even after individual sessions are cleared.
--
-- Design rules:
-- - Keyed by (repo_root, value) so data stays repo-scoped.
-- - Updated in-place with increment-or-insert semantics.
-- - Decoupled from per-session event tables; survive session deletion.

CREATE TABLE IF NOT EXISTS global_symbol_access (
    id              TEXT PRIMARY KEY,   -- sha256 hex of (repo_root || ":" || symbol_qn)
    repo_root       TEXT NOT NULL,
    symbol_qn       TEXT NOT NULL,
    access_count    INTEGER NOT NULL DEFAULT 1,
    last_accessed   TEXT NOT NULL,
    first_accessed  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_global_symbol_repo_count
    ON global_symbol_access(repo_root, access_count DESC, last_accessed DESC);

CREATE TABLE IF NOT EXISTS global_file_access (
    id              TEXT PRIMARY KEY,   -- sha256 hex of (repo_root || ":" || file_path)
    repo_root       TEXT NOT NULL,
    file_path       TEXT NOT NULL,
    access_count    INTEGER NOT NULL DEFAULT 1,
    last_accessed   TEXT NOT NULL,
    first_accessed  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_global_file_repo_count
    ON global_file_access(repo_root, access_count DESC, last_accessed DESC);

CREATE TABLE IF NOT EXISTS global_workflow_patterns (
    id                TEXT PRIMARY KEY,  -- sha256 hex of (repo_root || ":" || pattern_json)
    repo_root         TEXT NOT NULL,
    pattern_json      TEXT NOT NULL,     -- JSON array of event_type/command strings
    occurrence_count  INTEGER NOT NULL DEFAULT 1,
    last_seen         TEXT NOT NULL,
    first_seen        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_global_workflow_repo_count
    ON global_workflow_patterns(repo_root, occurrence_count DESC, last_seen DESC);
