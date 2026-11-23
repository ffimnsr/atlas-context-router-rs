use super::*;

impl SessionStore {
    /// Validates the feedback schema (`feedback_records` table, columns,
    /// indexes, and FTS table). Returns an empty list when healthy; used by
    /// `atlas db check`.
    pub fn feedback_schema_issues(&self) -> Vec<String> {
        feedback_schema_issues(&self.conn)
    }

    /// Validate and persist a new feedback record, returning the stored record.
    pub fn store_feedback(&mut self, input: &NewFeedback) -> Result<FeedbackRecord> {
        let record = store_feedback(&self.conn, input)?;
        markdown_records::sync_feedback(&self.markdown, &self.conn)?;
        Ok(record)
    }

    /// Lexical feedback search with exact-match filters; falls back to a
    /// deterministic LIKE scan when FTS is unavailable.
    pub fn search_feedback(
        &self,
        repo_root: &str,
        query: &str,
        filter: &FeedbackSearchFilter,
        limit: usize,
    ) -> Result<Vec<FeedbackSearchHit>> {
        search_feedback(&self.conn, repo_root, query, filter, limit)
    }

    /// Deterministic feedback statistics; stable zero-counts on an empty table.
    pub fn feedback_stats(&self, repo_root: &str) -> Result<FeedbackStats> {
        feedback_stats(&self.conn, repo_root)
    }

    /// Most recent feedback records for a repo, newest first (ICM-D wake-up).
    pub fn recent_feedback(&self, repo_root: &str, limit: usize) -> Result<Vec<FeedbackRecord>> {
        recent_feedback(&self.conn, repo_root, limit)
    }

    /// Records that can serve as false-positive evidence for a symbol/file/
    /// kind; used by the CLI confidence adjuster (ICM-C3).
    pub fn feedback_matching(
        &self,
        repo_root: &str,
        analysis_kind: &str,
        symbol: Option<&str>,
        file: Option<&str>,
    ) -> Result<Vec<FeedbackRecord>> {
        feedback_matching(&self.conn, repo_root, analysis_kind, symbol, file)
    }
}
