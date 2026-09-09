-- Migration 018: repo-scope node identity.
--
-- The global UNIQUE(qualified_name) let one repo's node rows silently replace
-- another repo's nodes that share a relative path (un-namespaced qnames),
-- clobbering graph data and orphaning external-content FTS rows in
-- multi-repo databases. Node ids are preserved so nodes_fts rowids stay valid.

ALTER TABLE nodes RENAME TO nodes_v17;

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
    source_repo_id TEXT NOT NULL DEFAULT 'legacy',
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
FROM nodes_v17;

DROP TABLE nodes_v17;

CREATE INDEX idx_nodes_file_path ON nodes (file_path);
CREATE INDEX idx_nodes_kind ON nodes (kind);
CREATE INDEX idx_nodes_language ON nodes (language);
CREATE INDEX idx_nodes_qualified_name ON nodes (qualified_name);
CREATE INDEX idx_nodes_source_repo_file_path ON nodes (source_repo_id, file_path);
-- The (source_repo_id, qualified_name) composite UNIQUE supplies its own index;
-- the pre-existing idx_nodes_source_repo_qname is intentionally dropped.