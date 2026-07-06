CREATE TABLE IF NOT EXISTS durable_tasks (
    task_id              TEXT PRIMARY KEY,
    originating_method   TEXT NOT NULL,
    request_id           TEXT,
    tool_name            TEXT,
    transport_kind       TEXT,
    session_id           TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL,
    status               TEXT NOT NULL,
    status_message       TEXT,
    progress_json        TEXT,
    result_json          TEXT,
    error_json           TEXT,
    ttl_ms               INTEGER,
    cancel_requested     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_durable_tasks_updated_task
    ON durable_tasks(updated_at DESC, task_id DESC);

CREATE INDEX IF NOT EXISTS idx_durable_tasks_status_updated
    ON durable_tasks(status, updated_at DESC, task_id DESC);
