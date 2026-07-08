//! Shared types, constants, and helpers for discovery tools.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use atlas_core::{BudgetPolicy, BudgetReport};
use atlas_repo::CanonicalRepoPath;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Serialize;

use crate::output::OutputFormat;
use crate::tool_result::{
    ToolErrorCode, ToolErrorPayload, normalize_tool_execution_error, tool_execution_error_value,
    tool_result_value as build_tool_result_value,
};

// ---------------------------------------------------------------------------
// Repo-path types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct RepoPathCandidate {
    pub(super) path: String,
    pub(super) reason: &'static str,
}

#[derive(Clone, Debug)]
pub(super) struct RepoPathIdentity {
    pub(super) repo_root: String,
    pub(super) repo_name: String,
    pub(super) workspace_root_prefix: String,
}

impl RepoPathIdentity {
    pub(super) fn new(repo_root: &str) -> Self {
        let repo_name = Path::new(repo_root)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| repo_root.to_owned());
        Self {
            repo_root: repo_root.to_owned(),
            workspace_root_prefix: format!("{repo_name}/"),
            repo_name,
        }
    }

    pub(super) fn details(&self, path: &str) -> serde_json::Value {
        serde_json::json!({
            "path": path,
            "repo_root": self.repo_root,
            "repo_name": self.repo_name,
            "accepted_root_prefixes": [""],
            "workspace_root_prefix": self.workspace_root_prefix,
            "canonical_path_guidance": "Atlas file tools expect repo-relative paths under current repo root, for example 'src/lib.rs'. Do not prefix with workspace root, nested project root, or foreign repo name.",
        })
    }
}

// ---------------------------------------------------------------------------
// Subpath helpers
// ---------------------------------------------------------------------------

/// Validate a user-supplied `subpath` and return the absolute walk root.
///
/// Security: `CanonicalRepoPath::from_repo_relative` rejects any `..` segments
/// that would escape `repo_root`, absolute paths, and empty inputs. Without this
/// guard a caller could supply `../../etc` and the `WalkBuilder` would traverse
/// outside the repository boundary before the `strip_prefix` output filter
/// had any effect.
///
/// Falls back to `repo_root` when the resolved candidate is not a directory.
pub(super) fn resolve_subpath_walk_root(repo_root: &str, subpath: &str) -> Result<String> {
    let canonical = CanonicalRepoPath::from_repo_relative(subpath)
        .map_err(|e| anyhow::anyhow!("invalid subpath '{subpath}': {e}"))?;
    let candidate = Path::new(repo_root).join(canonical.as_str());
    if candidate.is_dir() {
        Ok(candidate.to_string_lossy().into_owned())
    } else {
        Ok(repo_root.to_owned())
    }
}

