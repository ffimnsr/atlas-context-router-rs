ALTER TABLE sources ADD COLUMN agent_id TEXT;

CREATE INDEX IF NOT EXISTS idx_sources_session_agent
    ON sources(session_id, agent_id);

CREATE INDEX IF NOT EXISTS idx_sources_repo_agent
    ON sources(repo_root, agent_id);
