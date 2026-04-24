use std::time::{SystemTime, UNIX_EPOCH};

use atlas_core::{
    AtlasError, PostprocessExecutionMode, PostprocessRunState, PostprocessRunSummary,
    PostprocessStageSummary, PostprocessStatus, Result,
};
use rusqlite::{Row, params};

use super::Store;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn run_state_from_str(value: &str) -> PostprocessRunState {
    match value {
        "running" => PostprocessRunState::Running,
        "failed" => PostprocessRunState::Failed,
        _ => PostprocessRunState::Succeeded,
    }
}

fn mode_from_str(value: &str) -> PostprocessExecutionMode {
    match value {
        "changed_only" => PostprocessExecutionMode::ChangedOnly,
        _ => PostprocessExecutionMode::Full,
    }
}

fn row_to_postprocess_status(row: &Row<'_>) -> rusqlite::Result<PostprocessStatus> {
    let state: String = row.get(1)?;
    let mode: String = row.get(2)?;
    let stages_json: Option<String> = row.get(5)?;
    let stages = stages_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<PostprocessStageSummary>>(raw).ok())
        .unwrap_or_default();

    Ok(PostprocessStatus {
        repo_root: row.get(0)?,
        state: run_state_from_str(&state),
        mode: mode_from_str(&mode),
        stage_filter: row.get(3)?,
        changed_file_count: row.get::<_, i64>(4)? as usize,
        stages,
        started_at_ms: row.get(6)?,
        finished_at_ms: row.get(7)?,
        last_error_code: row.get(8)?,
        last_error: row.get(9)?,
        updated_at_ms: row.get(10)?,
    })
}

impl Store {
    pub fn begin_postprocess(
        &self,
        repo_root: &str,
        mode: PostprocessExecutionMode,
        stage_filter: Option<&str>,
        changed_file_count: usize,
    ) -> Result<()> {
        let timestamp = now_ms();
        self.conn
            .execute(
                "INSERT INTO postprocess_state
                    (repo_root, state, mode, stage_filter, changed_file_count, stages_json,
                     started_at_ms, finished_at_ms, last_error_code, last_error, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, NULL, ?7)
                 ON CONFLICT(repo_root) DO UPDATE SET
                    state = excluded.state,
                    mode = excluded.mode,
                    stage_filter = excluded.stage_filter,
                    changed_file_count = excluded.changed_file_count,
                    stages_json = excluded.stages_json,
                    started_at_ms = excluded.started_at_ms,
                    finished_at_ms = NULL,
                    last_error_code = NULL,
                    last_error = NULL,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    repo_root,
                    PostprocessRunState::Running.as_str(),
                    mode.as_str(),
                    stage_filter,
                    changed_file_count as i64,
                    serde_json::to_string(&Vec::<PostprocessStageSummary>::new())
                        .map_err(AtlasError::Serde)?,
                    timestamp,
                ],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        Ok(())
    }

    pub fn finish_postprocess(&self, summary: &PostprocessRunSummary) -> Result<()> {
        let updated_at_ms = now_ms();
        self.conn
            .execute(
                "INSERT INTO postprocess_state
                    (repo_root, state, mode, stage_filter, changed_file_count, stages_json,
                     started_at_ms, finished_at_ms, last_error_code, last_error, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(repo_root) DO UPDATE SET
                    state = excluded.state,
                    mode = excluded.mode,
                    stage_filter = excluded.stage_filter,
                    changed_file_count = excluded.changed_file_count,
                    stages_json = excluded.stages_json,
                    started_at_ms = excluded.started_at_ms,
                    finished_at_ms = excluded.finished_at_ms,
                    last_error_code = excluded.last_error_code,
                    last_error = excluded.last_error,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    summary.repo_root,
                    summary.state.as_str(),
                    summary.requested_mode.as_str(),
                    summary.stage_filter,
                    summary.changed_files.len() as i64,
                    serde_json::to_string(&summary.stages).map_err(AtlasError::Serde)?,
                    summary.started_at_ms,
                    summary.finished_at_ms,
                    if summary.ok {
                        None::<String>
                    } else {
                        Some(summary.error_code.clone())
                    },
                    if summary.ok {
                        None::<String>
                    } else {
                        Some(summary.message.clone())
                    },
                    updated_at_ms,
                ],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        Ok(())
    }

    pub fn get_postprocess_status(&self, repo_root: &str) -> Result<Option<PostprocessStatus>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT repo_root, state, mode, stage_filter, changed_file_count, stages_json,
                        started_at_ms, finished_at_ms, last_error_code, last_error, updated_at_ms
                 FROM postprocess_state
                 WHERE repo_root = ?1",
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        let mut rows = stmt
            .query_map([repo_root], row_to_postprocess_status)
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        match rows.next() {
            Some(Ok(status)) => Ok(Some(status)),
            Some(Err(error)) => Err(AtlasError::Db(error.to_string())),
            None => Ok(None),
        }
    }

    pub fn find_large_functions(
        &self,
        files: Option<&[String]>,
        min_lines: usize,
        limit: usize,
    ) -> Result<Vec<atlas_core::Node>> {
        let db_err = |error: rusqlite::Error| AtlasError::Db(error.to_string());
        let normalized_files = files
            .map(|paths| {
                paths
                    .iter()
                    .map(|path| super::helpers::canonicalize_repo_path(path))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;

        let mut filters = vec![
            "kind IN ('function', 'method')".to_string(),
            "is_test = 0".to_string(),
            "(line_end - line_start + 1) >= ?".to_string(),
        ];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(min_lines as i64)];

        if let Some(files) = &normalized_files
            && !files.is_empty()
        {
            let placeholders = super::helpers::repeat_placeholders(files.len());
            filters.push(format!("file_path IN ({placeholders})"));
            for file in files {
                params.push(Box::new(file.clone()));
            }
        }

        params.push(Box::new(limit as i64));
        let sql = format!(
            "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                    language, parent_name, params, return_type, modifiers,
                    is_test, file_hash, extra_json
             FROM nodes
             WHERE {}
             ORDER BY (line_end - line_start + 1) DESC, qualified_name ASC
             LIMIT ?",
            filters.join(" AND ")
        );

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|item| item.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(params_ref.as_slice(), super::helpers::row_to_node)
            .map_err(db_err)?
            .filter_map(|row| row.ok())
            .collect();
        Ok(rows)
    }
}
