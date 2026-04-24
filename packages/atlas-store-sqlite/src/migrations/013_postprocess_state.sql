CREATE TABLE IF NOT EXISTS postprocess_state (
    repo_root TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    mode TEXT NOT NULL,
    stage_filter TEXT,
    changed_file_count INTEGER NOT NULL DEFAULT 0,
    stages_json TEXT,
    started_at_ms INTEGER,
    finished_at_ms INTEGER,
    last_error_code TEXT,
    last_error TEXT,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_postprocess_state_state
    ON postprocess_state (state);

CREATE INDEX IF NOT EXISTS idx_postprocess_state_updated_at_ms
    ON postprocess_state (updated_at_ms DESC);
