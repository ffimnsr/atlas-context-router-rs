//! Literal directory scans over indexed file paths.

use atlas_core::{AtlasError, Result};
use rusqlite::params;

use super::Store;

impl Store {
    /// Return indexed file paths directly under `dir` (`dir/...`) for
    /// `source_repo_id`, ordered by path.
    ///
    /// Uses a half-open `(source_repo_id, path)` range scan rather than `LIKE`
    /// so the lookup stays index-backed and literal: paths containing `LIKE`
    /// metacharacters (`_`, `%`) are matched exactly, and default
    /// case-insensitive `LIKE` collation cannot widen the result set.
    pub fn file_paths_under_dir_for_repo(
        &self,
        source_repo_id: &str,
        dir: &str,
    ) -> Result<Vec<String>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let lower = format!("{dir}/");
        // Increment the trailing '/' so the upper bound is exclusive: every
        // string starting with `dir/` sorts below `dir0`.
        let mut upper = lower.clone();
        upper.pop();
        upper.push('0');

        let mut stmt = self
            .conn
            .prepare(
                "SELECT path FROM files
                 WHERE source_repo_id = ?1 AND path >= ?2 AND path < ?3
                 ORDER BY path",
            )
            .map_err(db_err)?;
        let paths = stmt
            .query_map(params![source_repo_id, lower, upper], |row| {
                row.get::<_, String>(0)
            })
            .map_err(db_err)?
            .filter_map(|row| row.ok())
            .collect();
        Ok(paths)
    }
}
