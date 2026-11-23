use super::*;

impl SessionStore {
    /// Record a single symbol access in the global memory layer.
    pub fn record_symbol_access(&self, repo_root: &str, symbol_qn: &str) -> Result<()> {
        let now = format_now();
        global_memory::upsert_symbol_access(&self.conn, repo_root, symbol_qn, &now)
    }

    /// Record a single file access in the global memory layer.
    pub fn record_file_access(&self, repo_root: &str, file_path: &str) -> Result<()> {
        let now = format_now();
        global_memory::upsert_file_access(&self.conn, repo_root, file_path, &now)
    }

    /// Record a workflow command sequence in the global memory layer.
    pub fn record_workflow_pattern(&self, repo_root: &str, pattern: &[String]) -> Result<()> {
        let now = format_now();
        global_memory::upsert_workflow_pattern(&self.conn, repo_root, pattern, &now)
    }

    /// Return the most-frequently accessed symbols across all sessions for `repo_root`.
    pub fn get_frequent_symbols(
        &self,
        repo_root: &str,
        limit: u32,
    ) -> Result<Vec<GlobalAccessEntry>> {
        global_memory::get_frequent_symbols(&self.conn, repo_root, limit)
    }

    /// Return the most-frequently accessed files across all sessions for `repo_root`.
    pub fn get_frequent_files(
        &self,
        repo_root: &str,
        limit: u32,
    ) -> Result<Vec<GlobalAccessEntry>> {
        global_memory::get_frequent_files(&self.conn, repo_root, limit)
    }

    /// Return recurring workflow patterns across all sessions for `repo_root`.
    pub fn get_recurring_workflows(
        &self,
        repo_root: &str,
        limit: u32,
    ) -> Result<Vec<GlobalWorkflowPattern>> {
        global_memory::get_recurring_workflows(&self.conn, repo_root, limit)
    }

    /// Find sessions that share symbols or files with the given focus lists.
    ///
    /// Returns sessions ordered by overlap score (most relevant first).
    pub fn find_relevant_sessions(
        &self,
        repo_root: &str,
        focus_symbols: &[String],
        focus_files: &[String],
        limit: u32,
    ) -> Result<Vec<SessionMeta>> {
        global_memory::find_relevant_sessions(
            &self.conn,
            repo_root,
            focus_symbols,
            focus_files,
            limit,
        )
    }
}
