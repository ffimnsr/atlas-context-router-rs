use super::*;

impl SessionStore {
    pub fn upsert_session_meta(
        &mut self,
        session_id: SessionId,
        repo_root: &str,
        frontend: &str,
        worktree_id: Option<&str>,
    ) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO session_meta (
                    session_id, repo_root, frontend, worktree_id, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(session_id) DO UPDATE SET
                    repo_root = excluded.repo_root,
                    frontend = excluded.frontend,
                    worktree_id = excluded.worktree_id,
                    updated_at = excluded.updated_at",
                params![
                    session_id.as_str(),
                    repo_root,
                    frontend,
                    worktree_id,
                    now,
                    now
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn get_session_meta(&self, session_id: &SessionId) -> Result<Option<SessionMeta>> {
        self.conn
            .query_row(
                "SELECT session_id, repo_root, frontend, worktree_id, created_at, updated_at,
                        last_resume_at, last_compaction_at
                 FROM session_meta
                 WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| {
                    Ok(SessionMeta {
                        session_id: SessionId(row.get(0)?),
                        repo_root: row.get(1)?,
                        frontend: row.get(2)?,
                        worktree_id: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        last_resume_at: row.get(6)?,
                        last_compaction_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|e| AtlasError::Db(e.to_string()))
    }

    pub fn append_event(&mut self, event: NewSessionEvent) -> Result<Option<SessionEventRow>> {
        let mut payload = event.payload;
        if let Some(meta) = self.get_session_meta(&event.session_id)? {
            normalize_event_payload_paths(&meta.repo_root, &mut payload);
        }
        let payload_json = canonical_json(&payload);
        self.ensure_payload_fits(&payload_json)?;
        let created_at = event.created_at.unwrap_or_else(format_now);
        let event_hash = util::hash_event(&event.event_type, event.priority, &payload_json);

        if self.config.dedup_window_secs > 0 {
            let window_cutoff = format_seconds_ago(self.config.dedup_window_secs);
            let recent: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM session_events
                     WHERE session_id = ?1
                       AND event_type = ?2
                       AND priority = ?3
                       AND created_at >= ?4
                       AND event_hash = ?5",
                    params![
                        event.session_id.as_str(),
                        event.event_type.as_str(),
                        event.priority,
                        window_cutoff,
                        event_hash,
                    ],
                    |row| row.get(0),
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            if recent > 0 {
                return Ok(None);
            }
        }

        let tx = self
            .conn
            .transaction()
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO session_events (
                    session_id, event_type, priority, payload_json, event_hash, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    event.session_id.as_str(),
                    event.event_type.as_str(),
                    event.priority,
                    payload_json,
                    event_hash,
                    created_at,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        tx.execute(
            "UPDATE session_meta SET updated_at = ?2 WHERE session_id = ?1",
            params![event.session_id.as_str(), format_now()],
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut evicted = 0usize;
        if inserted > 0 {
            evicted = enforce_event_limit(
                &tx,
                event.session_id.as_str(),
                self.config.max_events_per_session,
            )?;
        }

        if evicted > 0 {
            let compacted_at = format_now();
            tx.execute(
                "UPDATE session_meta
                 SET updated_at = ?2, last_compaction_at = ?2
                 WHERE session_id = ?1",
                params![event.session_id.as_str(), compacted_at],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        }

        let row = tx
            .query_row(
                "SELECT id, session_id, event_type, priority, payload_json, event_hash, created_at
                 FROM session_events
                 WHERE session_id = ?1 AND event_hash = ?2",
                params![event.session_id.as_str(), event_hash],
                row_to_event,
            )
            .optional()
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        if inserted > 0
            && event.event_type == SessionEventType::Decision
            && let Some(stored) = row.as_ref()
        {
            let payload: Value = serde_json::from_str(&stored.payload_json).unwrap_or(Value::Null);
            if let Err(error) = upsert_decision_from_event(
                &tx,
                &event.session_id,
                stored.id,
                &payload,
                &stored.created_at,
            ) {
                tracing::warn!(err = %error, "decision memory upsert failed; event kept");
            }
        }

        tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;
        markdown_records::sync_decisions(&self.markdown, &self.conn)?;
        if let Some(stored) = row.as_ref()
            && let Some(meta) = self.get_session_meta(&event.session_id)?
            && let Err(error) = markdown_session::append_event(&self.markdown, &meta, stored)
        {
            tracing::warn!(%error, "session ledger append failed; derived row is committed");
        }

        if inserted == 0 {
            return Ok(None);
        }

        Ok(row)
    }

    pub fn list_events(&self, session_id: &SessionId) -> Result<Vec<SessionEventRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, event_type, priority, payload_json, event_hash, created_at
                 FROM session_events
                 WHERE session_id = ?1
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let rows = stmt
            .query_map(params![session_id.as_str()], row_to_event)
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn create_durable_task(&mut self, task: &NewDurableTask) -> Result<DurableTaskRecord> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO durable_tasks (
                    task_id, originating_method, request_id, tool_name, transport_kind, session_id,
                    created_at, updated_at, status, status_message, ttl_ms, cancel_requested,
                    input_requests_json, request_state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, NULL, NULL)",
                params![
                    task.task_id,
                    task.originating_method,
                    task.request_id,
                    task.tool_name,
                    task.transport_kind,
                    task.session_id,
                    now,
                    now,
                    task.status.as_str(),
                    task.status_message,
                    task.ttl_ms.map(|value| value as i64),
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.get_durable_task(&task.task_id)?
            .ok_or_else(|| AtlasError::Db("created durable task missing on reload".to_owned()))
            .inspect(|_| {
                let _ = markdown_records::sync_tasks(&self.markdown, &self.conn);
            })
    }

    pub fn update_durable_task(&mut self, task_id: &str, update: &DurableTaskUpdate) -> Result<()> {
        let now = format_now();
        let status = update.status.map(|value| value.as_str().to_owned());
        let progress_json = update.progress.as_ref().map(canonical_json);
        let result_json = update.result.as_ref().map(canonical_json);
        let error_json = update.error.as_ref().map(canonical_json);
        let input_requests_json = update.input_requests.as_ref().map(canonical_json);
        self.conn
            .execute(
                "UPDATE durable_tasks
                 SET status = COALESCE(?2, status),
                     status_message = COALESCE(?3, status_message),
                     progress_json = COALESCE(?4, progress_json),
                     result_json = COALESCE(?5, result_json),
                     error_json = COALESCE(?6, error_json),
                     cancel_requested = COALESCE(?7, cancel_requested),
                     updated_at = ?8,
                     input_requests_json = COALESCE(?9, input_requests_json),
                     request_state = COALESCE(?10, request_state)
                 WHERE task_id = ?1",
                params![
                    task_id,
                    status,
                    update.status_message,
                    progress_json,
                    result_json,
                    error_json,
                    update.cancel_requested.map(i32::from),
                    now,
                    input_requests_json,
                    update.request_state,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        markdown_records::sync_tasks(&self.markdown, &self.conn)?;
        Ok(())
    }

    pub fn delete_durable_task(&mut self, task_id: &str) -> Result<bool> {
        let deleted = self
            .conn
            .execute(
                "DELETE FROM durable_tasks WHERE task_id = ?1",
                params![task_id],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        if deleted > 0 {
            markdown_records::delete_task(&self.markdown, task_id)?;
        }
        Ok(deleted > 0)
    }

    pub fn get_durable_task(&self, task_id: &str) -> Result<Option<DurableTaskRecord>> {
        self.conn
            .query_row(
                "SELECT task_id, originating_method, request_id, tool_name, transport_kind, session_id,
                        created_at, updated_at, status, status_message, progress_json, result_json,
                        error_json, input_requests_json, request_state, ttl_ms, cancel_requested
                 FROM durable_tasks
                 WHERE task_id = ?1",
                params![task_id],
                row_to_durable_task,
            )
            .optional()
            .map_err(|e| AtlasError::Db(e.to_string()))
    }

    pub fn list_durable_tasks(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<DurableTaskListPage> {
        let limit = limit.clamp(1, 100);
        let mut tasks = Vec::new();
        let mut next_cursor = None;
        let cursor = cursor.map(parse_durable_task_list_cursor).transpose()?;
        let mut stmt = if cursor.is_some() {
            self.conn
                .prepare(
                    "SELECT task_id, originating_method, request_id, tool_name, transport_kind, session_id,
                            created_at, updated_at, status, status_message, progress_json, result_json,
                            error_json, input_requests_json, request_state, ttl_ms, cancel_requested
                     FROM durable_tasks
                     WHERE updated_at < ?1
                        OR (updated_at = ?1 AND created_at < ?2)
                        OR (updated_at = ?1 AND created_at = ?2 AND task_id < ?3)
                     ORDER BY updated_at DESC, created_at DESC, task_id DESC
                     LIMIT ?4",
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?
        } else {
            self.conn
                .prepare(
                    "SELECT task_id, originating_method, request_id, tool_name, transport_kind, session_id,
                            created_at, updated_at, status, status_message, progress_json, result_json,
                            error_json, input_requests_json, request_state, ttl_ms, cancel_requested
                     FROM durable_tasks
                     ORDER BY updated_at DESC, created_at DESC, task_id DESC
                     LIMIT ?1",
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?
        };
        if let Some(cursor) = cursor.as_ref() {
            let rows = stmt
                .query_map(
                    params![
                        &cursor.updated_at,
                        &cursor.created_at,
                        &cursor.task_id,
                        (limit + 1) as i64
                    ],
                    row_to_durable_task,
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            tasks.extend(rows.filter_map(std::result::Result::ok));
        } else {
            let rows = stmt
                .query_map(params![(limit + 1) as i64], row_to_durable_task)
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            tasks.extend(rows.filter_map(std::result::Result::ok));
        }
        if tasks.len() > limit {
            tasks
                .pop()
                .expect("extra task exists when page exceeds limit");
            next_cursor = tasks.last().map(encode_durable_task_list_cursor);
        }
        Ok(DurableTaskListPage { tasks, next_cursor })
    }

    pub fn put_resume_snapshot(
        &mut self,
        session_id: &SessionId,
        snapshot: &str,
        event_count: i64,
        consumed: bool,
    ) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO session_resume (
                    session_id, snapshot, event_count, consumed, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(session_id) DO UPDATE SET
                    snapshot = excluded.snapshot,
                    event_count = excluded.event_count,
                    consumed = excluded.consumed,
                    updated_at = excluded.updated_at",
                params![
                    session_id.as_str(),
                    snapshot,
                    event_count,
                    i32::from(consumed),
                    now,
                    now,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.conn
            .execute(
                "UPDATE session_meta SET updated_at = ?2 WHERE session_id = ?1",
                params![session_id.as_str(), now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        markdown_session::write_resume(
            &self.markdown,
            session_id.as_str(),
            snapshot,
            event_count,
            consumed,
            &now,
        )?;
        Ok(())
    }

    pub fn get_resume_snapshot(&self, session_id: &SessionId) -> Result<Option<ResumeSnapshot>> {
        self.conn
            .query_row(
                "SELECT session_id, snapshot, event_count, consumed, created_at, updated_at
                 FROM session_resume
                 WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| {
                    Ok(ResumeSnapshot {
                        session_id: SessionId(row.get(0)?),
                        snapshot: row.get(1)?,
                        event_count: row.get(2)?,
                        consumed: row.get::<_, i32>(3)? != 0,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|e| AtlasError::Db(e.to_string()))
    }

    pub fn mark_resume_consumed(&mut self, session_id: &SessionId, consumed: bool) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "UPDATE session_resume
                 SET consumed = ?2, updated_at = ?3
                 WHERE session_id = ?1",
                params![session_id.as_str(), i32::from(consumed), now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.conn
            .execute(
                "UPDATE session_meta
                 SET updated_at = ?2, last_resume_at = ?2
                 WHERE session_id = ?1",
                params![session_id.as_str(), now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let stored: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT snapshot, event_count FROM session_resume WHERE session_id = ?1",
                params![session_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some((snapshot, event_count)) = stored {
            markdown_session::write_resume(
                &self.markdown,
                session_id.as_str(),
                &snapshot,
                event_count,
                consumed,
                &now,
            )?;
        }
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT session_id, repo_root, frontend, worktree_id, created_at, updated_at,
                        last_resume_at, last_compaction_at
                 FROM session_meta
                 ORDER BY updated_at DESC, session_id ASC",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionMeta {
                    session_id: SessionId(row.get(0)?),
                    repo_root: row.get(1)?,
                    frontend: row.get(2)?,
                    worktree_id: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    last_resume_at: row.get(6)?,
                    last_compaction_at: row.get(7)?,
                })
            })
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    pub fn delete_session(&mut self, session_id: &SessionId) -> Result<bool> {
        let rows = self
            .conn
            .execute(
                "DELETE FROM session_meta WHERE session_id = ?1",
                params![session_id.as_str()],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        if rows > 0 {
            markdown_session::delete_session(&self.markdown, session_id.as_str())?;
        }
        Ok(rows > 0)
    }

    pub fn stats(&self) -> Result<SessionStats> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let session_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM session_meta", [], |r| r.get(0))
            .map_err(db_err)?;
        let total_events: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM session_events", [], |r| r.get(0))
            .map_err(db_err)?;
        let snapshot_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM session_resume", [], |r| r.get(0))
            .map_err(db_err)?;
        Ok(SessionStats {
            session_count: session_count as usize,
            total_events: total_events as usize,
            snapshot_count: snapshot_count as usize,
        })
    }

    pub fn cleanup_stale_sessions(&mut self, keep_days: u32) -> Result<usize> {
        let cutoff = format_days_ago(keep_days);
        let stale = self
            .conn
            .prepare("SELECT session_id FROM session_meta WHERE updated_at < ?1")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map(params![cutoff], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| rows.filter_map(std::result::Result::ok).collect::<Vec<_>>())
            })
            .unwrap_or_default();
        let count = self
            .conn
            .execute(
                "DELETE FROM session_meta WHERE updated_at < ?1",
                params![cutoff],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        for session_id in stale {
            let _ = markdown_session::delete_session(&self.markdown, &session_id);
        }
        Ok(count)
    }

    pub fn compact_session(&mut self, session_id: &SessionId) -> Result<CurationResult> {
        compact_session_events(self, session_id)
    }

    pub fn build_resume(&mut self, session_id: &SessionId) -> Result<ResumeSnapshot> {
        // Apply curation before building the snapshot so the resume material
        // reflects clean, deduplicated event history (CM10 requirement).
        let _ = compact_session_events(self, session_id);
        build_resume_snapshot(self, session_id)
    }

    pub fn build_resume_view(
        &self,
        session_id: &SessionId,
        agent_id: Option<&str>,
        merge_agent_partitions: bool,
    ) -> Result<Value> {
        let events = self.list_events(session_id)?;
        self::resume::build_resume_snapshot_view(
            self,
            session_id,
            events,
            agent_id,
            merge_agent_partitions,
        )
    }

    fn ensure_payload_fits(&self, payload_json: &str) -> Result<()> {
        if payload_json.len() > self.config.max_inline_payload_bytes {
            return Err(AtlasError::Other(format!(
                "session event payload {} bytes exceeds inline limit {} bytes; store raw output in content store and write source_id into payload",
                payload_json.len(),
                self.config.max_inline_payload_bytes
            )));
        }
        Ok(())
    }
}
