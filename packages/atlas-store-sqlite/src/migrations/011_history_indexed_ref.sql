-- Migration 011: track source branch/ref used during history indexing.

ALTER TABLE commits ADD COLUMN indexed_ref TEXT;

CREATE INDEX IF NOT EXISTS idx_commits_indexed_ref
    ON commits (indexed_ref);