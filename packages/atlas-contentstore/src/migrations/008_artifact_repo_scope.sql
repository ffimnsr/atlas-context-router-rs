ALTER TABLE sources ADD COLUMN repo_roots_json TEXT NOT NULL DEFAULT '[]';

UPDATE sources
SET repo_roots_json = CASE
    WHEN repo_root IS NULL OR repo_root = '' THEN '[]'
    ELSE json_array(repo_root)
END
WHERE repo_roots_json = '[]';
