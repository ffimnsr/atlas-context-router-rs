-- ICM-C — feedback_records: first-class deterministic correction memory.
--
-- Records predicted vs actual analysis outcomes plus correction text and
-- related symbol/file so prior mistakes can inform future confidence.
-- `feedback_records_fts` keeps a standalone FTS5 index over the searchable
-- text fields, synced by triggers.

CREATE TABLE feedback_records (
    id TEXT PRIMARY KEY,
    repo_root TEXT NOT NULL,
    session_id TEXT,
    tool_name TEXT NOT NULL DEFAULT '',
    analysis_kind TEXT NOT NULL DEFAULT '',
    predicted TEXT NOT NULL,
    actual TEXT NOT NULL,
    correction TEXT NOT NULL DEFAULT '',
    related_symbol TEXT,
    related_file TEXT,
    source_id TEXT,
    created_at TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_feedback_repo_kind ON feedback_records(repo_root, analysis_kind);
CREATE INDEX idx_feedback_repo_symbol ON feedback_records(repo_root, related_symbol);
CREATE INDEX idx_feedback_repo_file ON feedback_records(repo_root, related_file);

CREATE VIRTUAL TABLE feedback_records_fts USING fts5(
    predicted,
    actual,
    correction,
    related_symbol,
    related_file,
    tokenize = 'unicode61'
);

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
