use super::*;

impl SessionStore {
    /// Validate and persist a new memory, returning the stored record.
    pub fn store_memory(&mut self, input: &NewMemory) -> Result<MemoryRecord> {
        markdown_memory::store_memory(&self.markdown, &self.conn, input)
    }

    /// Lexical recall; exact topic matches rank above broad text matches.
    /// Visibility rules (ICM-A3) are enforced for `viewer`; `shared_only`
    /// restricts results to `project` and `global` scopes.
    pub fn recall_memories(
        &self,
        repo_root: &str,
        query: &str,
        filter: &MemoryListFilter,
        shared_only: bool,
        viewer: &MemoryViewer,
        limit: usize,
    ) -> Result<Vec<MemorySearchHit>> {
        recall_memories(
            &self.markdown,
            &self.conn,
            repo_root,
            query,
            filter,
            shared_only,
            viewer,
            limit,
        )
    }

    /// Two-path quota recall using an explicit `[memory.recall]` policy.
    #[allow(clippy::too_many_arguments)]
    pub fn recall_memories_with_policy(
        &self,
        repo_root: &str,
        query: &str,
        filter: &MemoryListFilter,
        shared_only: bool,
        viewer: &MemoryViewer,
        limit: usize,
        policy: &MemoryRecallPolicy,
    ) -> Result<Vec<MemorySearchHit>> {
        recall_memories_with_policy(
            &self.markdown,
            &self.conn,
            repo_root,
            query,
            filter,
            shared_only,
            viewer,
            limit,
            policy,
        )
    }

    /// Fetch one memory row by exact id within a repo.
    pub fn get_memory(&self, repo_root: &str, memory_id: &str) -> Result<Option<MemoryRecord>> {
        markdown_memory::get_memory(&self.markdown, &self.conn, repo_root, memory_id)
    }

    /// Export pending derived-index rows to markdown truth files. With
    /// `dry_run` only the pending paths are reported; with `finalize` the
    /// pre-migration backup and legacy staging table are removed.
    pub fn migrate_markdown(
        &mut self,
        dry_run: bool,
        finalize: bool,
    ) -> Result<markdown_memory::MarkdownMigrationReport> {
        if finalize {
            return markdown_memory::finalize_migration(&self.db_path, &self.conn);
        }
        // Snapshot the pre-migration database before any markdown import or
        // export writes. Importing hand-edited markdown first would make the
        // backup no longer describe the legacy state.
        let pending_before = if dry_run {
            0
        } else {
            markdown_memory::pending_export_count(&self.conn)?
        };
        let backup = if !dry_run && pending_before > 0 {
            Some(markdown_memory::write_migration_backup(&self.db_path)?)
        } else {
            None
        };
        if !dry_run {
            // Respect edits made directly to markdown before regenerating any
            // legacy rows; dry-run stays read-only.
            let _ = markdown_import::import_all(&self.markdown, &self.conn);
        }
        let pending = markdown_memory::export_pending(&self.markdown, &self.conn, dry_run)?;
        let records_written = if dry_run {
            0
        } else {
            markdown_records::sync_all(&self.markdown, &self.conn)?
        };
        Ok(markdown_memory::MarkdownMigrationReport {
            dry_run,
            written: pending
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            pending: markdown_memory::pending_export_count(&self.conn)?,
            backup,
            records_written,
        })
    }

    /// Rewrite a memory record in the markdown truth store and refresh its
    /// derived index row. Used by tooling and tests that need deterministic
    /// timestamps or hand-edited fields.
    pub fn rewrite_memory(&mut self, record: &MemoryRecord) -> Result<()> {
        markdown_memory::write_existing_record(&self.markdown, &self.conn, record)
    }

    /// Add one directed memory-map link between two existing records.
    pub fn link_memories(
        &mut self,
        repo_root: &str,
        memory_id: &str,
        target_id: &str,
        relation: &str,
    ) -> Result<MemoryRecord> {
        let now = util::format_now();
        markdown_memory::add_memory_link(
            &self.markdown,
            &self.conn,
            repo_root,
            memory_id,
            target_id,
            relation,
            &now,
        )
    }

    /// Hybrid recall (ICM-E2): lexical hits reranked by reciprocal-rank fusion,
    /// then vector-only matches appended after every lexical tier.
    ///
    /// `query_vector` must be produced upstream by the configured embedding
    /// model; this layer never calls a provider, so recall stays deterministic
    /// and works with no embedding backend configured.
    #[allow(clippy::too_many_arguments)]
    pub fn recall_memories_hybrid_with_policy(
        &self,
        repo_root: &str,
        query: &str,
        query_vector: &[f32],
        filter: &MemoryListFilter,
        shared_only: bool,
        viewer: &MemoryViewer,
        limit: usize,
        recall_policy: &MemoryRecallPolicy,
        embedding_policy: &MemoryEmbeddingPolicy,
    ) -> Result<Vec<MemorySearchHit>> {
        memory_embeddings::hybrid_recall_with_policy(
            &self.markdown,
            &self.conn,
            repo_root,
            query,
            query_vector,
            filter,
            shared_only,
            viewer,
            limit,
            recall_policy,
            embedding_policy,
        )
    }

