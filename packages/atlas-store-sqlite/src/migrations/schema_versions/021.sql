-- schema_version: 21
PRAGMA user_version = 21;

-- table: commits
CREATE TABLE commits ( commit_sha TEXT NOT NULL, repo_id INTEGER NOT NULL REFERENCES repos (repo_id), parent_sha TEXT, author_name TEXT, author_email TEXT, author_time INTEGER NOT NULL, committer_time INTEGER NOT NULL, subject TEXT NOT NULL, message TEXT, indexed_at TEXT NOT NULL, indexed_ref TEXT, PRIMARY KEY (commit_sha, repo_id) );

-- table: communities
CREATE TABLE communities ( id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, algorithm TEXT, level INTEGER, parent_community_id INTEGER, extra_json TEXT, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, FOREIGN KEY(parent_community_id) REFERENCES communities(id) ON DELETE SET NULL );

-- table: community_nodes
CREATE TABLE community_nodes ( community_id INTEGER NOT NULL, node_qualified_name TEXT NOT NULL, PRIMARY KEY (community_id, node_qualified_name), FOREIGN KEY(community_id) REFERENCES communities(id) ON DELETE CASCADE );

-- table: edge_history
CREATE TABLE edge_history ( repo_id INTEGER NOT NULL REFERENCES repos (repo_id) ON DELETE CASCADE, source_qn TEXT NOT NULL, target_qn TEXT NOT NULL, kind TEXT NOT NULL, file_path TEXT NOT NULL, metadata_hash TEXT, first_snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, last_snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, first_commit_sha TEXT NOT NULL, last_commit_sha TEXT NOT NULL, introduction_commit_sha TEXT NOT NULL, removal_commit_sha TEXT, confidence REAL NOT NULL DEFAULT 1.0, evidence_json TEXT, PRIMARY KEY (repo_id, source_qn, target_qn, kind, file_path) );

-- table: edges
CREATE TABLE edges ( id INTEGER PRIMARY KEY, kind TEXT NOT NULL, source_qualified TEXT NOT NULL, target_qualified TEXT NOT NULL, file_path TEXT, line INTEGER, confidence REAL DEFAULT 1.0, confidence_tier TEXT, extra_json TEXT, source_repo_id TEXT NOT NULL );

-- table: files
CREATE TABLE files ( path TEXT NOT NULL, language TEXT, hash TEXT NOT NULL, size INTEGER, indexed_at TEXT NOT NULL, owner_id TEXT, owner_kind TEXT, owner_root TEXT, owner_manifest_path TEXT, owner_name TEXT, source_repo_id TEXT NOT NULL, PRIMARY KEY (source_repo_id, path) );

-- table: flow_memberships
CREATE TABLE "flow_memberships" ( flow_id INTEGER NOT NULL, node_qualified_name TEXT NOT NULL, position INTEGER, role TEXT, extra_json TEXT, PRIMARY KEY (flow_id, node_qualified_name), FOREIGN KEY(flow_id) REFERENCES flows(id) ON DELETE CASCADE );

-- table: flows
CREATE TABLE flows ( id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, kind TEXT, description TEXT, extra_json TEXT, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP );

