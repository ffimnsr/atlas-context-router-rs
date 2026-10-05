-- Migration 020: repo-scope retrieval chunk identity.
--
-- `retrieval_chunks` was keyed by `node_qn` alone while `nodes` is keyed by
-- `(source_repo_id, qualified_name)`. In multi-repo databases two repos that
-- share an un-namespaced qname therefore shared one chunk row and clobbered
-- each other's text and embedding (last writer won). Rebuild the table with
-- `source_repo_id` in the identity because SQLite cannot alter a UNIQUE
-- constraint in place.
--
-- Backfill rules for existing rows:
--   * exactly one repo owns the qname -> attribute the row to that repo;
--   * no owner (orphan) or multiple owners (ambiguous) -> drop the row.
--     Orphans are useless and ambiguous rows cannot be attributed correctly;
--     the next build/update/embed pass regenerates them per repo.

CREATE TABLE retrieval_chunks_new (
    id             INTEGER PRIMARY KEY,
    source_repo_id TEXT    NOT NULL,
    node_qn        TEXT    NOT NULL,
    chunk_idx      INTEGER NOT NULL DEFAULT 0,
    text           TEXT    NOT NULL,
    embedding      BLOB,           -- little-endian f32 bytes; NULL until computed
    UNIQUE(source_repo_id, node_qn, chunk_idx)
);

INSERT INTO retrieval_chunks_new (id, source_repo_id, node_qn, chunk_idx, text, embedding)
SELECT rc.id,
       (SELECT MIN(n.source_repo_id) FROM nodes n WHERE n.qualified_name = rc.node_qn),
       rc.node_qn,
       rc.chunk_idx,
       rc.text,
       rc.embedding
FROM retrieval_chunks rc
WHERE (SELECT COUNT(DISTINCT n.source_repo_id)
       FROM nodes n
       WHERE n.qualified_name = rc.node_qn) = 1;

DROP TABLE retrieval_chunks;
ALTER TABLE retrieval_chunks_new RENAME TO retrieval_chunks;

CREATE INDEX idx_chunks_node_qn ON retrieval_chunks (node_qn);
CREATE INDEX idx_chunks_has_embedding ON retrieval_chunks (id)
    WHERE embedding IS NOT NULL;
