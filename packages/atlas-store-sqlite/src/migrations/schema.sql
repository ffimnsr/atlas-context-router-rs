-- schema_version: 13
PRAGMA user_version = 13;

-- table: atlas_provenance
CREATE TABLE atlas_provenance ( singleton_key INTEGER PRIMARY KEY CHECK(singleton_key = 1), db_kind TEXT NOT NULL, created_by TEXT NOT NULL, created_at TEXT NOT NULL, last_opened_by TEXT NOT NULL, last_opened_at TEXT NOT NULL );

-- table: commits
CREATE TABLE commits ( commit_sha TEXT NOT NULL, repo_id INTEGER NOT NULL REFERENCES repos (repo_id), parent_sha TEXT, author_name TEXT, author_email TEXT, author_time INTEGER NOT NULL, committer_time INTEGER NOT NULL, subject TEXT NOT NULL, message TEXT, indexed_at TEXT NOT NULL, indexed_ref TEXT, PRIMARY KEY (commit_sha, repo_id) );

-- table: communities
CREATE TABLE communities ( id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, algorithm TEXT, level INTEGER, parent_community_id INTEGER, extra_json TEXT, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, FOREIGN KEY(parent_community_id) REFERENCES communities(id) ON DELETE SET NULL );

-- table: community_nodes
CREATE TABLE community_nodes ( community_id INTEGER NOT NULL, node_qualified_name TEXT NOT NULL, PRIMARY KEY (community_id, node_qualified_name), FOREIGN KEY(community_id) REFERENCES communities(id) ON DELETE CASCADE );

-- table: edge_history
CREATE TABLE edge_history ( repo_id INTEGER NOT NULL REFERENCES repos (repo_id) ON DELETE CASCADE, source_qn TEXT NOT NULL, target_qn TEXT NOT NULL, kind TEXT NOT NULL, file_path TEXT NOT NULL, metadata_hash TEXT, first_snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, last_snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id) ON DELETE CASCADE, first_commit_sha TEXT NOT NULL, last_commit_sha TEXT NOT NULL, introduction_commit_sha TEXT NOT NULL, removal_commit_sha TEXT, confidence REAL NOT NULL DEFAULT 1.0, evidence_json TEXT, PRIMARY KEY (repo_id, source_qn, target_qn, kind, file_path) );

-- table: edges
CREATE TABLE edges ( id INTEGER PRIMARY KEY, kind TEXT NOT NULL, source_qualified TEXT NOT NULL, target_qualified TEXT NOT NULL, file_path TEXT, line INTEGER, confidence REAL DEFAULT 1.0, confidence_tier TEXT, extra_json TEXT );

-- table: files
CREATE TABLE files ( path TEXT PRIMARY KEY, language TEXT, hash TEXT NOT NULL, size INTEGER, indexed_at TEXT NOT NULL , owner_id TEXT, owner_kind TEXT, owner_root TEXT, owner_manifest_path TEXT, owner_name TEXT);

-- table: flow_memberships
CREATE TABLE "flow_memberships" ( flow_id INTEGER NOT NULL, node_qualified_name TEXT NOT NULL, position INTEGER, role TEXT, extra_json TEXT, PRIMARY KEY (flow_id, node_qualified_name), FOREIGN KEY(flow_id) REFERENCES flows(id) ON DELETE CASCADE );

-- table: flows
CREATE TABLE flows ( id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, kind TEXT, description TEXT, extra_json TEXT, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP );

