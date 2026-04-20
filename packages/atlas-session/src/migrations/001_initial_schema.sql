-- Session store schema for Atlas context-mode event ledger persistence.
-- Kept separate from worldtree.db (graph) and context.db (artifact content).

CREATE TABLE IF NOT EXISTS session_meta (
    session_id          TEXT PRIMARY KEY,
    repo_root           TEXT NOT NULL,
    frontend            TEXT NOT NULL,
    worktree_id         TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    last_resume_at      TEXT,
    last_compaction_at  TEXT
);

CREATE TABLE IF NOT EXISTS session_events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL REFERENCES session_meta(session_id) ON DELETE CASCADE,
    event_type   TEXT NOT NULL,
    priority     INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    event_hash   TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    UNIQUE (session_id, event_hash)
);

CREATE INDEX IF NOT EXISTS idx_session_events_session_created
    ON session_events(session_id, created_at, id);

CREATE INDEX IF NOT EXISTS idx_session_events_session_priority_created
    ON session_events(session_id, priority, created_at, id);

CREATE TABLE IF NOT EXISTS session_resume (
    session_id   TEXT PRIMARY KEY REFERENCES session_meta(session_id) ON DELETE CASCADE,
    snapshot     TEXT NOT NULL,
    event_count  INTEGER NOT NULL,
    consumed     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
