-- ICM-A1 — Shared memory surface: `memories` table.
--
-- Generic memory records shared by CLI and MCP. Stored in the continuity
-- (session) database, NOT worldtree.db (graph) or context.db (content), so
-- memory bodies never land in graph storage.
--
-- Design rules:
-- - Scope/importance are constrained at the storage boundary; unknown values
--   are rejected by CHECK constraints in addition to the Rust model layer.
-- - `session`-scoped memories require a session id, `frontend`-scoped
--   memories require a frontend identifier.
-- - `project` and `global` writes do not need a session id (no active
--   session required).

CREATE TABLE IF NOT EXISTS memories (
    id               TEXT PRIMARY KEY,
    repo_root        TEXT NOT NULL,
    session_id       TEXT,
    frontend         TEXT,
    scope            TEXT NOT NULL DEFAULT 'project',
    topic            TEXT NOT NULL DEFAULT '',
    title            TEXT NOT NULL DEFAULT '',
    body             TEXT NOT NULL,
    importance       TEXT NOT NULL DEFAULT 'normal',
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    last_accessed_at TEXT NOT NULL,
    decay_score      REAL NOT NULL DEFAULT 0,
    source_id        TEXT,
    metadata_json    TEXT NOT NULL DEFAULT '{}',
    CHECK (scope IN ('project', 'session', 'frontend', 'global')),
    CHECK (importance IN ('critical', 'high', 'normal', 'low')),
    CHECK (scope <> 'session' OR (session_id IS NOT NULL AND session_id <> '')),
    CHECK (scope <> 'frontend' OR (frontend IS NOT NULL AND frontend <> ''))
);

CREATE INDEX IF NOT EXISTS idx_memories_repo_topic
    ON memories(repo_root, topic);

CREATE INDEX IF NOT EXISTS idx_memories_repo_importance
    ON memories(repo_root, importance);

CREATE INDEX IF NOT EXISTS idx_memories_repo_scope
    ON memories(repo_root, scope);

CREATE INDEX IF NOT EXISTS idx_memories_repo_session
    ON memories(repo_root, session_id);

CREATE INDEX IF NOT EXISTS idx_memories_repo_accessed
    ON memories(repo_root, last_accessed_at DESC);