-- table: graph_build_state
CREATE TABLE graph_build_state ( repo_root TEXT PRIMARY KEY, state TEXT NOT NULL DEFAULT 'built', files_discovered INTEGER NOT NULL DEFAULT 0, files_processed INTEGER NOT NULL DEFAULT 0, files_failed INTEGER NOT NULL DEFAULT 0, nodes_written INTEGER NOT NULL DEFAULT 0, edges_written INTEGER NOT NULL DEFAULT 0, last_built_at TEXT, last_error TEXT, updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')), files_accepted INTEGER NOT NULL DEFAULT 0, files_skipped_by_byte_budget INTEGER NOT NULL DEFAULT 0, bytes_accepted INTEGER NOT NULL DEFAULT 0, bytes_skipped INTEGER NOT NULL DEFAULT 0, budget_stop_reason TEXT, source_repo_id TEXT NOT NULL, recovery_mode TEXT, quarantine_path TEXT, last_indexed_ref TEXT );

-- table: graph_snapshots
CREATE TABLE graph_snapshots ( snapshot_id INTEGER PRIMARY KEY, repo_id INTEGER NOT NULL REFERENCES repos (repo_id), commit_sha TEXT NOT NULL, root_tree_hash TEXT, node_count INTEGER NOT NULL DEFAULT 0, edge_count INTEGER NOT NULL DEFAULT 0, file_count INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, completeness REAL NOT NULL DEFAULT 1.0, parse_error_count INTEGER NOT NULL DEFAULT 0, UNIQUE (repo_id, commit_sha) );

-- table: historical_edges
CREATE TABLE historical_edges ( file_hash TEXT NOT NULL, source_qn TEXT NOT NULL, target_qn TEXT NOT NULL, kind TEXT NOT NULL, file_path TEXT NOT NULL, line INTEGER, confidence REAL NOT NULL DEFAULT 1.0, confidence_tier TEXT, extra_json TEXT, PRIMARY KEY (file_hash, source_qn, target_qn, kind) );

-- table: historical_nodes
CREATE TABLE historical_nodes ( file_hash TEXT NOT NULL, qualified_name TEXT NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL, file_path TEXT NOT NULL, line_start INTEGER, line_end INTEGER, language TEXT, parent_name TEXT, params TEXT, return_type TEXT, modifiers TEXT, is_test INTEGER NOT NULL DEFAULT 0, extra_json TEXT, PRIMARY KEY (file_hash, qualified_name) );

-- table: metadata
CREATE TABLE metadata ( key TEXT PRIMARY KEY, value TEXT NOT NULL );

-- table: node_history
CREATE TABLE node_history ( repo_id INTEGER NOT NULL REFERENCES repos (repo_id) ON DELETE CASCADE, qualified_name TEXT NOT NULL, file_path TEXT NOT NULL, kind TEXT NOT NULL, signature_hash TEXT, first_snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, last_snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, first_commit_sha TEXT NOT NULL, last_commit_sha TEXT NOT NULL, introduction_commit_sha TEXT NOT NULL, removal_commit_sha TEXT, confidence REAL NOT NULL DEFAULT 1.0, evidence_json TEXT, PRIMARY KEY (repo_id, qualified_name, file_path, kind) );

-- table: nodes
CREATE TABLE nodes ( id INTEGER PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, qualified_name TEXT NOT NULL, file_path TEXT NOT NULL, line_start INTEGER, line_end INTEGER, language TEXT, parent_name TEXT, params TEXT, return_type TEXT, modifiers TEXT, is_test INTEGER NOT NULL DEFAULT 0, file_hash TEXT, extra_json TEXT, source_repo_id TEXT NOT NULL, UNIQUE (source_repo_id, qualified_name) );

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

-- table: postprocess_state
CREATE TABLE postprocess_state ( repo_root TEXT PRIMARY KEY, state TEXT NOT NULL, mode TEXT NOT NULL, stage_filter TEXT, changed_file_count INTEGER NOT NULL DEFAULT 0, stages_json TEXT, started_at_ms INTEGER, finished_at_ms INTEGER, last_error_code TEXT, last_error TEXT, updated_at_ms INTEGER NOT NULL, source_repo_id TEXT NOT NULL );

-- table: repos
CREATE TABLE repos ( repo_id INTEGER PRIMARY KEY, root_path TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL );

-- table: retrieval_chunks
CREATE TABLE "retrieval_chunks" ( id INTEGER PRIMARY KEY, source_repo_id TEXT NOT NULL, node_qn TEXT NOT NULL, chunk_idx INTEGER NOT NULL DEFAULT 0, text TEXT NOT NULL, embedding BLOB, -- little-endian f32 bytes; NULL until computed UNIQUE(source_repo_id, node_qn, chunk_idx) );

-- table: snapshot_edges
CREATE TABLE snapshot_edges ( snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, file_hash TEXT NOT NULL, source_qn TEXT NOT NULL, target_qn TEXT NOT NULL, kind TEXT NOT NULL, PRIMARY KEY (snapshot_id, source_qn, target_qn, kind) );

-- table: snapshot_files
CREATE TABLE snapshot_files ( snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id), file_path TEXT NOT NULL, file_hash TEXT NOT NULL, language TEXT, size INTEGER, PRIMARY KEY (snapshot_id, file_path) );

-- table: snapshot_membership_blobs
CREATE TABLE snapshot_membership_blobs ( snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, file_path TEXT NOT NULL, file_hash TEXT NOT NULL, node_membership TEXT NOT NULL, edge_membership TEXT NOT NULL, PRIMARY KEY (snapshot_id, file_path) );

