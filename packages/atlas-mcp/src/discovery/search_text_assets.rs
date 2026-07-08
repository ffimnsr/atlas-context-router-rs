//! MCP tool: `search_text_assets` — discover SQL, config, env, and prompt files.

use std::path::Path;

use anyhow::{Context, Result};
use atlas_core::BudgetManager;
use serde::Serialize;

use crate::output::OutputFormat;

use super::shared::{
    bool_arg, build_globset, discovery_tool_error_result, inject_budget_metadata,
    load_budget_policy, normalized_optional_subpath, render_tool_result, resolve_subpath_walk_root,
    str_arg, string_array_arg, u64_arg,
};

/// Text asset kinds and their associated glob patterns.
const TEXT_ASSET_SQL: &[&str] = &["*.sql"];
const TEXT_ASSET_CONFIG: &[&str] = &["*.toml", "*.yaml", "*.yml", "*.ini", "*.cfg", "*.conf"];
const TEXT_ASSET_ENV: &[&str] = &[".env", ".env.*", "*.env"];
const TEXT_ASSET_PROMPT: &[&str] = &[
    "*.prompt",
    "*.promptfile",
    "*.p8",
    "prompts/*.md",
    "prompts/**/*.md",
    "**/*.prompt.md",
];
const TEXT_ASSET_ALL: &[&str] = &[
    "*.sql",
    "*.toml",
    "*.yaml",
    "*.yml",
    "*.ini",
    "*.cfg",
    "*.conf",
    ".env",
    ".env.*",
    "*.env",
    "*.prompt",
    "*.promptfile",
    "*.p8",
];

/// MCP tool: `search_text_assets` — discover SQL, config, env, and prompt files.
///
/// Use this to locate non-template text assets that are not indexed as graph
/// symbols. Pass `kind` to narrow to a specific asset type: `sql`, `config`,
/// `env`, or `prompt`. Supports `subpath` for monorepo scoping and
/// `exclude_globs` for fine-grained exclusion.
pub(crate) fn tool_search_text_assets(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    (|| {
        let policy = load_budget_policy(repo_root)?;
        let mut budgets = BudgetManager::new();
        let kind = str_arg(args, "kind")?.map(str::to_owned);
        let globs = string_array_arg(args, "globs")?;
        let exclude_globs = string_array_arg(args, "exclude_globs")?;
        let subpath = normalized_optional_subpath(str_arg(args, "subpath")?);
        let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(false);
        let requested_max_results = u64_arg(args, "max_results").unwrap_or(100) as usize;
        let max_results = budgets.resolve_limit(
            policy.review_context_extraction.files,
            "review_context_extraction.max_files",
            Some(requested_max_results),
        );

        let extension_patterns: Vec<&str> = match kind.as_deref() {
            Some("sql") => TEXT_ASSET_SQL.to_vec(),
            Some("config") => TEXT_ASSET_CONFIG.to_vec(),
            Some("env") => TEXT_ASSET_ENV.to_vec(),
            Some("prompt") => TEXT_ASSET_PROMPT.to_vec(),
            None | Some(_) => TEXT_ASSET_ALL.to_vec(),
        };

        let name_matcher = build_globset(&extension_patterns, case_sensitive)
            .context("invalid text-asset extension patterns")?;

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
            if files.len() >= max_results {
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
                max_results,
                max_results.saturating_add(1),
                true,
            );
        }
        let atlas_hint = if result_count == 0 {
            let kind_hint = kind.as_deref().unwrap_or("any text asset");
            Some(format!(
                "No {kind_hint} files found. Supported kinds: sql, config, env, prompt. \
                 Try broadening with `kind` omitted or check the subpath.",
            ))
        } else {
            None
        };

        #[derive(Serialize)]
        struct SearchTextAssetsResult {
            files: Vec<String>,
            result_count: usize,
            truncated: bool,
            atlas_result_kind: &'static str,
            #[serde(skip_serializing_if = "Option::is_none")]
            atlas_hint: Option<String>,
        }

        let result = SearchTextAssetsResult {
            files,
            result_count,
            truncated,
            atlas_result_kind: "text_asset_files",
            atlas_hint,
        };

        let mut response = render_tool_result(&result, output_format)?;
        inject_budget_metadata(
            &mut response,
            &budgets.summary(
                "review_context_extraction.max_files",
                max_results,
                requested_max_results.max(result_count),
            ),
        );
        Ok(response)
    })()
    .or_else(|error| discovery_tool_error_result("search_text_assets", output_format, error))
}
