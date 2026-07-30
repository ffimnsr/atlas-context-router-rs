-- Migration 014: multi-repo graph identity/provenance columns.

ALTER TABLE files ADD COLUMN source_repo_id TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE nodes ADD COLUMN source_repo_id TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE edges ADD COLUMN source_repo_id TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE graph_build_state ADD COLUMN source_repo_id TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE postprocess_state ADD COLUMN source_repo_id TEXT NOT NULL DEFAULT 'legacy';

CREATE INDEX IF NOT EXISTS idx_files_source_repo_path ON files (source_repo_id, path);
CREATE INDEX IF NOT EXISTS idx_nodes_source_repo_file_path ON nodes (source_repo_id, file_path);
CREATE INDEX IF NOT EXISTS idx_nodes_source_repo_qname ON nodes (source_repo_id, qualified_name);
CREATE INDEX IF NOT EXISTS idx_edges_source_repo_file_path ON edges (source_repo_id, file_path);
CREATE INDEX IF NOT EXISTS idx_edges_source_repo_source ON edges (source_repo_id, source_qualified);
CREATE INDEX IF NOT EXISTS idx_edges_source_repo_target ON edges (source_repo_id, target_qualified);
CREATE INDEX IF NOT EXISTS idx_graph_build_state_source_repo ON graph_build_state (source_repo_id);
