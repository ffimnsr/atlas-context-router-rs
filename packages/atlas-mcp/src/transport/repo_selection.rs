//! Dynamic repo-selection helpers for multi-root MCP sessions.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use atlas_repo::{CanonicalRepoPath, canonical_filesystem_path, find_repo_root};
use camino::Utf8PathBuf;
use serde_json::Value;
use url::Url;

use super::types::ActiveRepoContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoSelectionSource {
    ExplicitCli,
    SingleRoot,
    ToolArgInference,
    CachedActiveRoot,
    ClientHint,
}

impl RepoSelectionSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitCli => "explicit_cli",
            Self::SingleRoot => "single_root",
            Self::ToolArgInference => "tool_arg_inference",
            Self::CachedActiveRoot => "cached_active_root",
            Self::ClientHint => "client_hint",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoSelectionOutcome {
    pub(crate) repo_context: ActiveRepoContext,
    pub(crate) selection_source: RepoSelectionSource,
    pub(crate) candidate_roots: Option<Vec<String>>,
}

pub(crate) fn parse_root_candidates(roots: Option<&Value>) -> Result<Vec<String>> {
    let roots = roots
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("roots/list response missing result.roots array"))?;
    let mut candidates = BTreeSet::new();
    for root in roots {
        let Some(uri) = root.get("uri").and_then(Value::as_str) else {
            continue;
        };
        candidates.insert(root_uri_to_candidate_repo_root(uri)?);
    }
    if candidates.is_empty() {
        anyhow::bail!("roots/list returned no usable file roots");
    }
    Ok(candidates.into_iter().collect())
}

pub(crate) fn select_repo_from_candidates(
    candidates: &[String],
    tool_name: Option<&str>,
    args: Option<&Value>,
) -> Result<RepoSelectionOutcome> {
    match candidates.len() {
        0 => anyhow::bail!("roots/list returned no usable file roots"),
        1 => Ok(RepoSelectionOutcome {
            repo_context: active_repo_context(&candidates[0]),
            selection_source: RepoSelectionSource::SingleRoot,
            candidate_roots: Some(candidates.to_vec()),
        }),
        _ => {
            let tool_name = tool_name.unwrap_or("tools/call");
            let paths = extract_tool_repo_paths(tool_name, args)?;
            if paths.is_empty() {
                anyhow::bail!(
                    "multiple workspace roots available and tool `{tool_name}` provided no file evidence; pass --repo or use file-bearing arguments to disambiguate: {}",
                    candidates.join(", ")
                );
            }
            let matching = candidates
                .iter()
                .filter(|root| root_contains_all_paths(root, &paths))
                .cloned()
                .collect::<Vec<_>>();
            match matching.len() {
                1 => Ok(RepoSelectionOutcome {
                    repo_context: active_repo_context(&matching[0]),
                    selection_source: RepoSelectionSource::ToolArgInference,
                    candidate_roots: Some(candidates.to_vec()),
                }),
                0 => anyhow::bail!(
                    "none of the advertised workspace roots contain all repo-relative paths for tool `{tool_name}`: paths={} candidates={}",
                    paths.join(", "),
                    candidates.join(", ")
                ),
                _ => anyhow::bail!(
                    "multiple workspace roots match tool `{tool_name}` arguments; same relative paths exist in more than one root: paths={} candidates={}",
                    paths.join(", "),
                    matching.join(", ")
                ),
            }
        }
    }
}

fn active_repo_context(repo_root: &str) -> ActiveRepoContext {
    ActiveRepoContext {
        db_path: atlas_engine::paths::default_db_path(repo_root),
        repo_root: repo_root.to_owned(),
    }
}

pub(crate) fn validate_hinted_root_uri(candidates: &[String], hint_uri: &str) -> Result<String> {
    let hinted_root = uri_to_candidate_repo_root(hint_uri, "active-root hint")?;
    if candidates.iter().any(|candidate| candidate == &hinted_root) {
        return Ok(hinted_root);
    }
    anyhow::bail!(
        "active-root hint URI does not match any advertised workspace root: {hint_uri}; candidates={}",
        candidates.join(", ")
    )
}

