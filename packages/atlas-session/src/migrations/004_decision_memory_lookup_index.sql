CREATE INDEX IF NOT EXISTS idx_decision_memory_repo_session_updated
    ON decision_memory(repo_root, session_id, updated_at DESC, created_at DESC);
