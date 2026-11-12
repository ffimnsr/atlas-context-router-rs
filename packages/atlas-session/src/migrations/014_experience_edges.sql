-- VR4 — query-conditioned experience edges.
--
-- These edges are retrieval experience, not code-graph truth: they live only
-- in `session.db` and are always bound to a commit plus endpoint file hashes.

CREATE TABLE IF NOT EXISTS experience_edges (
    id                  TEXT PRIMARY KEY,
    repo_root           TEXT NOT NULL,
    source_uri          TEXT NOT NULL,
    target_uri          TEXT NOT NULL,
    query_text          TEXT NOT NULL,
    query_tokens_json   TEXT NOT NULL DEFAULT '[]',
    query_embedding     BLOB,
    trace_summary       TEXT NOT NULL,
    commit_sha          TEXT NOT NULL,
    source_file_hash    TEXT,
    target_file_hash    TEXT,
    use_count           INTEGER NOT NULL DEFAULT 1 CHECK (use_count > 0),
    confidence          REAL NOT NULL DEFAULT 1.0 CHECK (confidence >= 0 AND confidence <= 1),
    freshness           TEXT NOT NULL DEFAULT 'fresh'
        CHECK (freshness IN ('fresh', 'possibly_stale', 'invalid')),
    created_at          TEXT NOT NULL,
    last_validated_at   TEXT NOT NULL,
    UNIQUE (repo_root, source_uri, target_uri, query_text)
);

CREATE INDEX IF NOT EXISTS idx_experience_edges_source
    ON experience_edges (repo_root, source_uri);
CREATE INDEX IF NOT EXISTS idx_experience_edges_target
    ON experience_edges (repo_root, target_uri);

-- Reverse-index dependency rows used by deterministic invalidation.
CREATE TABLE IF NOT EXISTS experience_edge_deps (
    edge_id   TEXT NOT NULL REFERENCES experience_edges(id) ON DELETE CASCADE,
    repo_root TEXT NOT NULL,
    file_path TEXT NOT NULL,
    file_hash TEXT,
    PRIMARY KEY (edge_id, repo_root, file_path)
);

CREATE INDEX IF NOT EXISTS idx_experience_edge_deps_path
    ON experience_edge_deps (repo_root, file_path);
