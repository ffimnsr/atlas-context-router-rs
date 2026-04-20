ALTER TABLE files ADD COLUMN owner_id TEXT;
ALTER TABLE files ADD COLUMN owner_kind TEXT;
ALTER TABLE files ADD COLUMN owner_root TEXT;
ALTER TABLE files ADD COLUMN owner_manifest_path TEXT;
ALTER TABLE files ADD COLUMN owner_name TEXT;

CREATE INDEX IF NOT EXISTS idx_files_owner_id ON files (owner_id);
