-- Migration 012: compact snapshot membership blobs for path-scoped materialization.

CREATE TABLE IF NOT EXISTS snapshot_membership_blobs (
    snapshot_id      INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE,
    file_path        TEXT    NOT NULL,
    file_hash        TEXT    NOT NULL,
    node_membership  TEXT    NOT NULL,
    edge_membership  TEXT    NOT NULL,
    PRIMARY KEY (snapshot_id, file_path)
);

CREATE INDEX IF NOT EXISTS idx_snapshot_membership_blobs_snapshot_id
    ON snapshot_membership_blobs (snapshot_id);

CREATE INDEX IF NOT EXISTS idx_snapshot_membership_blobs_file_hash
    ON snapshot_membership_blobs (file_hash);