-- Migration 009: content-addressed historical file graph storage
-- Tables: historical_nodes, historical_edges, snapshot_nodes, snapshot_edges

-- Content-addressed node store keyed by file blob hash.
-- One row per (file_hash, qualified_name): all nodes from parsing a unique blob.
CREATE TABLE IF NOT EXISTS historical_nodes (
    file_hash      TEXT    NOT NULL,
    qualified_name TEXT    NOT NULL,
    kind           TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    file_path      TEXT    NOT NULL,
    line_start     INTEGER,
    line_end       INTEGER,
    language       TEXT,
    parent_name    TEXT,
    params         TEXT,
    return_type    TEXT,
    modifiers      TEXT,
    is_test        INTEGER NOT NULL DEFAULT 0,
    extra_json     TEXT,
    PRIMARY KEY (file_hash, qualified_name)
);

CREATE INDEX IF NOT EXISTS idx_historical_nodes_file_hash
    ON historical_nodes (file_hash);

-- Content-addressed edge store keyed by file blob hash.
-- file_path of the source file is stored for provenance only.
CREATE TABLE IF NOT EXISTS historical_edges (
    file_hash    TEXT    NOT NULL,
    source_qn    TEXT    NOT NULL,
    target_qn    TEXT    NOT NULL,
    kind         TEXT    NOT NULL,
    file_path    TEXT    NOT NULL,
    line         INTEGER,
    confidence   REAL    NOT NULL DEFAULT 1.0,
    confidence_tier TEXT,
    extra_json   TEXT,
    PRIMARY KEY (file_hash, source_qn, target_qn, kind)
);

CREATE INDEX IF NOT EXISTS idx_historical_edges_file_hash
    ON historical_edges (file_hash);

-- Snapshot node membership: which nodes are active in a given snapshot.
-- References historical_nodes via (file_hash, qualified_name).
CREATE TABLE IF NOT EXISTS snapshot_nodes (
    snapshot_id    INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE,
    file_hash      TEXT    NOT NULL,
    qualified_name TEXT    NOT NULL,
    PRIMARY KEY (snapshot_id, qualified_name)
);

CREATE INDEX IF NOT EXISTS idx_snapshot_nodes_snapshot_id
    ON snapshot_nodes (snapshot_id);
CREATE INDEX IF NOT EXISTS idx_snapshot_nodes_file_hash
    ON snapshot_nodes (file_hash);

-- Snapshot edge membership: which edges are active in a given snapshot.
-- References historical_edges via (file_hash, source_qn, target_qn, kind).
CREATE TABLE IF NOT EXISTS snapshot_edges (
    snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE,
    file_hash   TEXT    NOT NULL,
    source_qn   TEXT    NOT NULL,
    target_qn   TEXT    NOT NULL,
    kind        TEXT    NOT NULL,
    PRIMARY KEY (snapshot_id, source_qn, target_qn, kind)
);

CREATE INDEX IF NOT EXISTS idx_snapshot_edges_snapshot_id
    ON snapshot_edges (snapshot_id);
CREATE INDEX IF NOT EXISTS idx_snapshot_edges_file_hash
    ON snapshot_edges (file_hash);
