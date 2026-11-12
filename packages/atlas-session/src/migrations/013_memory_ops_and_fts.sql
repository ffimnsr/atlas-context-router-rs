-- VM1 — typed event–entity operators for the shared `memories` table.
--
-- `numeric_value` caches the scalar produced by numeric operators (SUM/AVG),
-- and `op_history_json` keeps a bounded deterministic audit trail of applied
-- operators. `memories_fts` exposes token-aware lexical matching over topic,
-- title, and body for the VM3 recall upgrade while keeping `memories` as the
-- single authoritative memory entity table.

ALTER TABLE memories ADD COLUMN op_history_json TEXT NOT NULL DEFAULT '[]';

ALTER TABLE memories ADD COLUMN numeric_value REAL;

CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    topic,
    title,
    body,
    content = 'memories',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

INSERT INTO memories_fts(memories_fts) VALUES('rebuild');

CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, topic, title, body)
    VALUES (new.rowid, new.topic, new.title, new.body);
END;

CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, topic, title, body)
    VALUES ('delete', old.rowid, old.topic, old.title, old.body);
END;

CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, topic, title, body)
    VALUES ('delete', old.rowid, old.topic, old.title, old.body);
    INSERT INTO memories_fts(rowid, topic, title, body)
    VALUES (new.rowid, new.topic, new.title, new.body);
END;
