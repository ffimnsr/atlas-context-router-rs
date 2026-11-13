-- Migration 016: index hot session read paths and drop redundant indexes.
--
-- The audit behind this migration is ISSUES.md "SQLite Index Audit Patch".
-- Plans were checked with `EXPLAIN QUERY PLAN` against a live `session.db`.
--
-- Adds come before drops so a partially applied migration never loses coverage.

-- ── P0: overview active-record resolution ────────────────────────────────────
-- `ACTIVE_PREDICATE` is a correlated `NOT EXISTS (... WHERE newer.supersedes_id
-- = overview_records.id)`. Without this index the planner scans the whole table
-- once per candidate row.
CREATE INDEX IF NOT EXISTS idx_overview_records_supersedes
    ON overview_records (supersedes_id)
    WHERE supersedes_id IS NOT NULL;

-- `overview_source_ids` repair query filters non-empty `source_id`.
CREATE INDEX IF NOT EXISTS idx_overview_records_source_id
    ON overview_records (source_id)
    WHERE source_id IS NOT NULL;

-- ── P1: ordering, filters, and scoped lookups ────────────────────────────────
-- `list_sessions` ordering and `cleanup_stale_sessions` cutoff.
CREATE INDEX IF NOT EXISTS idx_session_meta_updated_at
    ON session_meta (updated_at DESC);

-- `recent_feedback` (wake-up packs) and feedback LIKE-fallback ordering.
CREATE INDEX IF NOT EXISTS idx_feedback_repo_created
    ON feedback_records (repo_root, created_at DESC, id DESC);

-- Experience activation (`freshness = 'fresh'`) and pruning by validation age.
CREATE INDEX IF NOT EXISTS idx_experience_edges_repo_freshness
    ON experience_edges (repo_root, freshness);
CREATE INDEX IF NOT EXISTS idx_experience_edges_repo_validated
    ON experience_edges (repo_root, last_validated_at);

-- Repo-scoped memoir relation walk (`source_concept_id = ? OR target_concept_id = ?`).
CREATE INDEX IF NOT EXISTS idx_memoir_relations_repo_source
    ON memoir_relations (repo_root, source_concept_id);
CREATE INDEX IF NOT EXISTS idx_memoir_relations_repo_target
    ON memoir_relations (repo_root, target_concept_id);

-- ── P2: memory lookups ───────────────────────────────────────────────────────
-- Recall and embedding-coverage counts only consider active memories.
CREATE INDEX IF NOT EXISTS idx_memories_repo_active
    ON memories (repo_root)
    WHERE superseded_by IS NULL;

-- `list_memories` ordering.
CREATE INDEX IF NOT EXISTS idx_memories_repo_updated
    ON memories (repo_root, updated_at DESC);

-- Source-id recall joins.
CREATE INDEX IF NOT EXISTS idx_memories_repo_source
    ON memories (repo_root, source_id);

-- Cursor pagination orders by `updated_at DESC, created_at DESC, task_id DESC`.
DROP INDEX IF EXISTS idx_durable_tasks_updated_task;
CREATE INDEX IF NOT EXISTS idx_durable_tasks_updated_created_task
    ON durable_tasks (updated_at DESC, created_at DESC, task_id DESC);

-- Global-memory focus lookups filter `event_type` without a session id.
CREATE INDEX IF NOT EXISTS idx_session_events_event_type
    ON session_events (event_type, session_id);

-- ── Drop duplicate indexes ───────────────────────────────────────────────────
-- Prefix of the corresponding primary key or UNIQUE autoindex.
DROP INDEX IF EXISTS idx_memoir_concepts_graph;   -- memoir_concepts(graph_id, name) UNIQUE
DROP INDEX IF EXISTS idx_memoir_graphs_repo_scope; -- memoir_graphs(repo_root, scope, name) UNIQUE
DROP INDEX IF EXISTS idx_experience_edges_source; -- experience_edges(repo_root, source_uri, ...) UNIQUE
DROP INDEX IF EXISTS idx_overview_records_repo;   -- prefix of idx_overview_records_subject

-- ── Drop indexes with no current query usage ─────────────────────────────────
-- Overview filtering happens in Rust after `load_records`; packets are read by
-- `packet_id`. `idx_overview_records_subject` stays: the partial active-key
-- unique index cannot replace repo + subject lookups.
DROP INDEX IF EXISTS idx_overview_records_commit;
DROP INDEX IF EXISTS idx_overview_records_file;
DROP INDEX IF EXISTS idx_overview_records_qname;
DROP INDEX IF EXISTS idx_overview_records_freshness;
DROP INDEX IF EXISTS idx_overview_records_subject_kind;
DROP INDEX IF EXISTS idx_overview_packets_repo;

-- Every decision query includes `repo_root`.
DROP INDEX IF EXISTS idx_decision_memory_session_updated;

-- Nothing filters or orders by `last_accessed_at`.
DROP INDEX IF EXISTS idx_memories_repo_accessed;

-- Supersession rows are only inserted, never read by the new id index.
DROP INDEX IF EXISTS idx_memory_supersessions_new;
