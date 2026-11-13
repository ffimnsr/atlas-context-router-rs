-- Migration 021: index hot graph read paths and drop redundant indexes.
--
-- The audit behind this migration is ISSUES.md "SQLite Index Audit Patch".
-- Plans were checked with `EXPLAIN QUERY PLAN` against a live `worldtree.db`.
--
-- Two rules drive the new indexes:
--   * default SQLite `LIKE` is case-insensitive, so a BINARY index cannot serve
--     `column LIKE 'prefix%'`; prefix filters need `COLLATE NOCASE` indexes.
--     The NOCASE index does NOT serve `column = ?`; separate BINARY indexes are
--     kept for equality and ordering.
--   * composite indexes ordered by the exact `ORDER BY` of a hot query remove
--     temp B-tree sorts during result streaming.
--
-- Adds come before drops so a partially applied migration never loses coverage.

-- ── P0: name lookups ─────────────────────────────────────────────────────────
-- `nodes_by_name`: `WHERE name = ?1 ORDER BY file_path, line_start`.
CREATE INDEX IF NOT EXISTS idx_nodes_name_order
    ON nodes (name, file_path, line_start);

-- `nodes_by_name_prefix`: `WHERE name LIKE ?1 ESCAPE '\'` (case-insensitive).
CREATE INDEX IF NOT EXISTS idx_nodes_name_nocase
    ON nodes (name COLLATE NOCASE);

-- ── P1: file-path reads, ordering, and LIKE prefixes ─────────────────────────
-- Replaces `idx_nodes_file_path`; covers equality plus the full list ordering.
CREATE INDEX IF NOT EXISTS idx_nodes_file_path_order
    ON nodes (file_path, line_start, qualified_name);

-- Replaces `idx_nodes_source_repo_file_path` with the repo-scoped full order.
CREATE INDEX IF NOT EXISTS idx_nodes_repo_file_path_order
    ON nodes (source_repo_id, file_path, line_start, qualified_name);

