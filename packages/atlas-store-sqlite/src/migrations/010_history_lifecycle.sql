-- Migration 010: lifecycle rows for historical nodes and edges

CREATE TABLE IF NOT EXISTS node_history (
    repo_id                 INTEGER NOT NULL REFERENCES repos (repo_id) ON DELETE CASCADE,
    qualified_name          TEXT    NOT NULL,
    file_path               TEXT    NOT NULL,
    kind                    TEXT    NOT NULL,
    signature_hash          TEXT,
    first_snapshot_id       INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE,
    last_snapshot_id        INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE,
    first_commit_sha        TEXT    NOT NULL,
    last_commit_sha         TEXT    NOT NULL,
    introduction_commit_sha TEXT    NOT NULL,
    removal_commit_sha      TEXT,
    confidence              REAL    NOT NULL DEFAULT 1.0,
    evidence_json           TEXT,
    PRIMARY KEY (repo_id, qualified_name, file_path, kind)
);

CREATE INDEX IF NOT EXISTS idx_node_history_repo_id
    ON node_history (repo_id);
CREATE INDEX IF NOT EXISTS idx_node_history_qualified_name
    ON node_history (qualified_name);
CREATE INDEX IF NOT EXISTS idx_node_history_introduction_commit
    ON node_history (introduction_commit_sha);
CREATE INDEX IF NOT EXISTS idx_node_history_removal_commit
    ON node_history (removal_commit_sha);

CREATE TABLE IF NOT EXISTS edge_history (
    repo_id                 INTEGER NOT NULL REFERENCES repos (repo_id) ON DELETE CASCADE,
    source_qn               TEXT    NOT NULL,
    target_qn               TEXT    NOT NULL,
    kind                    TEXT    NOT NULL,
    file_path               TEXT    NOT NULL,
    metadata_hash           TEXT,
    first_snapshot_id       INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE,
    last_snapshot_id        INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE,
    first_commit_sha        TEXT    NOT NULL,
    last_commit_sha         TEXT    NOT NULL,
    introduction_commit_sha TEXT    NOT NULL,
    removal_commit_sha      TEXT,
    confidence              REAL    NOT NULL DEFAULT 1.0,
    evidence_json           TEXT,
    PRIMARY KEY (repo_id, source_qn, target_qn, kind, file_path)
);

CREATE INDEX IF NOT EXISTS idx_edge_history_repo_id
    ON edge_history (repo_id);
CREATE INDEX IF NOT EXISTS idx_edge_history_source_qn
    ON edge_history (source_qn);
CREATE INDEX IF NOT EXISTS idx_edge_history_target_qn
    ON edge_history (target_qn);
CREATE INDEX IF NOT EXISTS idx_edge_history_introduction_commit
    ON edge_history (introduction_commit_sha);
CREATE INDEX IF NOT EXISTS idx_edge_history_removal_commit
    ON edge_history (removal_commit_sha);
