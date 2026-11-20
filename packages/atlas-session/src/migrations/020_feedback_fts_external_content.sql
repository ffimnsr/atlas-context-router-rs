-- Fix 010: `feedback_records_fts` was created as a regular (contentful) FTS5
-- table, but the `feedback_records_ad` and `feedback_records_au` triggers use
-- the FTS5 'delete' command form. That command is only valid on contentless
-- or external-content FTS5 tables; on a regular table it raises a generic
-- SQLITE_ERROR ("SQL logic error"), so every UPDATE or DELETE on
-- `feedback_records` failed. The write path only ever INSERTed, so the bug
-- stayed dormant until the markdown import began upserting existing rows on
-- every store open.
--
-- Recreate the index as external-content over `feedback_records` (the same
-- shape as `memories_fts` in 013), rebuild it from the content table, and
-- restore the original 010 trigger bodies, which are correct for
-- external-content tables.

DROP TRIGGER IF EXISTS feedback_records_ai;
DROP TRIGGER IF EXISTS feedback_records_ad;
DROP TRIGGER IF EXISTS feedback_records_au;
DROP TABLE IF EXISTS feedback_records_fts;

CREATE VIRTUAL TABLE feedback_records_fts USING fts5(
    predicted,
    actual,
    correction,
    related_symbol,
    related_file,
    content = 'feedback_records',
    content_rowid = 'rowid',
    tokenize = 'unicode61'
);

INSERT INTO feedback_records_fts(feedback_records_fts) VALUES('rebuild');

CREATE TRIGGER feedback_records_ai AFTER INSERT ON feedback_records BEGIN
    INSERT INTO feedback_records_fts(rowid, predicted, actual, correction, related_symbol, related_file)
    VALUES (NEW.rowid, NEW.predicted, NEW.actual, NEW.correction,
            COALESCE(NEW.related_symbol, ''), COALESCE(NEW.related_file, ''));
END;

CREATE TRIGGER feedback_records_ad AFTER DELETE ON feedback_records BEGIN
    INSERT INTO feedback_records_fts(feedback_records_fts, rowid, predicted, actual, correction, related_symbol, related_file)
    VALUES ('delete', OLD.rowid, OLD.predicted, OLD.actual, OLD.correction,
            COALESCE(OLD.related_symbol, ''), COALESCE(OLD.related_file, ''));
END;

CREATE TRIGGER feedback_records_au AFTER UPDATE ON feedback_records BEGIN
    INSERT INTO feedback_records_fts(feedback_records_fts, rowid, predicted, actual, correction, related_symbol, related_file)
    VALUES ('delete', OLD.rowid, OLD.predicted, OLD.actual, OLD.correction,
            COALESCE(OLD.related_symbol, ''), COALESCE(OLD.related_file, ''));
    INSERT INTO feedback_records_fts(rowid, predicted, actual, correction, related_symbol, related_file)
    VALUES (NEW.rowid, NEW.predicted, NEW.actual, NEW.correction,
            COALESCE(NEW.related_symbol, ''), COALESCE(NEW.related_file, ''));
END;
