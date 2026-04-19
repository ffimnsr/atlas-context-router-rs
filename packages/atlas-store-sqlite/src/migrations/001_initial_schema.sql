-- Migration 001: initial atlas graph schema

CREATE TABLE IF NOT EXISTS metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS files (
    path        TEXT PRIMARY KEY,
    language    TEXT,
    hash        TEXT NOT NULL,
    size        INTEGER,
    indexed_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS nodes (
    id             INTEGER PRIMARY KEY,
    kind           TEXT    NOT NULL,
    name           TEXT    NOT NULL,
    qualified_name TEXT    NOT NULL UNIQUE,
    file_path      TEXT    NOT NULL,
    line_start     INTEGER,
    line_end       INTEGER,
    language       TEXT,
    parent_name    TEXT,
    params         TEXT,
    return_type    TEXT,
    modifiers      TEXT,
    is_test        INTEGER NOT NULL DEFAULT 0,
    file_hash      TEXT,
    extra_json     TEXT
);

CREATE INDEX IF NOT EXISTS idx_nodes_kind           ON nodes (kind);
CREATE INDEX IF NOT EXISTS idx_nodes_file_path      ON nodes (file_path);
CREATE INDEX IF NOT EXISTS idx_nodes_qualified_name ON nodes (qualified_name);
CREATE INDEX IF NOT EXISTS idx_nodes_language       ON nodes (language);

CREATE TABLE IF NOT EXISTS edges (
    id               INTEGER PRIMARY KEY,
    kind             TEXT    NOT NULL,
    source_qualified TEXT    NOT NULL,
    target_qualified TEXT    NOT NULL,
    file_path        TEXT,
    line             INTEGER,
    confidence       REAL    DEFAULT 1.0,
    confidence_tier  TEXT,
    extra_json       TEXT
);

CREATE INDEX IF NOT EXISTS idx_edges_kind      ON edges (kind);
CREATE INDEX IF NOT EXISTS idx_edges_source    ON edges (source_qualified);
CREATE INDEX IF NOT EXISTS idx_edges_target    ON edges (target_qualified);
CREATE INDEX IF NOT EXISTS idx_edges_file_path ON edges (file_path);

-- FTS5 virtual table for full-text search over node symbols
CREATE VIRTUAL TABLE IF NOT EXISTS nodes_fts USING fts5(
    qualified_name,
    name,
    kind,
    file_path,
    language,
    params,
    return_type,
    modifiers,
    content='nodes',
    content_rowid='id'
);
