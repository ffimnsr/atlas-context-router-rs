-- Migration 002: trigram FTS5 table and vocabulary for typo-tolerant retrieval.

-- Trigram FTS5 table: enables substring and fuzzy-tolerant search.
-- The trigram tokenizer breaks content into 3-character n-grams so that
-- partial matches and minor typos in query terms still hit relevant chunks.
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_trigram USING fts5(
    title,
    content,
    source_id UNINDEXED,
    content_type UNINDEXED,
    content=chunks,
    content_rowid=id,
    tokenize='trigram'
);

-- Vocabulary table: accumulates indexed terms for bounded fuzzy correction
-- and term suggestions.  term_hash is a pre-computed 8-char hex prefix of
-- the SHA-256 of the lowercased term, used for fast lookup by prefix.
CREATE TABLE IF NOT EXISTS vocabulary (
    term       TEXT PRIMARY KEY,
    doc_freq   INTEGER NOT NULL DEFAULT 1,
    term_hash  TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_vocabulary_hash ON vocabulary(term_hash);
