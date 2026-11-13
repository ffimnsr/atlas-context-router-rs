-- Migration 012: persist the verbatim artifact payload alongside its chunks.
--
-- Chunking is deliberately lossy: paragraph splitting drops blank lines, text
-- chunks are trimmed, and JSON chunks store pretty-printed values keyed by
-- property name instead of the original serialization. That is correct for
-- retrieval, but `read_saved_context` must round-trip the payload byte for
-- byte, so the raw text is stored here and served verbatim.

ALTER TABLE sources ADD COLUMN raw_content TEXT;
