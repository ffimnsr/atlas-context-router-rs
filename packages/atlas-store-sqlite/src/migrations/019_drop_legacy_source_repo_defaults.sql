-- Migration 019: drop legacy source_repo_id defaults.
--
-- Migration 014 added source_repo_id with a database-level DEFAULT 'legacy'.
-- Every live writer now passes the stable repo identity explicitly, and
-- legacy-stamped rows are rejected outright, so the default only remains as
-- residue that can silently stamp new rows with an unsupported identity.
-- Rebuild each affected table without the default and recreate its indexes.
-- Row ids are preserved so nodes_fts external-content rowids stay valid.

-- edges -------------------------------------------------------------------
ALTER TABLE edges RENAME TO edges_v18;

CREATE TABLE edges (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    source_qualified TEXT NOT NULL,
    target_qualified TEXT NOT NULL,
    file_path TEXT,
    line INTEGER,
    confidence REAL DEFAULT 1.0,
    confidence_tier TEXT,
    extra_json TEXT,
    source_repo_id TEXT NOT NULL
);

INSERT INTO edges (
    id,
    kind,
    source_qualified,
    target_qualified,
    file_path,
    line,
    confidence,
    confidence_tier,
    extra_json,
    source_repo_id
)
SELECT
    id,
    kind,
    source_qualified,
    target_qualified,
    file_path,
    line,
    confidence,
    confidence_tier,
    extra_json,
    source_repo_id
FROM edges_v18;

DROP TABLE edges_v18;

CREATE INDEX idx_edges_file_path ON edges (file_path);
CREATE INDEX idx_edges_kind ON edges (kind);
CREATE INDEX idx_edges_source ON edges (source_qualified);
CREATE INDEX idx_edges_source_repo_file_path ON edges (source_repo_id, file_path);
CREATE INDEX idx_edges_source_repo_source ON edges (source_repo_id, source_qualified);
CREATE INDEX idx_edges_source_repo_target ON edges (source_repo_id, target_qualified);
CREATE INDEX idx_edges_target ON edges (target_qualified);

-- files -------------------------------------------------------------------
ALTER TABLE files RENAME TO files_v18;

CREATE TABLE files (
    path TEXT NOT NULL,
    language TEXT,
    hash TEXT NOT NULL,
    size INTEGER,
    indexed_at TEXT NOT NULL,
    owner_id TEXT,
    owner_kind TEXT,
    owner_root TEXT,
    owner_manifest_path TEXT,
    owner_name TEXT,
    source_repo_id TEXT NOT NULL,
    PRIMARY KEY (source_repo_id, path)
);

INSERT INTO files (
    path,
    language,
    hash,
    size,
    indexed_at,
    owner_id,
    owner_kind,
    owner_root,
    owner_manifest_path,
    owner_name,
    source_repo_id
)
SELECT
    path,
    language,
    hash,
    size,
    indexed_at,
    owner_id,
    owner_kind,
    owner_root,
    owner_manifest_path,
    owner_name,
    source_repo_id
FROM files_v18;

DROP TABLE files_v18;

CREATE INDEX idx_files_owner_id ON files (owner_id);
CREATE INDEX idx_files_source_repo_path ON files (source_repo_id, path);

-- graph_build_state -------------------------------------------------------
ALTER TABLE graph_build_state RENAME TO graph_build_state_v18;

CREATE TABLE graph_build_state (
    repo_root TEXT PRIMARY KEY,
    state TEXT NOT NULL DEFAULT 'built',
    files_discovered INTEGER NOT NULL DEFAULT 0,
    files_processed INTEGER NOT NULL DEFAULT 0,
    files_failed INTEGER NOT NULL DEFAULT 0,
    nodes_written INTEGER NOT NULL DEFAULT 0,
    edges_written INTEGER NOT NULL DEFAULT 0,
    last_built_at TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    files_accepted INTEGER NOT NULL DEFAULT 0,
    files_skipped_by_byte_budget INTEGER NOT NULL DEFAULT 0,
    bytes_accepted INTEGER NOT NULL DEFAULT 0,
    bytes_skipped INTEGER NOT NULL DEFAULT 0,
    budget_stop_reason TEXT,
    source_repo_id TEXT NOT NULL,
    recovery_mode TEXT,
    quarantine_path TEXT,
    last_indexed_ref TEXT
);

