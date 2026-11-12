-- Directory hierarchy for content search scope (VR2 / TrieHI DSQ stage 1).
--
-- `relative_path` is the canonical repo-relative identity for file-backed
-- sources. Non-file artifacts keep NULL and are only reachable through their
-- existing identity filters.

ALTER TABLE sources ADD COLUMN relative_path TEXT;
ALTER TABLE sources ADD COLUMN parent_dir TEXT;

-- Backfill from canonical repo-path identity only. Other identity kinds have
-- no repo-relative path and must stay NULL.
UPDATE sources
SET relative_path = identity_value
WHERE relative_path IS NULL
  AND identity_kind = 'repo_path'
  AND identity_value <> '';

UPDATE sources
SET parent_dir = CASE
    WHEN relative_path IS NULL OR relative_path = '' THEN NULL
    WHEN relative_path LIKE '%/' THEN rtrim(relative_path, '/')
    WHEN instr(relative_path, '/') = 0 THEN ''
    ELSE substr(
        rtrim(relative_path, replace(relative_path, '/', '')),
        1,
        length(rtrim(relative_path, replace(relative_path, '/', ''))) - 1
    )
END
WHERE relative_path IS NOT NULL
  AND parent_dir IS NULL;

CREATE INDEX IF NOT EXISTS idx_sources_repo_parent_dir
    ON sources (repo_root, parent_dir);
CREATE INDEX IF NOT EXISTS idx_sources_repo_relative_path
    ON sources (repo_root, relative_path);
