-- Migration 011: index hot content-store read paths and drop redundant indexes.
--
-- The audit behind this migration is ISSUES.md "SQLite Index Audit Patch".
-- Plans were checked with `EXPLAIN QUERY PLAN` against a live `context.db`.
--
-- Adds come before drops so a partially applied migration never loses coverage.

-- ── P1: source ordering and lookups ──────────────────────────────────────────
-- Oldest-source lookup, age cleanup cutoff, and `ORDER BY created_at DESC`.
CREATE INDEX IF NOT EXISTS idx_sources_created_at
    ON sources (created_at);

-- Session-scoped source listing.
CREATE INDEX IF NOT EXISTS idx_sources_session_created
    ON sources (session_id, created_at DESC);

-- `recent_source_ids_by_prefix` keeps `ORDER BY created_at DESC LIMIT n`, so the
-- planner walks `idx_sources_created_at` and filters `id LIKE 'prefix%'` in
-- place. A NOCASE id index would never be chosen: the ordered covering scan is
-- always cheaper for a bounded artifact table.

-- ── Drop duplicate indexes ───────────────────────────────────────────────────
-- Prefix of the `chunks(source_id, chunk_index)` UNIQUE autoindex.
DROP INDEX IF EXISTS idx_chunks_source_id;
-- Prefix of `idx_sources_session_agent`.
DROP INDEX IF EXISTS idx_sources_session;

-- ── Drop indexes with no current query usage ─────────────────────────────────
-- Every chunk-id lookup is paired with `source_id` and uses
-- `idx_chunks_source_chunk_id`.
DROP INDEX IF EXISTS idx_chunks_chunk_id;
-- No query filters `sources.repo_id`.
DROP INDEX IF EXISTS idx_sources_repo_id;
