ALTER TABLE sources ADD COLUMN repo_id TEXT;
ALTER TABLE sources ADD COLUMN repo_ids_json TEXT NOT NULL DEFAULT '[]';
CREATE INDEX IF NOT EXISTS idx_sources_repo_id ON sources (repo_id);
