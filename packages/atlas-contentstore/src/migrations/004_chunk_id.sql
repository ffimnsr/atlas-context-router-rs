-- Migration 004: stable content-derived chunk identity (Patch R5).
--
-- Adds a `chunk_id` column to the chunks table. The id is a SHA-256 hex
-- string computed over (source_id + NUL + normalized content), making it
-- stable across re-indexing runs as long as the source path and content are
-- unchanged.
--
-- Existing rows receive a legacy sentinel value of the form 'legacy-<rowid>'
-- so the UNIQUE(source_id, chunk_id) constraint remains satisfiable without
-- re-computing hashes for old data.  They will be replaced with proper ids
-- on the next re-index of the corresponding source.

ALTER TABLE chunks ADD COLUMN chunk_id TEXT NOT NULL DEFAULT '';

-- Back-fill existing rows with a stable legacy placeholder.
UPDATE chunks SET chunk_id = 'legacy-' || CAST(id AS TEXT) WHERE chunk_id = '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_chunks_source_chunk_id ON chunks(source_id, chunk_id);
CREATE INDEX IF NOT EXISTS idx_chunks_chunk_id ON chunks(chunk_id);
