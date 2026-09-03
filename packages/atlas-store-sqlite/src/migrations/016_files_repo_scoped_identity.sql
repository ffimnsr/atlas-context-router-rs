-- Migration 016: scope file identity by source repository.
--
-- Migration 014 added source_repo_id but retained the original path-only primary
-- key, causing repositories with matching relative paths to replace each
-- other's file inventory rows.

ALTER TABLE files RENAME TO files_v15;

CREATE TABLE files (
    path                TEXT NOT NULL,
    language            TEXT,
    hash                TEXT NOT NULL,
    size                INTEGER,
    indexed_at          TEXT NOT NULL,
    owner_id             TEXT,
    owner_kind           TEXT,
    owner_root           TEXT,
    owner_manifest_path  TEXT,
    owner_name           TEXT,
    source_repo_id       TEXT NOT NULL DEFAULT 'legacy',
    PRIMARY KEY (source_repo_id, path)
);

INSERT INTO files (
    path,
    language,
    hash,
    size,
    indexed_at,
    owner_id,
    owner_kind,
    owner_root,
    owner_manifest_path,
    owner_name,
    source_repo_id
)
SELECT
    path,
    language,
    hash,
    size,
    indexed_at,
    owner_id,
    owner_kind,
    owner_root,
    owner_manifest_path,
    owner_name,
    source_repo_id
FROM files_v15;

DROP TABLE files_v15;

CREATE INDEX idx_files_owner_id ON files (owner_id);
CREATE INDEX idx_files_source_repo_path ON files (source_repo_id, path);