pub(crate) fn request_active_root_hint_uri(request: &Value) -> Option<String> {
    request
        .get("_meta")
        .and_then(|value| value.get("atlas"))
        .and_then(|value| value.get("activeRootUri"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            request
                .get("params")
                .and_then(|value| value.get("_meta"))
                .and_then(|value| value.get("atlas"))
                .and_then(|value| value.get("activeRootUri"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

pub(crate) fn preferred_root_hint_uri(meta: Option<&Value>) -> Option<String> {
    meta.and_then(|value| value.get("atlas")).and_then(|atlas| {
        atlas
            .get("preferredRootUri")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                atlas
                    .get("activeRootUri")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
    })
}

fn root_uri_to_candidate_repo_root(uri: &str) -> Result<String> {
    uri_to_candidate_repo_root(uri, "root")
}

fn uri_to_candidate_repo_root(uri: &str, label: &str) -> Result<String> {
    let url = Url::parse(uri).with_context(|| format!("invalid {label} URI: {uri}"))?;
    let path = url
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("{label} URI must use file:// scheme: {uri}"))?;
    let utf8 = Utf8PathBuf::from_path_buf(path)
        .map_err(|path| anyhow::anyhow!("{label} path is not valid UTF-8: {}", path.display()))?;
    let start = if utf8.is_file() {
        utf8.parent()
            .map(|parent| parent.to_owned())
            .unwrap_or_else(|| utf8.clone())
    } else {
        utf8.clone()
    };
    let repo_root = find_repo_root(start.as_path()).unwrap_or(start);
    Ok(canonical_filesystem_path(repo_root.as_path())?.into_string())
}

fn extract_tool_repo_paths(tool_name: &str, args: Option<&Value>) -> Result<Vec<String>> {
    let Some(args) = args else {
        return Ok(Vec::new());
    };
    let raw_paths = match tool_name {
        "cross_file_links"
        | "get_docs_section"
        | "read_file_excerpt"
        | "read_file_around_match" => optional_string(args, "file")
            .into_iter()
            .collect::<Vec<_>>(),
        "get_context" => {
            let files = string_array(args, "files");
            if !files.is_empty() {
                files
            } else {
                optional_string(args, "file")
                    .into_iter()
                    .collect::<Vec<_>>()
            }
        }
        "get_review_context"
        | "get_impact_radius"
        | "concept_clusters"
        | "build_or_update_graph" => string_array(args, "files"),
        _ => Vec::new(),
    };
    canonicalize_repo_paths(raw_paths)
}

fn canonicalize_repo_paths(paths: Vec<String>) -> Result<Vec<String>> {
    let mut canonical = BTreeSet::new();
    for path in paths {
        let canonical_path = CanonicalRepoPath::from_repo_relative(&path)
            .with_context(|| format!("invalid repo-relative path '{path}'"))?;
        canonical.insert(canonical_path.as_str().to_owned());
    }
    Ok(canonical.into_iter().collect())
}

fn optional_string(args: &Value, field: &str) -> Option<String> {
    args.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn string_array(args: &Value, field: &str) -> Vec<String> {
    args.get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn root_contains_all_paths(root: &str, paths: &[String]) -> bool {
    paths.iter().all(|path| Path::new(root).join(path).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_root_candidates_canonicalizes_and_dedupes() {
        let repo = tempfile::tempdir().expect("repo tempdir");
        std::fs::create_dir_all(repo.path().join("src")).expect("create src");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create git dir");
        std::fs::write(repo.path().join("src/lib.rs"), "fn main() {}\n").expect("write file");

        let root_uri = Url::from_directory_path(repo.path())
            .expect("repo root uri")
            .to_string();
        let file_uri = Url::from_file_path(repo.path().join("src/lib.rs"))
            .expect("repo file uri")
            .to_string();
        let roots = serde_json::json!({
            "roots": [
                { "uri": root_uri },
                { "uri": file_uri }
            ]
        });

        let candidates = parse_root_candidates(roots.get("roots")).expect("parse candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0],
            repo.path().canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn single_root_reports_single_root_source() {
        let outcome = select_repo_from_candidates(&["/tmp/repo".to_owned()], None, None)
            .expect("single root outcome");
        assert_eq!(outcome.selection_source, RepoSelectionSource::SingleRoot);
        assert_eq!(outcome.repo_context.repo_root, "/tmp/repo");
        assert_eq!(outcome.selection_source.as_str(), "single_root");
    }

    #[test]
    fn multi_root_file_inference_reports_tool_arg_source() {
        let repo_a = tempfile::tempdir().expect("repo a tempdir");
        let repo_b = tempfile::tempdir().expect("repo b tempdir");
        std::fs::create_dir_all(repo_b.path().join("src")).expect("create src");
        std::fs::write(repo_b.path().join("src/guide.rs"), "pub fn guide() {}\n")
            .expect("write file");

        let candidates = vec![
            repo_a
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            repo_b
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ];
        let outcome = select_repo_from_candidates(
            &candidates,
            Some("read_file_excerpt"),
            Some(&serde_json::json!({"file":"src/guide.rs"})),
        )
        .expect("tool arg outcome");
        assert_eq!(
            outcome.selection_source,
            RepoSelectionSource::ToolArgInference
        );
        assert_eq!(outcome.repo_context.repo_root, candidates[1]);
        assert_eq!(outcome.selection_source.as_str(), "tool_arg_inference");
    }
}