-- table: graph_build_state
CREATE TABLE graph_build_state ( repo_root TEXT PRIMARY KEY, state TEXT NOT NULL DEFAULT 'built', files_discovered INTEGER NOT NULL DEFAULT 0, files_processed INTEGER NOT NULL DEFAULT 0, files_failed INTEGER NOT NULL DEFAULT 0, nodes_written INTEGER NOT NULL DEFAULT 0, edges_written INTEGER NOT NULL DEFAULT 0, last_built_at TEXT, last_error TEXT, updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')) , files_accepted INTEGER NOT NULL DEFAULT 0, files_skipped_by_byte_budget INTEGER NOT NULL DEFAULT 0, bytes_accepted INTEGER NOT NULL DEFAULT 0, bytes_skipped INTEGER NOT NULL DEFAULT 0, budget_stop_reason TEXT);

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

-- table: postprocess_state
CREATE TABLE postprocess_state ( repo_root TEXT PRIMARY KEY, state TEXT NOT NULL, mode TEXT NOT NULL, stage_filter TEXT, changed_file_count INTEGER NOT NULL DEFAULT 0, stages_json TEXT, started_at_ms INTEGER, finished_at_ms INTEGER, last_error_code TEXT, last_error TEXT, updated_at_ms INTEGER NOT NULL );

-- table: repos
CREATE TABLE repos ( repo_id INTEGER PRIMARY KEY, root_path TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL );

-- table: retrieval_chunks
CREATE TABLE retrieval_chunks ( id INTEGER PRIMARY KEY, node_qn TEXT NOT NULL, chunk_idx INTEGER NOT NULL DEFAULT 0, text TEXT NOT NULL, embedding BLOB, -- little-endian f32 bytes; NULL until computed UNIQUE(node_qn, chunk_idx) );

-- table: schema_migrations
CREATE TABLE schema_migrations ( id INTEGER PRIMARY KEY, version INTEGER NOT NULL, name TEXT NOT NULL, direction TEXT NOT NULL CHECK(direction IN ('up', 'down')), atlas_version TEXT NOT NULL, applied_at TEXT NOT NULL );

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

-- index: idx_commits_author_time
CREATE INDEX idx_commits_author_time ON commits (author_time);

-- index: idx_commits_committer_time
CREATE INDEX idx_commits_committer_time ON commits (committer_time);

-- index: idx_commits_indexed_ref
CREATE INDEX idx_commits_indexed_ref ON commits (indexed_ref);

-- index: idx_commits_repo_id
CREATE INDEX idx_commits_repo_id ON commits (repo_id);

-- index: idx_communities_algorithm
CREATE INDEX idx_communities_algorithm ON communities (algorithm);

-- index: idx_communities_parent
CREATE INDEX idx_communities_parent ON communities (parent_community_id);

-- index: idx_community_nodes_node_qn
CREATE INDEX idx_community_nodes_node_qn ON community_nodes (node_qualified_name);

-- index: idx_edge_history_introduction_commit
CREATE INDEX idx_edge_history_introduction_commit ON edge_history (introduction_commit_sha);

-- index: idx_edge_history_removal_commit
CREATE INDEX idx_edge_history_removal_commit ON edge_history (removal_commit_sha);

-- index: idx_edge_history_repo_id
CREATE INDEX idx_edge_history_repo_id ON edge_history (repo_id);

-- index: idx_edge_history_source_qn
CREATE INDEX idx_edge_history_source_qn ON edge_history (source_qn);

-- index: idx_edge_history_target_qn
CREATE INDEX idx_edge_history_target_qn ON edge_history (target_qn);

-- index: idx_edges_file_path
CREATE INDEX idx_edges_file_path ON edges (file_path);

-- index: idx_edges_kind
CREATE INDEX idx_edges_kind ON edges (kind);

-- index: idx_edges_source
CREATE INDEX idx_edges_source ON edges (source_qualified);

-- index: idx_edges_target
CREATE INDEX idx_edges_target ON edges (target_qualified);

-- index: idx_files_owner_id
CREATE INDEX idx_files_owner_id ON files (owner_id);

-- index: idx_flow_memberships_flow_position
CREATE INDEX idx_flow_memberships_flow_position ON flow_memberships (flow_id, position);

-- index: idx_flow_memberships_node_qualified_name
CREATE INDEX idx_flow_memberships_node_qualified_name ON flow_memberships (node_qualified_name);

-- index: idx_flows_kind
CREATE INDEX idx_flows_kind ON flows (kind);

-- index: idx_graph_snapshots_commit_sha
CREATE INDEX idx_graph_snapshots_commit_sha ON graph_snapshots (commit_sha);

-- index: idx_graph_snapshots_repo_id
CREATE INDEX idx_graph_snapshots_repo_id ON graph_snapshots (repo_id);

-- index: idx_historical_edges_file_hash
CREATE INDEX idx_historical_edges_file_hash ON historical_edges (file_hash);

-- index: idx_historical_nodes_file_hash
CREATE INDEX idx_historical_nodes_file_hash ON historical_nodes (file_hash);

-- index: idx_node_history_introduction_commit
CREATE INDEX idx_node_history_introduction_commit ON node_history (introduction_commit_sha);

-- index: idx_node_history_qualified_name
CREATE INDEX idx_node_history_qualified_name ON node_history (qualified_name);

-- index: idx_node_history_removal_commit
CREATE INDEX idx_node_history_removal_commit ON node_history (removal_commit_sha);

-- index: idx_node_history_repo_id
CREATE INDEX idx_node_history_repo_id ON node_history (repo_id);

-- index: idx_nodes_file_path
CREATE INDEX idx_nodes_file_path ON nodes (file_path);

-- index: idx_nodes_kind
CREATE INDEX idx_nodes_kind ON nodes (kind);

-- index: idx_nodes_language
CREATE INDEX idx_nodes_language ON nodes (language);

-- index: idx_nodes_qualified_name
CREATE INDEX idx_nodes_qualified_name ON nodes (qualified_name);

-- index: idx_postprocess_state_state
CREATE INDEX idx_postprocess_state_state ON postprocess_state (state);

-- index: idx_postprocess_state_updated_at_ms
CREATE INDEX idx_postprocess_state_updated_at_ms ON postprocess_state (updated_at_ms DESC);

-- index: idx_repos_root_path
CREATE INDEX idx_repos_root_path ON repos (root_path);

-- index: idx_schema_migrations_version_id
CREATE INDEX idx_schema_migrations_version_id ON schema_migrations(version, id);

-- index: idx_snapshot_edges_file_hash
CREATE INDEX idx_snapshot_edges_file_hash ON snapshot_edges (file_hash);

-- index: idx_snapshot_edges_snapshot_id
CREATE INDEX idx_snapshot_edges_snapshot_id ON snapshot_edges (snapshot_id);

-- index: idx_snapshot_files_file_hash
CREATE INDEX idx_snapshot_files_file_hash ON snapshot_files (file_hash);

-- index: idx_snapshot_files_snapshot_id
CREATE INDEX idx_snapshot_files_snapshot_id ON snapshot_files (snapshot_id);

-- index: idx_snapshot_membership_blobs_file_hash
CREATE INDEX idx_snapshot_membership_blobs_file_hash ON snapshot_membership_blobs (file_hash);

-- index: idx_snapshot_membership_blobs_snapshot_id
CREATE INDEX idx_snapshot_membership_blobs_snapshot_id ON snapshot_membership_blobs (snapshot_id);

-- index: idx_snapshot_nodes_file_hash
CREATE INDEX idx_snapshot_nodes_file_hash ON snapshot_nodes (file_hash);

-- index: idx_snapshot_nodes_snapshot_id
CREATE INDEX idx_snapshot_nodes_snapshot_id ON snapshot_nodes (snapshot_id);

