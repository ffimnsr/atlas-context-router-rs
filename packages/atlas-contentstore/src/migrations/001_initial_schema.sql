-- Content store schema for atlas-contextmode artifact persistence.
-- Kept separate from worldtree.db (graph) and session.db (session events).

-- Sources: one row per indexed artifact (command output, tool result, large context).
CREATE TABLE IF NOT EXISTS sources (
    id           TEXT PRIMARY KEY,
    session_id   TEXT,
    source_type  TEXT NOT NULL,
    label        TEXT NOT NULL,
    repo_root    TEXT,
    created_at   TEXT NOT NULL
);

-- Chunks: subdivided pieces of each source for retrieval.
CREATE TABLE IF NOT EXISTS chunks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id    TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    content      TEXT NOT NULL,
    content_type TEXT NOT NULL,
    chunk_index  INTEGER NOT NULL,
    title        TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL,
    UNIQUE (source_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_chunks_source_id ON chunks(source_id);
CREATE INDEX IF NOT EXISTS idx_sources_session ON sources(session_id);

-- FTS5 index over chunk content for keyword retrieval.
-- FTS is updated manually (insert/delete/update) rather than via triggers to
-- keep the migration DDL simple and compatible with rusqlite execute_batch.
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    title,
    content,
    source_id UNINDEXED,
    content_type UNINDEXED,
    content=chunks,
    content_rowid=id
);
