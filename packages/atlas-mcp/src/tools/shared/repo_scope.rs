use super::*;

pub(crate) const MAX_MULTI_REPO_SELECTION: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct RepoScopeSelection {
    pub repo_ids: Vec<String>,
    pub registrations: Vec<RepoRegistration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedRepoScopeKind {
    Current,
    RepoId,
    All,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedRepoScopeSelection {
    #[allow(dead_code)]
    pub kind: ResolvedRepoScopeKind,
    pub selection: Option<RepoScopeSelection>,
    pub deprecated_input_fields: Vec<String>,
}

pub(crate) fn repo_aliases_by_id(repo_root: &str) -> BTreeMap<String, String> {
    RepoRegistry::load_or_bootstrap(Utf8Path::new(repo_root))
        .map(|registry| {
            registry
                .registrations
                .into_iter()
                .map(|entry| (entry.repo_id, entry.display_alias))
                .collect()
        })
        .unwrap_or_default()
}

fn repo_scope_error_payload(
    tool_name: &str,
    message: impl Into<String>,
    detail: impl Into<String>,
    offending_fields: Vec<String>,
    retry_example: serde_json::Value,
) -> Box<ToolErrorPayload> {
    Box::new(input_shape_error_payload(
        tool_name,
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields,
            normalization_performed: Vec::new(),
            accepted_argument_families: vec![
                "repo_scope.kind=current".to_owned(),
                "repo_scope.kind=repo_id".to_owned(),
                "repo_scope.kind=all".to_owned(),
            ],
            retry_example: Some(retry_example),
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting repository scope selectors".to_owned(),
            ),
            retry_guidance: Some("Provide exactly one repo scope selector, then retry.".to_owned()),
            extra_details: Some(serde_json::json!({
                "accepted_repo_scope_shapes": [
                    { "repo_scope": { "kind": "current" } },
                    { "repo_scope": { "kind": "repo_id", "repo_id": "atlas-core" } },
                    { "repo_scope": { "kind": "all" } }
                ]
            })),
        },
    ))
}

pub(crate) fn resolve_repo_scope_selection(
    tool_name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
) -> std::result::Result<ResolvedRepoScopeSelection, Box<ToolErrorPayload>> {
    let repo_scope_value = args.and_then(|value| value.get("repo_scope"));
    let repo_scope_object = repo_scope_value.and_then(|value| value.as_object());

    if args.is_some_and(|value| value.get("repo_id").is_some() || value.get("all_repos").is_some())
    {
        return Err(repo_scope_error_payload(
            tool_name,
            "legacy repo scope fields are no longer supported",
            "Use repo_scope={ kind: 'current' | 'repo_id' | 'all' } and remove top-level repo_id/all_repos fields.",
            vec!["repo_id".to_owned(), "all_repos".to_owned()],
            serde_json::json!({ "repo_scope": { "kind": "current" } }),
        ));
    }

    if repo_scope_value.is_some() && repo_scope_object.is_none() {
        return Err(repo_scope_error_payload(
            tool_name,
            "invalid repo_scope selector",
            "repo_scope must be an object with required kind field",
            vec!["repo_scope".to_owned()],
            serde_json::json!({ "repo_scope": { "kind": "current" } }),
        ));
    }

    let (kind, target_repo_id, deprecated_input_fields) = if let Some(scope) = repo_scope_object {
        let kind = scope
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                repo_scope_error_payload(
                    tool_name,
                    "repo_scope.kind is required",
                    "repo_scope object requires kind=current, kind=repo_id, or kind=all",
                    vec!["repo_scope.kind".to_owned()],
                    serde_json::json!({ "repo_scope": { "kind": "current" } }),
                )
            })?;
        match kind {
            "current" => (ResolvedRepoScopeKind::Current, None, Vec::new()),
            "repo_id" => {
                let repo_id = scope
                    .get("repo_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        repo_scope_error_payload(
                            tool_name,
                            "repo_scope.kind='repo_id' requires non-empty repo_id",
                            "repo_scope repo_id selector requires repo_scope.repo_id",
                            vec!["repo_scope.kind".to_owned(), "repo_scope.repo_id".to_owned()],
                            serde_json::json!({ "repo_scope": { "kind": "repo_id", "repo_id": "atlas-core" } }),
                        )
                    })?;
                (
                    ResolvedRepoScopeKind::RepoId,
                    Some(repo_id.to_owned()),
                    Vec::new(),
                )
            }
            "all" => (ResolvedRepoScopeKind::All, None, Vec::new()),
            other => {
                return Err(repo_scope_error_payload(
                    tool_name,
                    format!("invalid repo_scope.kind '{other}'"),
                    "repo_scope.kind must be one of: current, repo_id, all",
                    vec!["repo_scope.kind".to_owned()],
                    serde_json::json!({ "repo_scope": { "kind": "current" } }),
                ));
            }
        }
    } else {
        (ResolvedRepoScopeKind::Current, None, Vec::new())
    };

    if kind == ResolvedRepoScopeKind::Current {
        return Ok(ResolvedRepoScopeSelection {
            kind,
            selection: None,
            deprecated_input_fields,
        });
    }

    let registry = RepoRegistry::load(Utf8Path::new(repo_root)).with_context(|| {
        "repo registry missing; run `atlas init` or `atlas repo sync` before multi-repo MCP queries"
    }).map_err(|error| {
        repo_scope_error_payload(
            tool_name,
            "repo registry missing for requested repo scope",
            error.to_string(),
            vec!["repo_scope".to_owned()],
            serde_json::json!({ "repo_scope": { "kind": "current" } }),
        )
    })?;

    let selection = if kind == ResolvedRepoScopeKind::All {
        let registrations: Vec<RepoRegistration> = registry
            .registrations
            .into_iter()
            .filter(|entry| entry.enabled)
            .filter(|entry| phase1_multi_repo_supported(entry.relationship.kind))
            .collect();
        if registrations.len() > MAX_MULTI_REPO_SELECTION {
            return Err(repo_scope_error_payload(
                tool_name,
                "all repo scope exceeds max supported fan-out",
                format!(
                    "all repo scope exceeds max supported repo fan-out ({MAX_MULTI_REPO_SELECTION})"
                ),
                vec!["repo_scope".to_owned()],
                serde_json::json!({ "repo_scope": { "kind": "current" } }),
            ));
        }
        let repo_ids = registrations
            .iter()
            .map(|entry| entry.repo_id.clone())
            .collect();
        RepoScopeSelection {
            repo_ids,
            registrations,
        }
    } else {
        let target = target_repo_id.expect("repo_id target required");
        let registration = registry
            .registrations
            .into_iter()
            .find(|entry| entry.repo_id == target)
            .ok_or_else(|| {
                repo_scope_error_payload(
                    tool_name,
                    format!("repo id '{target}' is not registered"),
                    format!("repo id '{target}' is not registered"),
                    vec!["repo_scope.repo_id".to_owned()],
                    serde_json::json!({ "repo_scope": { "kind": "current" } }),
                )
            })?;
        if !registration.enabled {
            return Err(repo_scope_error_payload(
                tool_name,
                format!("repo id '{target}' is disabled"),
                format!("repo id '{target}' is disabled"),
                vec!["repo_scope.repo_id".to_owned()],
                serde_json::json!({ "repo_scope": { "kind": "current" } }),
            ));
        }
        RepoScopeSelection {
            repo_ids: vec![registration.repo_id.clone()],
            registrations: vec![registration],
        }
    };

    Ok(ResolvedRepoScopeSelection {
        kind,
        selection: Some(selection),
        deprecated_input_fields,
    })
}
