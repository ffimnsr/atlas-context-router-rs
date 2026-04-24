CREATE TABLE IF NOT EXISTS decision_memory (
    decision_id         TEXT PRIMARY KEY,
    session_id          TEXT NOT NULL REFERENCES session_meta(session_id) ON DELETE CASCADE,
    event_id            INTEGER REFERENCES session_events(id) ON DELETE SET NULL,
    repo_root           TEXT NOT NULL,
    summary             TEXT NOT NULL,
    rationale           TEXT,
    conclusion          TEXT,
    query_text          TEXT,
    source_ids_json     TEXT NOT NULL DEFAULT '[]',
    evidence_json       TEXT NOT NULL DEFAULT '[]',
    related_files_json  TEXT NOT NULL DEFAULT '[]',
    related_symbols_json TEXT NOT NULL DEFAULT '[]',
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_decision_memory_repo_updated
    ON decision_memory(repo_root, updated_at DESC, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_decision_memory_session_updated
    ON decision_memory(session_id, updated_at DESC, created_at DESC);
