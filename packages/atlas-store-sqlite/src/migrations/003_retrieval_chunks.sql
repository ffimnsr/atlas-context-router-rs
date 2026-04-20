-- Migration 003: retrieval chunks for hybrid (FTS + vector) search
--
-- Each row holds the embeddable text for one symbol-sized chunk.
-- `embedding` is NULL until `atlas embed` fills it in; vector retrieval is
-- silently skipped for chunks that have no embedding yet.

CREATE TABLE IF NOT EXISTS retrieval_chunks (
    id        INTEGER PRIMARY KEY,
    node_qn   TEXT    NOT NULL,
    chunk_idx INTEGER NOT NULL DEFAULT 0,
    text      TEXT    NOT NULL,
    embedding BLOB,           -- little-endian f32 bytes; NULL until computed
    UNIQUE(node_qn, chunk_idx)
);

CREATE INDEX IF NOT EXISTS idx_chunks_node_qn      ON retrieval_chunks (node_qn);
CREATE INDEX IF NOT EXISTS idx_chunks_has_embedding ON retrieval_chunks (id)
    WHERE embedding IS NOT NULL;
