CREATE VIRTUAL TABLE IF NOT EXISTS decision_memory_fts USING fts5(
    decision_id UNINDEXED,
    summary,
    rationale,
    conclusion,
    query_text,
    related_files,
    related_symbols,
    source_ids,
    tokenize = 'unicode61'
);

DELETE FROM decision_memory_fts;

INSERT INTO decision_memory_fts(
    decision_id,
    summary,
    rationale,
    conclusion,
    query_text,
    related_files,
    related_symbols,
    source_ids
)
SELECT
    decision_id,
    summary,
    COALESCE(rationale, ''),
    COALESCE(conclusion, ''),
    COALESCE(query_text, ''),
    related_files_json,
    related_symbols_json,
    source_ids_json
FROM decision_memory;

CREATE TRIGGER IF NOT EXISTS decision_memory_ai AFTER INSERT ON decision_memory BEGIN
    DELETE FROM decision_memory_fts WHERE decision_id = NEW.decision_id;
    INSERT INTO decision_memory_fts(
        decision_id,
        summary,
        rationale,
        conclusion,
        query_text,
        related_files,
        related_symbols,
        source_ids
    ) VALUES (
        NEW.decision_id,
        NEW.summary,
        COALESCE(NEW.rationale, ''),
        COALESCE(NEW.conclusion, ''),
        COALESCE(NEW.query_text, ''),
        NEW.related_files_json,
        NEW.related_symbols_json,
        NEW.source_ids_json
    );
END;

CREATE TRIGGER IF NOT EXISTS decision_memory_au AFTER UPDATE ON decision_memory BEGIN
    DELETE FROM decision_memory_fts WHERE decision_id = OLD.decision_id;
    INSERT INTO decision_memory_fts(
        decision_id,
        summary,
        rationale,
        conclusion,
        query_text,
        related_files,
        related_symbols,
        source_ids
    ) VALUES (
        NEW.decision_id,
        NEW.summary,
        COALESCE(NEW.rationale, ''),
        COALESCE(NEW.conclusion, ''),
        COALESCE(NEW.query_text, ''),
        NEW.related_files_json,
        NEW.related_symbols_json,
        NEW.source_ids_json
    );
END;

CREATE TRIGGER IF NOT EXISTS decision_memory_ad AFTER DELETE ON decision_memory BEGIN
    DELETE FROM decision_memory_fts WHERE decision_id = OLD.decision_id;
END;
