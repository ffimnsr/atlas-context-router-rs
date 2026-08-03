//! Repo-selection helpers shared by rmcp transport adapters.

use serde_json::json;

use super::repo_selection::{
    RepoSelectionOutcome, RepoSelectionSource, explicit_repo_context_from_tool_args,
};
use super::types::{ActiveRepoContext, RepoResolutionState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoSelectionFailureKind {
    NoRepoContextAvailable,
    InvalidExplicitRepoSelector,
}

impl RepoSelectionFailureKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoRepoContextAvailable => "no_repo_context_available",
            Self::InvalidExplicitRepoSelector => "invalid_explicit_repo_selector",
        }
    }
}

pub(crate) struct RepoSelectionError {
    pub(crate) kind: RepoSelectionFailureKind,
    pub(crate) message: String,
    pub(crate) candidate_roots: Vec<String>,
    pub(crate) selection_attempts: Vec<String>,
    pub(crate) selection_source: Option<RepoSelectionSource>,
    pub(crate) tool_name: String,
    pub(crate) recommended_fix: String,
    pub(crate) session_mode: &'static str,
    pub(crate) active_repo_root: Option<String>,
}

impl RepoSelectionError {
    pub(crate) fn message(&self) -> String {
        self.message.clone()
    }

    pub(crate) fn error_data(&self) -> serde_json::Value {
        json!({
            "atlas_repo_selection": {
                "failure_kind": self.kind.as_str(),
                "candidate_roots": self.candidate_roots,
                "selection_attempts": self.selection_attempts,
                "selectionSource": self.selection_source.map(|source| source.as_str()),
                "tool": self.tool_name,
                "recommended_fix": self.recommended_fix,
                "session_mode": self.session_mode,
                "active_repo_root": self.active_repo_root,
            }
        })
    }
}

pub(crate) struct ToolRepoResolutionContext;

pub(crate) fn resolve_repo_context_for_tool_call(
    repo_resolution: &RepoResolutionState,
    tool_name: Option<&str>,
    tool_args: Option<&serde_json::Value>,
    _ctx: ToolRepoResolutionContext,
) -> std::result::Result<RepoSelectionOutcome, Box<RepoSelectionError>> {
    let tool_name = tool_name.unwrap_or("tools/call");
    let active_repo_root = repo_resolution
        .active
        .as_ref()
        .or(repo_resolution.startup.as_ref())
        .map(|context| context.repo_root.clone());

    if let Some(repo_context) = explicit_repo_context_from_tool_args(tool_args, repo_resolution).map_err(|error| {
        Box::new(RepoSelectionError {
            kind: RepoSelectionFailureKind::InvalidExplicitRepoSelector,
            message: error.to_string(),
            candidate_roots: Vec::new(),
            selection_attempts: vec!["explicit_request_repo_selector".to_owned()],
            selection_source: Some(RepoSelectionSource::ExplicitRequest),
            tool_name: tool_name.to_owned(),
            recommended_fix: "Pass arguments.repo_root as canonical repo path, pass registered arguments.repo_id, or start Atlas with --repo for fixed-mode MCP.".to_owned(),
            session_mode: "fixed",
            active_repo_root: active_repo_root.clone(),
        })
    })? {
        return Ok(RepoSelectionOutcome {
            repo_context,
            selection_source: RepoSelectionSource::ExplicitRequest,
            candidate_roots: repo_resolution.candidate_roots.clone(),
        });
    }

    if let Some(active) = repo_resolution.active.clone() {
        return Ok(RepoSelectionOutcome {
            repo_context: active,
            selection_source: RepoSelectionSource::CachedActiveRoot,
            candidate_roots: repo_resolution.candidate_roots.clone(),
        });
    }

    if repo_resolution.dynamic_roots {
        if let Some(candidate_roots) = repo_resolution.candidate_roots.clone() {
            if candidate_roots.len() > 1 {
                return Err(Box::new(RepoSelectionError {
                    kind: RepoSelectionFailureKind::NoRepoContextAvailable,
                    message: "atlas repo context ambiguous across multiple workspace roots; include arguments.repo_root or arguments.repo_id".to_owned(),
                    candidate_roots,
                    selection_attempts: vec![
                        "cached_active_root".to_owned(),
                        "single_advertised_root".to_owned(),
                        "explicit_request_repo_selector".to_owned(),
                    ],
                    selection_source: None,
                    tool_name: tool_name.to_owned(),
                    recommended_fix: "Advertise one root, or pass arguments.repo_root / registered arguments.repo_id explicitly.".to_owned(),
                    session_mode: "dynamic",
                    active_repo_root,
                }));
            }
            if let Some(only_root) = candidate_roots.first() {
                return Ok(RepoSelectionOutcome {
                    repo_context: ActiveRepoContext {
                        repo_root: only_root.clone(),
                        db_path: atlas_engine::paths::default_db_path(only_root),
                    },
                    selection_source: RepoSelectionSource::CachedActiveRoot,
                    candidate_roots: Some(candidate_roots.clone()),
                });
            }
        }
        return Err(Box::new(RepoSelectionError {
            kind: RepoSelectionFailureKind::NoRepoContextAvailable,
            message: "atlas repo context missing; advertise client roots or include arguments.repo_root/repo_id".to_owned(),
            candidate_roots: Vec::new(),
            selection_attempts: vec![
                "cached_active_root".to_owned(),
                "single_advertised_root".to_owned(),
                "explicit_request_repo_selector".to_owned(),
            ],
            selection_source: None,
            tool_name: tool_name.to_owned(),
            recommended_fix: "Advertise one client root, or pass arguments.repo_root / registered arguments.repo_id explicitly.".to_owned(),
            session_mode: "dynamic",
            active_repo_root,
        }));
    }

    if let Some(startup) = repo_resolution.startup.clone() {
        return Ok(RepoSelectionOutcome {
            repo_context: startup,
            selection_source: RepoSelectionSource::ExplicitCli,
            candidate_roots: repo_resolution.candidate_roots.clone(),
        });
    }

    Err(Box::new(RepoSelectionError {
        kind: RepoSelectionFailureKind::NoRepoContextAvailable,
        message: "atlas repo context missing; pass --repo or include arguments.repo_root/repo_id".to_owned(),
        candidate_roots: Vec::new(),
        selection_attempts: vec!["explicit_cli".to_owned(), "explicit_request_repo_selector".to_owned()],
        selection_source: None,
        tool_name: tool_name.to_owned(),
        recommended_fix: "Start Atlas with --repo, or pass arguments.repo_root / registered arguments.repo_id for repo-bound tools.".to_owned(),
        session_mode: "fixed",
        active_repo_root,
    }))
}

pub(crate) fn annotate_tool_result_with_repo_selection(
    result: &mut serde_json::Value,
    repo_root: &str,
    selection_source: RepoSelectionSource,
    dynamic_mode: bool,
) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    let meta_value = object.entry("_meta").or_insert_with(|| json!({}));
    let Some(meta) = meta_value.as_object_mut() else {
        return;
    };

    meta.insert("atlas:repoRoot".to_owned(), json!(repo_root));
    meta.insert(
        "atlas:repoSelection".to_owned(),
        json!({
            "selectionSource": selection_source.as_str(),
            "dynamicMode": dynamic_mode,
        }),
    );
}
