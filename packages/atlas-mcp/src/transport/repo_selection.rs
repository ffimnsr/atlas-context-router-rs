//! Repo-selection helpers for explicit repo-bound MCP sessions.

use anyhow::{Context, Result};
use atlas_repo::{RepoRegistry, canonical_filesystem_path, find_repo_root};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use super::types::{ActiveRepoContext, RepoResolutionState};

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
    repo_resolution: &RepoResolutionState,
) -> Result<Option<ActiveRepoContext>> {
    let Some(args) = args.and_then(Value::as_object) else {
        return Ok(None);
    };
    let repo_id = args
        .get("repo_id")
        .or_else(|| args.get("repoId"))
        .and_then(Value::as_str);
    let repo_root = args
        .get("repo_root")
        .or_else(|| args.get("repoRoot"))
        .and_then(Value::as_str);
    if repo_id.is_none() && repo_root.is_none() {
        return Ok(None);
    }

    let base_context = repo_resolution
        .active
        .as_ref()
        .or(repo_resolution.startup.as_ref());

    match (repo_id, repo_root) {
        (Some(repo_id), Some(repo_root)) => {
            let repo_root = canonical_repo_root_selector(repo_root)?;
            let repo_context = repo_context_for_repo_id(repo_id, base_context)?;
            anyhow::ensure!(
                repo_context.repo_root == repo_root,
                "repo selector mismatch: repo_id='{repo_id}' resolves to '{}' not '{repo_root}'",
                repo_context.repo_root,
            );
            Ok(Some(repo_context))
        }
        (Some(repo_id), None) => repo_context_for_repo_id(repo_id, base_context).map(Some),
        (None, Some(repo_root)) => resolve_repo_root_context(repo_root, base_context).map(Some),
        (None, None) => Ok(None),
    }
}

fn repo_context_for_repo_id(
    repo_id: &str,
    base_context: Option<&ActiveRepoContext>,
) -> Result<ActiveRepoContext> {
    let Some(base_context) = base_context else {
        anyhow::bail!(
            "repo_id selector requires startup repo context; start Atlas with --repo or pass repo_root first"
        );
    };
    let registry =
        RepoRegistry::load(Utf8Path::new(&base_context.repo_root)).with_context(|| {
            format!(
                "repo_id selector requires registry at {}/.atlas/{}",
                base_context.repo_root,
                atlas_repo::REPO_REGISTRY_FILE_NAME
            )
        })?;
    let registration = registry
        .registrations
        .into_iter()
        .find(|entry| entry.repo_id == repo_id)
        .with_context(|| format!("repo_id selector '{repo_id}' is not registered"))?;
    anyhow::ensure!(
        registration.enabled,
        "repo_id selector '{repo_id}' is disabled"
    );
    Ok(ActiveRepoContext {
        repo_root: registration.root.to_string(),
        db_path: base_context.db_path.clone(),
    })
}

fn resolve_repo_root_context(
    repo_root: &str,
    base_context: Option<&ActiveRepoContext>,
) -> Result<ActiveRepoContext> {
    let canonical = canonical_repo_root_selector(repo_root)?;
    if let Some(base_context) = base_context
        && let Ok(registry) =
            RepoRegistry::load_or_bootstrap(Utf8Path::new(&base_context.repo_root))
        && registry
            .registrations
            .iter()
            .any(|entry| entry.root.as_str() == canonical)
    {
        return Ok(ActiveRepoContext {
            repo_root: canonical,
            db_path: base_context.db_path.clone(),
        });
    }
    Ok(active_repo_context(&canonical))
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

        let context = explicit_repo_context_from_tool_args(
            Some(&serde_json::json!({
                "repo_root": repo.path().join("src").to_string_lossy().into_owned()
            })),
            &crate::transport::types::RepoResolutionState {
                startup: None,
                active: None,
                active_selection_source: None,
                candidate_roots: None,
                dynamic_roots: false,
            },
        )
        .expect("explicit repo context")
        .expect("repo context");

        assert_eq!(
            context.repo_root,
            repo.path().canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn explicit_repo_context_from_tool_args_resolves_repo_id_against_registry() {
        let root = tempfile::tempdir().expect("root tempdir");
        let sibling = tempfile::tempdir().expect("sibling tempdir");
        std::fs::create_dir_all(root.path().join(".atlas")).expect("atlas dir");
        let sibling_root = sibling.path().canonicalize().expect("canonical sibling");
        let sibling_root = Utf8PathBuf::from_path_buf(sibling_root).expect("utf8 sibling");
        let registry = atlas_repo::RepoRegistry {
            schema_version: atlas_repo::REPO_REGISTRY_SCHEMA_VERSION,
            root_repo_id: "root_repo".to_owned(),
            registrations: vec![atlas_repo::RepoRegistration {
                repo_id: "repo_demo".to_owned(),
                root: sibling_root.clone(),
                display_alias: "../sibling".to_owned(),
                vcs: atlas_repo::VcsMetadata {
                    head: None,
                    default_branch: None,
                    remote_url: None,
                },
                relationship: atlas_repo::RepoRelationship {
                    kind: atlas_repo::RepoRelationshipKind::Manual,
                    parent_repo_id: None,
                    parent_path: None,
                },
                trust_state: atlas_repo::TrustState::Trusted,
                enabled: true,
                include_globs: None,
                exclude_globs: None,
                dependencies: Vec::new(),
            }],
            warnings: Vec::new(),
        };
        registry
            .save(Utf8Path::from_path(root.path()).expect("utf8 root"))
            .expect("save registry");

        let base_root = root.path().canonicalize().expect("canonical root");
        let base_root = base_root.to_string_lossy().into_owned();
        let context = explicit_repo_context_from_tool_args(
            Some(&serde_json::json!({ "repo_id": "repo_demo" })),
            &crate::transport::types::RepoResolutionState {
                startup: Some(ActiveRepoContext {
                    repo_root: base_root.clone(),
                    db_path: root.path().join("atlas.db").to_string_lossy().into_owned(),
                }),
                active: None,
                active_selection_source: None,
                candidate_roots: None,
                dynamic_roots: false,
            },
        )
        .expect("repo_id context")
        .expect("resolved repo context");

        assert_eq!(context.repo_root, sibling_root.to_string());
        assert_eq!(
            context.db_path,
            root.path().join("atlas.db").to_string_lossy()
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