-- Subpath / exclusion filters in `search.rs` and `dead_code_candidates_filtered`.
-- Note: `file_paths_with_prefix_for_repo` deliberately uses a half-open BINARY
-- range instead of `LIKE`, so `files(source_repo_id, path)` needs no NOCASE twin.
CREATE INDEX IF NOT EXISTS idx_nodes_file_path_nocase
    ON nodes (file_path COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_nodes_repo_file_path_nocase
    ON nodes (source_repo_id, file_path COLLATE NOCASE);

-- `containment_siblings`: `WHERE file_path = ? AND parent_name = ?`.
CREATE INDEX IF NOT EXISTS idx_nodes_file_parent
    ON nodes (file_path, parent_name);

-- Unscoped `SELECT hash FROM files WHERE path = ?1` and single-source lookups.
CREATE INDEX IF NOT EXISTS idx_files_path
    ON files (path);

-- `recently_indexed_files` and `MAX(indexed_at)` freshness checks.
CREATE INDEX IF NOT EXISTS idx_files_indexed_at
    ON files (indexed_at DESC);

-- Replaces `idx_edges_target`; dead-code anti-joins filter target + kind.
CREATE INDEX IF NOT EXISTS idx_edges_target_kind
    ON edges (target_qualified, kind);

-- Replaces `idx_edges_source`; ordered whole-graph edge export.
CREATE INDEX IF NOT EXISTS idx_edges_order
    ON edges (source_qualified, target_qualified, kind, file_path);

-- Replaces `idx_commits_repo_id`; `list_commits` and history status ordering.
CREATE INDEX IF NOT EXISTS idx_commits_repo_author_time
    ON commits (repo_id, author_time DESC, commit_sha DESC);

-- ── P2: lower-impact filters ─────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_edges_repo_confidence_tier
    ON edges (source_repo_id, confidence_tier);

-- Repo-scoped embedded-chunk coverage counts (`vec_index::embedded_count`).
CREATE INDEX IF NOT EXISTS idx_retrieval_chunks_repo_embedded
    ON retrieval_chunks (source_repo_id)
    WHERE embedding IS NOT NULL;

-- ── Drop indexes replaced by the new composites ─────────────────────────────
-- Each new index keeps the old prefix as its leading columns.
DROP INDEX IF EXISTS idx_nodes_file_path;              -- idx_nodes_file_path_order
DROP INDEX IF EXISTS idx_nodes_source_repo_file_path;   -- idx_nodes_repo_file_path_order
DROP INDEX IF EXISTS idx_edges_target;                  -- idx_edges_target_kind
DROP INDEX IF EXISTS idx_edges_source;                  -- idx_edges_order
DROP INDEX IF EXISTS idx_commits_repo_id;               -- idx_commits_repo_author_time

-- ── Drop duplicate indexes ───────────────────────────────────────────────────
-- Prefixes of a primary key or UNIQUE autoindex: the autoindex already serves
-- every lookup that these can serve.
DROP INDEX IF EXISTS idx_files_source_repo_path;            -- files(source_repo_id, path) PK
DROP INDEX IF EXISTS idx_repos_root_path;                   -- repos(root_path) UNIQUE
DROP INDEX IF EXISTS idx_graph_snapshots_repo_id;           -- (repo_id, commit_sha) UNIQUE
DROP INDEX IF EXISTS idx_historical_nodes_file_hash;        -- historical_nodes PK prefix
DROP INDEX IF EXISTS idx_historical_edges_file_hash;        -- historical_edges PK prefix
DROP INDEX IF EXISTS idx_snapshot_files_snapshot_id;        -- snapshot_files PK prefix
DROP INDEX IF EXISTS idx_snapshot_edges_snapshot_id;        -- snapshot_edges PK prefix
DROP INDEX IF EXISTS idx_snapshot_nodes_snapshot_id;        -- snapshot_nodes PK prefix
DROP INDEX IF EXISTS idx_snapshot_membership_blobs_snapshot_id; -- blobs PK prefix
DROP INDEX IF EXISTS idx_node_history_repo_id;              -- node_history PK prefix
DROP INDEX IF EXISTS idx_edge_history_repo_id;              -- edge_history PK prefix

-- ── Drop indexes with no current query usage ─────────────────────────────────
-- Superseded by `idx_commits_repo_author_time`.
DROP INDEX IF EXISTS idx_commits_indexed_ref;
DROP INDEX IF EXISTS idx_commits_author_time;
DROP INDEX IF EXISTS idx_commits_committer_time;

-- Every snapshot lookup filters `repo_id` and uses the UNIQUE autoindex.
DROP INDEX IF EXISTS idx_graph_snapshots_commit_sha;

-- Snapshot membership is read by `snapshot_id`; hash joins go through the
-- `historical_*` primary keys. `idx_snapshot_files_file_hash` is kept because
-- `prune_orphan_historical_file_graphs` joins on it.
DROP INDEX IF EXISTS idx_snapshot_membership_blobs_file_hash;
DROP INDEX IF EXISTS idx_snapshot_nodes_file_hash;
DROP INDEX IF EXISTS idx_snapshot_edges_file_hash;

-- History rows are read per repo and reordered in Rust; no query filters by
-- qualified name or commit SHA alone.
DROP INDEX IF EXISTS idx_node_history_qualified_name;
DROP INDEX IF EXISTS idx_node_history_introduction_commit;
DROP INDEX IF EXISTS idx_node_history_removal_commit;
DROP INDEX IF EXISTS idx_edge_history_source_qn;
DROP INDEX IF EXISTS idx_edge_history_target_qn;
DROP INDEX IF EXISTS idx_edge_history_introduction_commit;
DROP INDEX IF EXISTS idx_edge_history_removal_commit;

-- Taxonomy queries fetch by id/name or list every row.
DROP INDEX IF EXISTS idx_flows_kind;
DROP INDEX IF EXISTS idx_communities_algorithm;
DROP INDEX IF EXISTS idx_communities_parent;

-- Postprocess state is only read by `repo_root` (the primary key).
DROP INDEX IF EXISTS idx_postprocess_state_state;
DROP INDEX IF EXISTS idx_postprocess_state_updated_at_ms;

-- No production query filters `edges` by `kind` alone.
DROP INDEX IF EXISTS idx_edges_kind;