INSERT INTO graph_build_state (
    repo_root,
    state,
    files_discovered,
    files_processed,
    files_failed,
    nodes_written,
    edges_written,
    last_built_at,
    last_error,
    updated_at,
    files_accepted,
    files_skipped_by_byte_budget,
    bytes_accepted,
    bytes_skipped,
    budget_stop_reason,
    source_repo_id,
    recovery_mode,
    quarantine_path,
    last_indexed_ref
)
SELECT
    repo_root,
    state,
    files_discovered,
    files_processed,
    files_failed,
    nodes_written,
    edges_written,
    last_built_at,
    last_error,
    updated_at,
    files_accepted,
    files_skipped_by_byte_budget,
    bytes_accepted,
    bytes_skipped,
    budget_stop_reason,
    source_repo_id,
    recovery_mode,
    quarantine_path,
    last_indexed_ref
FROM graph_build_state_v18;

DROP TABLE graph_build_state_v18;

CREATE INDEX idx_graph_build_state_source_repo ON graph_build_state (source_repo_id);

-- nodes -------------------------------------------------------------------
ALTER TABLE nodes RENAME TO nodes_v18;

CREATE TABLE nodes (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    qualified_name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    line_start INTEGER,
    line_end INTEGER,
    language TEXT,
    parent_name TEXT,
    params TEXT,
    return_type TEXT,
    modifiers TEXT,
    is_test INTEGER NOT NULL DEFAULT 0,
    file_hash TEXT,
    extra_json TEXT,
    source_repo_id TEXT NOT NULL,
    UNIQUE (source_repo_id, qualified_name)
);

INSERT INTO nodes (
    id,
    kind,
    name,
    qualified_name,
    file_path,
    line_start,
    line_end,
    language,
    parent_name,
    params,
    return_type,
    modifiers,
    is_test,
    file_hash,
    extra_json,
    source_repo_id
)
SELECT
    id,
    kind,
    name,
    qualified_name,
    file_path,
    line_start,
    line_end,
    language,
    parent_name,
    params,
    return_type,
    modifiers,
    is_test,
    file_hash,
    extra_json,
    source_repo_id
FROM nodes_v18;

DROP TABLE nodes_v18;

CREATE INDEX idx_nodes_file_path ON nodes (file_path);
CREATE INDEX idx_nodes_kind ON nodes (kind);
CREATE INDEX idx_nodes_language ON nodes (language);
CREATE INDEX idx_nodes_qualified_name ON nodes (qualified_name);
CREATE INDEX idx_nodes_source_repo_file_path ON nodes (source_repo_id, file_path);
-- The (source_repo_id, qualified_name) composite UNIQUE supplies its own index;
-- the pre-existing idx_nodes_source_repo_qname is intentionally dropped.

-- postprocess_state -------------------------------------------------------
ALTER TABLE postprocess_state RENAME TO postprocess_state_v18;

CREATE TABLE postprocess_state (
    repo_root TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    mode TEXT NOT NULL,
    stage_filter TEXT,
    changed_file_count INTEGER NOT NULL DEFAULT 0,
    stages_json TEXT,
    started_at_ms INTEGER,
    finished_at_ms INTEGER,
    last_error_code TEXT,
    last_error TEXT,
    updated_at_ms INTEGER NOT NULL,
    source_repo_id TEXT NOT NULL
);

INSERT INTO postprocess_state (
    repo_root,
    state,
    mode,
    stage_filter,
    changed_file_count,
    stages_json,
    started_at_ms,
    finished_at_ms,
    last_error_code,
    last_error,
    updated_at_ms,
    source_repo_id
)
SELECT
    repo_root,
    state,
    mode,
    stage_filter,
    changed_file_count,
    stages_json,
    started_at_ms,
    finished_at_ms,
    last_error_code,
    last_error,
    updated_at_ms,
    source_repo_id
FROM postprocess_state_v18;

DROP TABLE postprocess_state_v18;

CREATE INDEX idx_postprocess_state_state ON postprocess_state (state);
CREATE INDEX idx_postprocess_state_updated_at_ms ON postprocess_state (updated_at_ms DESC);
