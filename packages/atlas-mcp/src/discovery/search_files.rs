//! MCP tool: `search_files` — file-path discovery by glob pattern.

use std::path::Path;

use anyhow::{Context, Result};
use atlas_core::BudgetManager;
use serde::Serialize;

use crate::output::OutputFormat;

use super::shared::{
    bool_arg, build_globset, discovery_tool_error_result, inject_budget_metadata,
    load_budget_policy, normalized_optional_subpath, render_tool_result, resolve_subpath_walk_root,
    str_arg, string_array_arg,
};

/// MCP tool: `search_files` — file-path discovery by glob pattern.
///
/// Walks the repo honouring `.gitignore` / `.git/info/exclude` / `.atlasignore`
/// rules (via the `ignore` crate `WalkBuilder`). Matches filenames and paths
/// against the required `pattern` glob and optional `globs` include-path filters.
pub(crate) fn tool_search_files(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    (|| {
        let policy = load_budget_policy(repo_root)?;
        let mut budgets = BudgetManager::new();
        let pattern = str_arg(args, "pattern")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: pattern"))?
            .to_owned();
        let globs = string_array_arg(args, "globs")?;
        let exclude_globs = string_array_arg(args, "exclude_globs")?;
        let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(false);
        let subpath = normalized_optional_subpath(str_arg(args, "subpath")?);
        let result_limit = budgets.resolve_limit(
            policy.review_context_extraction.files,
            "review_context_extraction.max_files",
            None,
        );

        let name_matcher = build_globset(&[&pattern], case_sensitive)
            .with_context(|| format!("invalid pattern glob: {pattern}"))?;

        let include_filter = if globs.is_empty() {
            None
        } else {
            Some(build_globset(&globs, case_sensitive).context("invalid globs filter")?)
        };

        let exclude_filter = if exclude_globs.is_empty() {
            None
        } else {
            Some(build_globset(&exclude_globs, false).context("invalid exclude_globs filter")?)
        };

        let walk_root = match subpath.as_deref() {
            Some(sp) => resolve_subpath_walk_root(repo_root, sp)?,
            None => repo_root.to_owned(),
        };

        let atlasignore_path = Path::new(repo_root).join(".atlasignore");
        let mut walker = ignore::WalkBuilder::new(&walk_root);
        walker
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .ignore(true);
        if atlasignore_path.exists() {
            walker.add_ignore(&atlasignore_path);
        }

        let mut files: Vec<String> = Vec::new();
        let mut truncated = false;

        for entry in walker.build().flatten() {
            if files.len() >= result_limit {
                truncated = true;
                break;
            }
            let ft = match entry.file_type() {
                Some(ft) => ft,
                None => continue,
            };
            if !ft.is_file() {
                continue;
            }

            let full_path = entry.path();
            let rel_path = match full_path.strip_prefix(repo_root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };

            if let Some(ref filter) = include_filter
                && !filter.is_match(&*rel_path)
            {
                continue;
            }

            if let Some(ref excl) = exclude_filter
                && excl.is_match(&*rel_path)
            {
                continue;
            }

            let file_name = full_path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            if !name_matcher.is_match(file_name.as_ref()) && !name_matcher.is_match(&*rel_path) {
                continue;
            }

            files.push(rel_path);
        }

        files.sort_unstable();

        let result_count = files.len();
        if truncated {
            budgets.record_usage(
                policy.review_context_extraction.files,
                "review_context_extraction.max_files",
                result_limit,
                result_limit.saturating_add(1),
                true,
            );
        }
        let atlas_hint = if result_count == 0 {
            Some(
                "No files matched. Try a broader glob (e.g. '*.rs' instead of 'foo*.rs'), \
                     verify the pattern syntax, or use search_content for content-based lookup.",
            )
        } else {
            None
        };

        #[derive(Serialize)]
        struct SearchFilesResult<'a> {
            files: Vec<String>,
            result_count: usize,
            truncated: bool,
            atlas_result_kind: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            atlas_hint: Option<&'a str>,
        }

        let result = SearchFilesResult {
            files,
            result_count,
            truncated,
            atlas_result_kind: "file_paths",
            atlas_hint,
        };

        let mut response = render_tool_result(&result, output_format)?;
        inject_budget_metadata(
            &mut response,
            &budgets.summary(
                "review_context_extraction.max_files",
                result_limit,
                result_count,
            ),
        );
        Ok(response)
    })()
    .or_else(|error| discovery_tool_error_result("search_files", output_format, error))
}
