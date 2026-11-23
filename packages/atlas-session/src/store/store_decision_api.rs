use super::*;

impl SessionStore {
    pub fn search_decisions(
        &self,
        repo_root: &str,
        query: &str,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DecisionSearchHit>> {
        search_decisions(&self.conn, repo_root, session_id, query, limit)
    }

    pub fn summarize_agent_memory(
        &self,
        session_id: &SessionId,
        agent_id: Option<&str>,
        merge_agent_partitions: bool,
    ) -> Result<AgentMemorySummary> {
        summarize_agent_memory(self, session_id, agent_id, merge_agent_partitions)
    }

    pub fn summarize_agent_memory_from_events(
        &self,
        events: Vec<SessionEventRow>,
        agent_id: Option<&str>,
        merge_agent_partitions: bool,
    ) -> Result<AgentMemorySummary> {
        summarize_agent_memory_from_events(events, agent_id, merge_agent_partitions)
    }

    // ── CM11: Cross-Session Intelligence ─────────────────────────────────────
}