    /// Store or replace the embedding vector for one memory (ICM-E2).
    pub fn store_memory_embedding(
        &mut self,
        memory_id: &str,
        embedding_model: &str,
        dimension: usize,
        vector: &[f32],
    ) -> Result<()> {
        memory_embeddings::upsert_memory_embedding(
            &self.conn,
            memory_id,
            embedding_model,
            dimension,
            vector,
            &util::format_now(),
        )
    }

    /// Stored `(model, dimension, vector)` for one memory, if any.
    pub fn memory_embedding(&self, memory_id: &str) -> Result<Option<(String, usize, Vec<f32>)>> {
        memory_embeddings::get_memory_embedding(&self.conn, memory_id)
    }

    /// Embedding coverage for one model across recallable memories in a repo.
    pub fn memory_embedding_coverage(
        &self,
        repo_root: &str,
        embedding_model: &str,
        dimension: usize,
    ) -> Result<MemoryEmbeddingCoverage> {
        memory_embeddings::memory_embedding_coverage(
            &self.conn,
            repo_root,
            embedding_model,
            dimension,
        )
    }

    /// Recallable memories that still need an embedding for `model`.
    pub fn memories_missing_embeddings(
        &self,
        repo_root: &str,
        embedding_model: &str,
        dimension: usize,
        filter: &MemoryListFilter,
        limit: usize,
    ) -> Result<Vec<MemoryRecord>> {
        memory_embeddings::memories_missing_embeddings(
            &self.conn,
            repo_root,
            embedding_model,
            dimension,
            filter,
            limit,
        )
    }

    /// List memories for a repo, filtered and sorted by `updated_at DESC`.
    pub fn list_memories(
        &self,
        repo_root: &str,
        filter: &MemoryListFilter,
    ) -> Result<Vec<MemoryRecord>> {
        markdown_memory::list_memories(&self.markdown, &self.conn, repo_root, filter)
    }

    /// Delete a memory by exact id within a repo; `dry_run` only inspects.
    pub fn delete_memory(
        &mut self,
        repo_root: &str,
        memory_id: &str,
        dry_run: bool,
    ) -> Result<MemoryDeleteResult> {
        markdown_memory::delete_memory(&self.markdown, &self.conn, repo_root, memory_id, dry_run)
    }

    /// Compute (and, when not dry-running, persist) updated decay scores for
    /// every memory matching `filter`. Never deletes rows.
    pub fn decay_reports(
        &self,
        repo_root: &str,
        filter: &MemoryListFilter,
        policy: &MemoryDecayPolicy,
        dry_run: bool,
    ) -> Result<Vec<MemoryDecayReport>> {
        decay_reports(
            &self.markdown,
            &self.conn,
            repo_root,
            filter,
            policy,
            SystemClock.now_utc(),
            dry_run,
        )
    }

    /// Rows past their retention window per policy; protected critical
    /// memories are never reported as stale candidates.
    pub fn stale_memories(
        &self,
        repo_root: &str,
        filter: &MemoryListFilter,
        policy: &MemoryDecayPolicy,
    ) -> Result<Vec<MemoryDecayReport>> {
        stale_memories(
            &self.markdown,
            &self.conn,
            repo_root,
            filter,
            policy,
            SystemClock.now_utc(),
        )
    }

    /// Delete (or, in dry-run, only report) memories past their retention
    /// window; critical rows need `allow_critical`.
    pub fn prune_memories(
        &self,
        repo_root: &str,
        filter: &MemoryListFilter,
        policy: &MemoryDecayPolicy,
        dry_run: bool,
        allow_critical: bool,
    ) -> Result<MemoryPruneResult> {
        prune_memories(
            &self.markdown,
            &self.conn,
            repo_root,
            filter,
            policy,
            SystemClock.now_utc(),
            dry_run,
            allow_critical,
        )
    }

    /// Deterministic health report with actionable findings.
    pub fn memory_health(
        &self,
        repo_root: &str,
        filter: &MemoryListFilter,
        policy: &MemoryDecayPolicy,
        source_exists: &dyn Fn(&str) -> bool,
    ) -> Result<MemoryHealthReport> {
        health_report(
            &self.markdown,
            &self.conn,
            repo_root,
            filter,
            policy,
            SystemClock.now_utc(),
            source_exists,
        )
    }

    /// Deterministic consolidation plan; when `dry_run` is false the plan is
    /// also applied (consolidated memory created, merged rows superseded).
    pub fn consolidate_memories(
        &self,
        repo_root: &str,
        filter: &MemoryListFilter,
        dry_run: bool,
    ) -> Result<MemoryConsolidationPlan> {
        markdown_memory::consolidation_plan(
            &self.markdown,
            &self.conn,
            repo_root,
            filter,
            dry_run,
            None,
        )
    }

    /// Deterministic consolidation with an overflow sink (typically `context.db`)
    /// for synthesized bodies that exceed `OVERSIZED_BODY_CHARS`.
    pub fn consolidate_memories_with_overflow(
        &self,
        repo_root: &str,
        filter: &MemoryListFilter,
        dry_run: bool,
        overflow: &mut self::memory_consolidate::OverflowSink<'_>,
    ) -> Result<MemoryConsolidationPlan> {
        markdown_memory::consolidation_plan(
            &self.markdown,
            &self.conn,
            repo_root,
            filter,
            dry_run,
            Some(overflow),
        )
    }
}
