-- schema_version: 1
PRAGMA user_version = 1;

-- table: edges
CREATE TABLE edges ( id INTEGER PRIMARY KEY, kind TEXT NOT NULL, source_qualified TEXT NOT NULL, target_qualified TEXT NOT NULL, file_path TEXT, line INTEGER, confidence REAL DEFAULT 1.0, confidence_tier TEXT, extra_json TEXT );

-- table: files
CREATE TABLE files ( path TEXT PRIMARY KEY, language TEXT, hash TEXT NOT NULL, size INTEGER, indexed_at TEXT NOT NULL );

-- table: metadata
CREATE TABLE metadata ( key TEXT PRIMARY KEY, value TEXT NOT NULL );

-- table: nodes
CREATE TABLE nodes ( id INTEGER PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, qualified_name TEXT NOT NULL UNIQUE, file_path TEXT NOT NULL, line_start INTEGER, line_end INTEGER, language TEXT, parent_name TEXT, params TEXT, return_type TEXT, modifiers TEXT, is_test INTEGER NOT NULL DEFAULT 0, file_hash TEXT, extra_json TEXT );

-- table: nodes_fts
CREATE VIRTUAL TABLE nodes_fts USING fts5( qualified_name, name, kind, file_path, language, params, return_type, modifiers, content='nodes', content_rowid='id' );

-- table: nodes_fts_config
CREATE TABLE 'nodes_fts_config'(k PRIMARY KEY, v) WITHOUT ROWID;

-- table: nodes_fts_data
CREATE TABLE 'nodes_fts_data'(id INTEGER PRIMARY KEY, block BLOB);

-- table: nodes_fts_docsize
CREATE TABLE 'nodes_fts_docsize'(id INTEGER PRIMARY KEY, sz BLOB);

-- table: nodes_fts_idx
CREATE TABLE 'nodes_fts_idx'(segid, term, pgno, PRIMARY KEY(segid, term)) WITHOUT ROWID;

-- index: idx_edges_file_path
CREATE INDEX idx_edges_file_path ON edges (file_path);

-- index: idx_edges_kind
CREATE INDEX idx_edges_kind ON edges (kind);

-- index: idx_edges_source
CREATE INDEX idx_edges_source ON edges (source_qualified);

-- index: idx_edges_target
CREATE INDEX idx_edges_target ON edges (target_qualified);

-- index: idx_nodes_file_path
CREATE INDEX idx_nodes_file_path ON nodes (file_path);

-- index: idx_nodes_kind
CREATE INDEX idx_nodes_kind ON nodes (kind);

-- index: idx_nodes_language
CREATE INDEX idx_nodes_language ON nodes (language);

-- index: idx_nodes_qualified_name
CREATE INDEX idx_nodes_qualified_name ON nodes (qualified_name);
