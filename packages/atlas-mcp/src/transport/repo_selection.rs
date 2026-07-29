//! Repo-selection helpers for explicit repo-bound MCP sessions.

use anyhow::{Context, Result};
use atlas_repo::{canonical_filesystem_path, find_repo_root};
use camino::Utf8PathBuf;
use serde_json::Value;

use super::types::ActiveRepoContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoSelectionSource {
    ExplicitCli,
    ExplicitRequest,
    CachedActiveRoot,
}

impl RepoSelectionSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitCli => "explicit_cli",
            Self::ExplicitRequest => "explicit_request",
            Self::CachedActiveRoot => "cached_active_root",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoSelectionOutcome {
    pub(crate) repo_context: ActiveRepoContext,
    pub(crate) selection_source: RepoSelectionSource,
    pub(crate) candidate_roots: Option<Vec<String>>,
}

pub(crate) fn explicit_repo_context_from_tool_args(
    args: Option<&Value>,
) -> Result<Option<ActiveRepoContext>> {
    let Some(args) = args.and_then(Value::as_object) else {
        return Ok(None);
    };
    if let Some(repo_id) = args
        .get("repo_id")
        .or_else(|| args.get("repoId"))
        .and_then(Value::as_str)
    {
        anyhow::bail!(
            "unsupported repo selector repo_id='{repo_id}'; pass arguments.repo_root with canonical repo path"
        );
    }
    let Some(repo_root) = args
        .get("repo_root")
        .or_else(|| args.get("repoRoot"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    Ok(Some(active_repo_context(&canonical_repo_root_selector(
        repo_root,
    )?)))
}

pub(crate) fn strip_repo_selector_fields(mut args: Value) -> Value {
    if let Some(object) = args.as_object_mut() {
        object.remove("repo_root");
        object.remove("repoRoot");
        object.remove("repo_id");
        object.remove("repoId");
    }
    args
}

fn active_repo_context(repo_root: &str) -> ActiveRepoContext {
    ActiveRepoContext {
        db_path: atlas_engine::paths::default_db_path(repo_root),
        repo_root: repo_root.to_owned(),
    }
}

fn canonical_repo_root_selector(repo_root: &str) -> Result<String> {
    let utf8 = Utf8PathBuf::from(repo_root);
    let start = if utf8.is_file() {
        utf8.parent()
            .map(|parent| parent.to_owned())
            .unwrap_or_else(|| utf8.clone())
    } else {
        utf8.clone()
    };
    let repo_root = find_repo_root(start.as_path()).unwrap_or(start);
    canonical_filesystem_path(repo_root.as_path())
        .with_context(|| format!("invalid repo_root selector '{repo_root}'"))
        .map(|path| path.into_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_request_reports_expected_source_name() {
        assert_eq!(
            RepoSelectionSource::ExplicitRequest.as_str(),
            "explicit_request"
        );
    }

    #[test]
    fn explicit_repo_context_from_tool_args_canonicalizes_repo_root() {
        let repo = tempfile::tempdir().expect("repo tempdir");
        std::fs::create_dir_all(repo.path().join("src")).expect("create src");
        std::fs::create_dir_all(repo.path().join(".git")).expect("create git dir");
        std::fs::write(repo.path().join("src/lib.rs"), "fn main() {}\n").expect("write file");

        let context = explicit_repo_context_from_tool_args(Some(&serde_json::json!({
            "repo_root": repo.path().join("src").to_string_lossy().into_owned()
        })))
        .expect("explicit repo context")
        .expect("repo context");

        assert_eq!(
            context.repo_root,
            repo.path().canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn strip_repo_selector_fields_removes_transport_only_keys() {
        let args = strip_repo_selector_fields(serde_json::json!({
            "repo_root": "/tmp/repo",
            "repoId": "abc",
            "text": "compute"
        }));
        assert_eq!(args, serde_json::json!({ "text": "compute" }));
    }
}
