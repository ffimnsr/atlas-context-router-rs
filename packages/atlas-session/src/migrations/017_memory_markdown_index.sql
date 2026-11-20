-- M1 — markdown is the source of truth for memories.
--
-- `.atlas/memories/<topic-slug>/<id>.md` holds the durable record. This
-- migration keeps only derived, rebuildable state in sqlite:
--
-- - `memory_index` / `memory_index_fts`: lexical recall index rebuilt from the
--   markdown tree (`atlas index rebuild`, or opportunistically on open);
-- - `memory_access`: read-path bookkeeping (`last_accessed_at`, `decay_score`)
--   so recall and decay never rewrite truth files;
-- - `memory_embeddings`: optional vectors keyed by memory id (no FK, because
--   `memories` is now a view).
--
-- The legacy `memories` rows are copied into `memory_index` first so a repo
-- upgrading in place keeps recallable data until the exporter writes markdown.
-- Supersession rows are preserved in `memory_legacy_supersessions` so the
-- exporter can turn them into `supersedes` links.

CREATE TABLE IF NOT EXISTS memory_index (
    id              TEXT PRIMARY KEY,
    repo_root       TEXT NOT NULL,
    session_id      TEXT,
    frontend        TEXT,
    scope           TEXT NOT NULL DEFAULT 'project',
    topic           TEXT NOT NULL DEFAULT '',
    title           TEXT NOT NULL DEFAULT '',
    body            TEXT NOT NULL,
    importance      TEXT NOT NULL DEFAULT 'normal',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    source_id       TEXT,
    metadata_json   TEXT NOT NULL DEFAULT '{}',
    superseded_by   TEXT,
    tags_json       TEXT NOT NULL DEFAULT '[]',
    links_json      TEXT NOT NULL DEFAULT '[]',
    op_history_json TEXT NOT NULL DEFAULT '[]',
    numeric_value   REAL,
    path            TEXT
);

CREATE INDEX IF NOT EXISTS idx_memory_index_repo_topic
    ON memory_index(repo_root, topic);
CREATE INDEX IF NOT EXISTS idx_memory_index_repo_importance
    ON memory_index(repo_root, importance);
CREATE INDEX IF NOT EXISTS idx_memory_index_repo_scope
    ON memory_index(repo_root, scope);
CREATE INDEX IF NOT EXISTS idx_memory_index_repo_session
    ON memory_index(repo_root, session_id);
CREATE INDEX IF NOT EXISTS idx_memory_index_repo_updated
    ON memory_index(repo_root, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_index_repo_source
    ON memory_index(repo_root, source_id);
CREATE INDEX IF NOT EXISTS idx_memory_index_repo_active
    ON memory_index(repo_root)
    WHERE superseded_by IS NULL;
CREATE INDEX IF NOT EXISTS idx_memory_index_superseded
    ON memory_index(superseded_by);

CREATE TABLE IF NOT EXISTS memory_access (
    memory_id        TEXT PRIMARY KEY,
    repo_root        TEXT NOT NULL,
    last_accessed_at TEXT NOT NULL,
    access_count     INTEGER NOT NULL DEFAULT 0,
    decay_score      REAL NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_memory_access_repo
    ON memory_access(repo_root, last_accessed_at DESC);

-- Preserve legacy supersession links for the exporter, then drop the table.
CREATE TABLE IF NOT EXISTS memory_legacy_supersessions (
    old_memory_id TEXT NOT NULL,
    new_memory_id TEXT NOT NULL,
    reason        TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    PRIMARY KEY (old_memory_id, new_memory_id)
);

INSERT OR IGNORE INTO memory_legacy_supersessions
    (old_memory_id, new_memory_id, reason, created_at)
SELECT old_memory_id, new_memory_id, reason, created_at FROM memory_supersessions;

INSERT OR REPLACE INTO memory_index
    (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
     created_at, updated_at, source_id, metadata_json, superseded_by, tags_json,
     links_json, op_history_json, numeric_value, path)
SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
       created_at, updated_at, source_id, metadata_json, superseded_by, '[]',
       '[]', op_history_json, numeric_value, NULL
FROM memories;

INSERT OR REPLACE INTO memory_access
    (memory_id, repo_root, last_accessed_at, access_count, decay_score)
SELECT id, repo_root, last_accessed_at, 0, decay_score FROM memories;

DROP TRIGGER IF EXISTS memories_ai;
DROP TRIGGER IF EXISTS memories_ad;
DROP TRIGGER IF EXISTS memories_au;
DROP TABLE IF EXISTS memories_fts;
DROP TABLE IF EXISTS memories;
DROP TABLE IF EXISTS memory_supersessions;

CREATE VIEW memories AS
SELECT i.id,
       i.repo_root,
       i.session_id,
       i.frontend,
       i.scope,
       i.topic,
       i.title,
       i.body,
       i.importance,
       i.created_at,
       i.updated_at,
       COALESCE(a.last_accessed_at, i.created_at) AS last_accessed_at,
       COALESCE(a.decay_score, 0) AS decay_score,
       i.source_id,
       i.metadata_json,
       i.tags_json,
       i.superseded_by,
       i.op_history_json,
       i.numeric_value
FROM memory_index i
LEFT JOIN memory_access a ON a.memory_id = i.id;

CREATE VIRTUAL TABLE IF NOT EXISTS memory_index_fts USING fts5(
    topic,
    title,
    body,
    content = 'memory_index',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS memory_index_ai AFTER INSERT ON memory_index BEGIN
    INSERT INTO memory_index_fts(rowid, topic, title, body)
    VALUES (new.rowid, new.topic, new.title, new.body);
END;
CREATE TRIGGER IF NOT EXISTS memory_index_ad AFTER DELETE ON memory_index BEGIN
    INSERT INTO memory_index_fts(memory_index_fts, rowid, topic, title, body)
    VALUES ('delete', old.rowid, old.topic, old.title, old.body);
END;
CREATE TRIGGER IF NOT EXISTS memory_index_au AFTER UPDATE ON memory_index BEGIN
    INSERT INTO memory_index_fts(memory_index_fts, rowid, topic, title, body)
    VALUES ('delete', old.rowid, old.topic, old.title, old.body);
    INSERT INTO memory_index_fts(rowid, topic, title, body)
    VALUES (new.rowid, new.topic, new.title, new.body);
END;

INSERT INTO memory_index_fts(memory_index_fts) VALUES('rebuild');

-- Rebuild `memory_embeddings` without the foreign key: the referenced
-- `memories` table is now a view.
CREATE TABLE IF NOT EXISTS memory_embeddings_new (
    memory_id       TEXT PRIMARY KEY,
    embedding_model TEXT NOT NULL,
    dimension       INTEGER NOT NULL CHECK (dimension > 0),
    vector_blob     BLOB NOT NULL,
    created_at      TEXT NOT NULL
);

INSERT OR REPLACE INTO memory_embeddings_new
    (memory_id, embedding_model, dimension, vector_blob, created_at)
SELECT memory_id, embedding_model, dimension, vector_blob, created_at
FROM memory_embeddings;

DROP TABLE memory_embeddings;
ALTER TABLE memory_embeddings_new RENAME TO memory_embeddings;

CREATE INDEX IF NOT EXISTS idx_memory_embeddings_model
    ON memory_embeddings(embedding_model, dimension);
