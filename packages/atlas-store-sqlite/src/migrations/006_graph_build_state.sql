-- Graph build lifecycle state table.
-- Tracks per-repo build state: building / built / build_failed.
-- Mirrors Patch R1 index state pattern for consistency across all three stores.
CREATE TABLE IF NOT EXISTS graph_build_state (
    repo_root         TEXT    PRIMARY KEY,
    state             TEXT    NOT NULL DEFAULT 'built',
    files_discovered  INTEGER NOT NULL DEFAULT 0,
    files_processed   INTEGER NOT NULL DEFAULT 0,
    files_failed      INTEGER NOT NULL DEFAULT 0,
    nodes_written     INTEGER NOT NULL DEFAULT 0,
    edges_written     INTEGER NOT NULL DEFAULT 0,
    last_built_at     TEXT,
    last_error        TEXT,
    updated_at        TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
