use super::*;

impl SessionStore {
    /// Record one query-conditioned experience edge (VR4).
    pub fn record_experience_edge(
        &mut self,
        edge: &NewExperienceEdge,
    ) -> Result<ExperienceEdgeRecord> {
        let now = util::format_now();
        let record = experience_edges::upsert_experience_edge(&self.conn, edge, &now)?;
        markdown_records::sync_experience(&self.markdown, &self.conn)?;
        Ok(record)
    }

    /// Mark edges touched by changed or deleted paths (VR4).
    pub fn invalidate_experience_paths(
        &mut self,
        repo_root: &str,
        changed: &[(String, Option<String>)],
    ) -> Result<usize> {
        let changed =
            experience_edges::invalidate_experience_paths(&self.conn, repo_root, changed)?;
        markdown_records::sync_experience(&self.markdown, &self.conn)?;
        Ok(changed)
    }

    /// Activate only fresh experience edges above the documented thresholds.
    pub fn activate_experience_edges(
        &self,
        repo_root: &str,
        query_text: &str,
        intent: &str,
        limit: usize,
    ) -> Result<Vec<ExperienceEdgeRecord>> {
        experience_edges::activate_experience_edges(
            &self.conn, repo_root, query_text, intent, limit, None,
        )
    }

    /// Inspect edges touching one URI.
    pub fn inspect_experience_edges(
        &self,
        repo_root: &str,
        uri: &str,
        direction: ExperienceDirection,
        limit: usize,
    ) -> Result<Vec<ExperienceEdgeRecord>> {
        experience_edges::inspect_experience_edges(&self.conn, repo_root, uri, direction, limit)
    }

    /// Prune stale experience edges; `dry_run` only reports.
    pub fn prune_experience_edges(
        &mut self,
        repo_root: &str,
        older_than_days: u64,
        dry_run: bool,
    ) -> Result<usize> {
        let removed = experience_edges::prune_experience_edges(
            &self.conn,
            repo_root,
            older_than_days,
            dry_run,
        )?;
        if !dry_run {
            markdown_records::sync_experience(&self.markdown, &self.conn)?;
            markdown_records::prune_orphans(
                &self.markdown,
                &self.conn,
                "experience_edges",
                "id",
                "experience",
                "id",
            )?;
        }
        Ok(removed)
    }

    /// Schema invariants surfaced by `atlas db check`.
    pub fn experience_schema_issues(&self) -> Vec<String> {
        experience_edges::experience_schema_issues(&self.conn)
    }
}