pub(super) fn normalized_optional_subpath(raw: Option<&str>) -> Option<String> {
    raw.and_then(|subpath| {
        let trimmed = subpath.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

// ---------------------------------------------------------------------------
// Repo-path candidate helpers
// ---------------------------------------------------------------------------

pub(super) fn unique_existing_repo_path_candidates(
    repo_root: &str,
    path: &str,
) -> Vec<RepoPathCandidate> {
    let normalized = path.trim().replace('\\', "/");
    let segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() < 2 {
        return Vec::new();
    }

    let repo_name = Path::new(repo_root)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut candidates: BTreeMap<String, &'static str> = BTreeMap::new();
    for strip_count in 1..segments.len() {
        let tail = segments[strip_count..].join("/");
        if tail.is_empty() {
            continue;
        }
        let Ok(canonical) = CanonicalRepoPath::from_repo_relative(&tail) else {
            continue;
        };
        let absolute = Path::new(repo_root).join(canonical.as_str());
        if !absolute.exists() {
            continue;
        }
        let reason = if strip_count == 1 && segments[0] == repo_name {
            "duplicated_root_prefix"
        } else if strip_count == 1 {
            "foreign_root_prefix"
        } else {
            "nested_subdir_root_prefix"
        };
        candidates
            .entry(canonical.as_str().to_owned())
            .or_insert(reason);
    }

    candidates
        .into_iter()
        .map(|(path, reason)| RepoPathCandidate { path, reason })
        .collect()
}

pub(super) fn unique_basename_repo_path_candidate(
    repo_root: &str,
    path: &str,
) -> Option<RepoPathCandidate> {
    let basename = Path::new(path.trim())
        .file_name()
        .and_then(|name| name.to_str())?;
    if basename.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    let atlasignore_path = Path::new(repo_root).join(".atlasignore");
    let mut walker = ignore::WalkBuilder::new(repo_root);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true);
    if atlasignore_path.exists() {
        walker.add_ignore(&atlasignore_path);
    }

    for entry in walker.build().flatten() {
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let full_path = entry.path();
        let Some(name) = full_path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name != basename {
            continue;
        }
        let Ok(rel_path) = full_path.strip_prefix(repo_root) else {
            continue;
        };
        matches.push(rel_path.to_string_lossy().replace('\\', "/"));
        if matches.len() > 1 {
            return None;
        }
    }

    matches.into_iter().next().map(|path| RepoPathCandidate {
        path,
        reason: "unique_basename_match",
    })
}

// ---------------------------------------------------------------------------
// Error builders
// ---------------------------------------------------------------------------

pub(super) fn build_repo_path_error_payload(
    tool_name: &str,
    repo_root: &str,
    path: &str,
    code: ToolErrorCode,
    message: impl Into<String>,
    retry_guidance: impl Into<String>,
    candidates: Vec<RepoPathCandidate>,
) -> ToolErrorPayload {
    let identity = RepoPathIdentity::new(repo_root);
    let mut details = identity.details(path);
    if candidates.len() == 1
        && let Some(primary) = candidates.first()
    {
        details["suggested_repo_relative_path"] = serde_json::Value::String(primary.path.clone());
        details["suggestion_reason"] = serde_json::Value::String(primary.reason.to_owned());
    }
    if !candidates.is_empty() {
        details["candidate_paths"] =
            serde_json::to_value(&candidates).unwrap_or(serde_json::Value::Null);
    }
    if candidates.len() > 1 {
        details["ambiguity"] = serde_json::Value::String(
            "multiple deterministic repo-relative candidates exist; Atlas refused to guess"
                .to_owned(),
        );
    }

    ToolErrorPayload::new(code, message)
        .with_tool(tool_name)
        .with_retry_guidance(retry_guidance)
        .with_details(details)
}

pub(super) fn resolve_repo_file_path(repo_root: &str, path: &str) -> Result<(String, PathBuf)> {
    let canonical = CanonicalRepoPath::from_repo_relative(path)
        .map_err(|error| anyhow::anyhow!("invalid file path '{path}': {error}"))?;
    let absolute = Path::new(repo_root).join(canonical.as_str());
    Ok((canonical.as_str().to_owned(), absolute))
}

pub(super) fn resolve_repo_file_path_or_error(
    tool_name: &str,
    repo_root: &str,
    path: &str,
    require_existing_file: bool,
) -> std::result::Result<(String, PathBuf), Box<ToolErrorPayload>> {
    let recovered_candidates = unique_existing_repo_path_candidates(repo_root, path);
    let basename_candidate = unique_basename_repo_path_candidate(repo_root, path);
    let build_candidates = || {
        if !recovered_candidates.is_empty() {
            recovered_candidates.clone()
        } else {
            basename_candidate.clone().into_iter().collect()
        }
    };

    let (canonical, absolute) = match resolve_repo_file_path(repo_root, path) {
        Ok(resolved) => resolved,
        Err(_) => {
            let candidates = build_candidates();
            let message = if candidates.len() > 1 {
                format!(
                    "invalid file path '{path}': Atlas found multiple repo-relative candidates after removing root-like prefixes"
                )
            } else if let Some(candidate) = candidates.first() {
                format!(
                    "invalid file path '{path}': Atlas file tools expect repo-relative paths. Retry with '{}'",
                    candidate.path
                )
            } else {
                format!("invalid file path '{path}': Atlas file tools expect repo-relative paths")
            };
            return Err(Box::new(build_repo_path_error_payload(
                tool_name,
                repo_root,
                path,
                ToolErrorCode::InvalidInput,
                message,
                "Use exact repo-relative file path inside current Atlas repo, then retry.",
                candidates,
            )));
        }
    };

    if require_existing_file && !absolute.is_file() {
        let mut candidates = recovered_candidates;
        let code = if candidates.is_empty() {
            if let Some(candidate) = basename_candidate {
                candidates.push(candidate);
            }
            ToolErrorCode::FileNotFound
        } else {
            ToolErrorCode::InvalidInput
        };
        let message = if candidates.len() > 1 {
            format!(
                "file not found: {canonical}. Atlas found multiple repo-relative candidates after removing root-like prefixes"
            )
        } else if let Some(candidate) = candidates.first() {
            format!(
                "file not found: {canonical}. Retry with repo-relative path '{}'",
                candidate.path
            )
        } else {
            format!("file not found: {canonical}")
        };
        return Err(Box::new(build_repo_path_error_payload(
            tool_name,
            repo_root,
            path,
            code,
            message,
            "Use exact repo-relative file path inside current Atlas repo, then retry.",
            candidates,
        )));
    }

    Ok((canonical, absolute))
}

pub(super) fn repo_path_tool_error_result(
    output_format: OutputFormat,
    payload: ToolErrorPayload,
) -> Result<serde_json::Value> {
    tool_execution_error_value(output_format, &payload)
}

pub(super) fn validate_optional_repo_scope_or_error(
    tool_name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
) -> std::result::Result<(), Box<ToolErrorPayload>> {
    let Some(requested_repo_root) = args
        .and_then(|value| value.get("repo_root"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    let identity = RepoPathIdentity::new(repo_root);
    if requested_repo_root == repo_root
        || requested_repo_root == identity.repo_name
        || requested_repo_root == identity.workspace_root_prefix.trim_end_matches('/')
    {
        return Ok(());
    }

    let mut details = identity.details("");
    details["requested_repo_root"] = serde_json::Value::String(requested_repo_root.to_owned());
    Err(Box::new(
        ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            format!(
                "repo_root '{requested_repo_root}' does not match current Atlas repo '{}'",
                identity.repo_root
            ),
        )
        .with_tool(tool_name)
        .with_retry_guidance(
            "Use current repo root or omit repo_root when repository scope is already unambiguous, then retry.",
        )
        .with_details(details),
    ))
}

pub(super) fn invalid_search_content_regex_error(
    query: &str,
    error: impl std::fmt::Display,
) -> anyhow::Error {
    let escaped_example = r"Command::Context|Context \{";
    anyhow::anyhow!(
        "invalid regex pattern for search_content query '{query}': {error}. \
         search_content keeps is_regex=true strict and does not fall back to literal search. \
         Set is_regex=false for literal text search, or escape regex metacharacters, e.g. {escaped_example}"
    )
}

pub(super) fn discovery_tool_error_result(
    tool_name: &str,
    output_format: OutputFormat,
    error: anyhow::Error,
) -> Result<serde_json::Value> {
    normalize_tool_execution_error(tool_name, output_format, error)
}

// ---------------------------------------------------------------------------
// Generic helpers
// ---------------------------------------------------------------------------

/// Build a `GlobSet` from a slice of pattern strings.
pub(super) fn build_globset(patterns: &[impl AsRef<str>], case_sensitive: bool) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = GlobBuilder::new(p.as_ref())
            .case_insensitive(!case_sensitive)
            .build()
            .with_context(|| format!("invalid glob pattern: {}", p.as_ref()))?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}

pub(super) fn str_arg<'a>(
    args: Option<&'a serde_json::Value>,
    key: &str,
) -> Result<Option<&'a str>> {
    Ok(args.and_then(|a| a.get(key)).and_then(|v| v.as_str()))
}

pub(super) fn u64_arg(args: Option<&serde_json::Value>, key: &str) -> Option<u64> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_u64())
}

pub(super) fn bool_arg(args: Option<&serde_json::Value>, key: &str) -> Option<bool> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_bool())
}

pub(super) fn string_array_arg(args: Option<&serde_json::Value>, key: &str) -> Result<Vec<String>> {
    Ok(args
        .and_then(|a| a.get(key))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

pub(super) fn load_budget_policy(repo_root: &str) -> Result<BudgetPolicy> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    config.budget_policy()
}

pub(super) fn inject_budget_metadata(response: &mut serde_json::Value, budget: &BudgetReport) {
    response["budget_status"] = serde_json::json!(budget.budget_status);
    response["budget_hit"] = serde_json::json!(budget.budget_hit);
    response["budget_name"] = serde_json::json!(&budget.budget_name);
    response["budget_limit"] = serde_json::json!(budget.budget_limit);
    response["budget_observed"] = serde_json::json!(budget.budget_observed);
    response["partial"] = serde_json::json!(budget.partial);
    response["safe_to_answer"] = serde_json::json!(budget.safe_to_answer);
}

pub(super) fn render_tool_result<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    build_tool_result_value(value, output_format)
}
