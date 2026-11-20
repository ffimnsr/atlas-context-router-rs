-- M1 follow-up — file identity for markdown-to-index reconciliation.
--
-- Recall reads the derived index. These columns let `ensure_index` detect a
-- hand-edited or `git checkout`-refreshed memory file with one stat per file and
-- rebuild only the affected rows, without reading every body on every lookup.

ALTER TABLE memory_index ADD COLUMN file_mtime_ns INTEGER;
ALTER TABLE memory_index ADD COLUMN file_size INTEGER;