-- table: snapshot_nodes
CREATE TABLE snapshot_nodes ( snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, file_hash TEXT NOT NULL, qualified_name TEXT NOT NULL, PRIMARY KEY (snapshot_id, qualified_name) );

-- index: idx_chunks_has_embedding
CREATE INDEX idx_chunks_has_embedding ON retrieval_chunks (id) WHERE embedding IS NOT NULL;

-- index: idx_chunks_node_qn
CREATE INDEX idx_chunks_node_qn ON retrieval_chunks (node_qn);

-- index: idx_commits_repo_author_time
CREATE INDEX idx_commits_repo_author_time ON commits (repo_id, author_time DESC, commit_sha DESC);

-- index: idx_community_nodes_node_qn
CREATE INDEX idx_community_nodes_node_qn ON community_nodes (node_qualified_name);

-- index: idx_edges_file_path
CREATE INDEX idx_edges_file_path ON edges (file_path);

-- index: idx_edges_order
CREATE INDEX idx_edges_order ON edges (source_qualified, target_qualified, kind, file_path);

-- index: idx_edges_repo_confidence_tier
CREATE INDEX idx_edges_repo_confidence_tier ON edges (source_repo_id, confidence_tier);

-- index: idx_edges_source_repo_file_path
CREATE INDEX idx_edges_source_repo_file_path ON edges (source_repo_id, file_path);

-- index: idx_edges_source_repo_source
CREATE INDEX idx_edges_source_repo_source ON edges (source_repo_id, source_qualified);

-- index: idx_edges_source_repo_target
CREATE INDEX idx_edges_source_repo_target ON edges (source_repo_id, target_qualified);

-- index: idx_edges_target_kind
CREATE INDEX idx_edges_target_kind ON edges (target_qualified, kind);

-- index: idx_files_indexed_at
CREATE INDEX idx_files_indexed_at ON files (indexed_at DESC);

-- index: idx_files_owner_id
CREATE INDEX idx_files_owner_id ON files (owner_id);

-- index: idx_files_path
CREATE INDEX idx_files_path ON files (path);

-- index: idx_flow_memberships_flow_position
CREATE INDEX idx_flow_memberships_flow_position ON flow_memberships (flow_id, position);

-- index: idx_flow_memberships_node_qualified_name
CREATE INDEX idx_flow_memberships_node_qualified_name ON flow_memberships (node_qualified_name);

-- index: idx_graph_build_state_source_repo
CREATE INDEX idx_graph_build_state_source_repo ON graph_build_state (source_repo_id);

-- index: idx_nodes_file_parent
CREATE INDEX idx_nodes_file_parent ON nodes (file_path, parent_name);

-- index: idx_nodes_file_path_nocase
CREATE INDEX idx_nodes_file_path_nocase ON nodes (file_path COLLATE NOCASE);

-- index: idx_nodes_file_path_order
CREATE INDEX idx_nodes_file_path_order ON nodes (file_path, line_start, qualified_name);

-- index: idx_nodes_kind
CREATE INDEX idx_nodes_kind ON nodes (kind);

-- index: idx_nodes_language
CREATE INDEX idx_nodes_language ON nodes (language);

-- index: idx_nodes_name_nocase
CREATE INDEX idx_nodes_name_nocase ON nodes (name COLLATE NOCASE);

-- index: idx_nodes_name_order
CREATE INDEX idx_nodes_name_order ON nodes (name, file_path, line_start);

-- index: idx_nodes_qualified_name
CREATE INDEX idx_nodes_qualified_name ON nodes (qualified_name);

-- index: idx_nodes_repo_file_path_nocase
CREATE INDEX idx_nodes_repo_file_path_nocase ON nodes (source_repo_id, file_path COLLATE NOCASE);

-- index: idx_nodes_repo_file_path_order
CREATE INDEX idx_nodes_repo_file_path_order ON nodes (source_repo_id, file_path, line_start, qualified_name);

-- index: idx_retrieval_chunks_repo_embedded
CREATE INDEX idx_retrieval_chunks_repo_embedded ON retrieval_chunks (source_repo_id) WHERE embedding IS NOT NULL;

-- index: idx_snapshot_files_file_hash
CREATE INDEX idx_snapshot_files_file_hash ON snapshot_files (file_hash);
