ALTER TABLE sources ADD COLUMN identity_kind TEXT NOT NULL DEFAULT 'artifact_label';
ALTER TABLE sources ADD COLUMN identity_value TEXT NOT NULL DEFAULT '';

UPDATE sources
SET identity_value = label
WHERE identity_value = '';

CREATE INDEX IF NOT EXISTS idx_sources_identity_kind_value
ON sources(identity_kind, identity_value);
