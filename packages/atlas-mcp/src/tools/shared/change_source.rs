use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedChangeSourceKind {
    WorkingTree,
    Staged,
    Base,
    Files,
}

impl ResolvedChangeSourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WorkingTree => "working_tree",
            Self::Staged => "staged",
            Self::Base => "base",
            Self::Files => "files",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedChangeSourceSelection {
    pub kind: ResolvedChangeSourceKind,
    pub files: Vec<String>,
    pub base: Option<String>,
    pub deprecated_input_fields: Vec<String>,
}

fn change_source_examples(allow_explicit_files: bool) -> Vec<serde_json::Value> {
    let mut examples = Vec::new();
    if allow_explicit_files {
        examples.push(serde_json::json!({
            "change_source": {
                "kind": "files",
                "files": ["src/service.rs"]
            }
        }));
    }
    examples.push(serde_json::json!({
        "change_source": {
            "kind": "base",
            "base": "origin/main"
        }
    }));
    examples.push(serde_json::json!({
        "change_source": {
            "kind": "staged"
        }
    }));
    examples.push(serde_json::json!({
        "change_source": {
            "kind": "working_tree"
        }
    }));
    examples
}

fn accepted_change_source_families(allow_explicit_files: bool) -> Vec<String> {
    let mut families = Vec::new();
    if allow_explicit_files {
        families.push("change_source.kind=files".to_owned());
    }
    families.extend([
        "change_source.kind=base".to_owned(),
        "change_source.kind=staged".to_owned(),
        "change_source.kind=working_tree".to_owned(),
    ]);
    families
}

fn change_source_error_payload(
    tool_name: &str,
    message: impl Into<String>,
    detail: impl Into<String>,
    allow_explicit_files: bool,
    offending_fields: Vec<String>,
    present_mode_families: Vec<String>,
) -> Box<ToolErrorPayload> {
    let examples = change_source_examples(allow_explicit_files);
    let retry_example = examples.first().cloned();
    let accepted_shapes = if allow_explicit_files {
        serde_json::json!([
            { "change_source": { "kind": "files", "files": ["src/service.rs"] } },
            { "change_source": { "kind": "base", "base": "origin/main" } },
            { "change_source": { "kind": "staged" } },
            { "change_source": { "kind": "working_tree" } }
        ])
    } else {
        serde_json::json!([
            { "change_source": { "kind": "base", "base": "origin/main" } },
            { "change_source": { "kind": "staged" } },
            { "change_source": { "kind": "working_tree" } }
        ])
    };
    Box::new(input_shape_error_payload(
        tool_name,
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields,
            normalization_performed: Vec::new(),
            accepted_argument_families: accepted_change_source_families(allow_explicit_files),
            retry_example,
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting change-source selectors".to_owned(),
            ),
            retry_guidance: Some(
                "Provide exactly one change_source selector and retry.".to_owned(),
            ),
            extra_details: Some(serde_json::json!({
                "present_mode_families": present_mode_families,
                "accepted_change_source_shapes": accepted_shapes,
            })),
        },
    ))
}

fn canonicalize_change_source_files(
    repo_root: &str,
    files: &[String],
    tool_name: &str,
    field_name: &str,
    allow_explicit_files: bool,
) -> std::result::Result<Vec<String>, Box<ToolErrorPayload>> {
    files
        .iter()
        .map(|path| {
            let repo_root_path = camino::Utf8Path::new(repo_root);
            atlas_repo::normalize_repo_change_path(repo_root_path, path)
                .map(|resolved| resolved.canonical)
                .with_context(|| format!("invalid explicit file path '{path}'"))
                .map_err(|error| {
                    change_source_error_payload(
                        tool_name,
                        format!("invalid {field_name} path"),
                        error.to_string(),
                        allow_explicit_files,
                        vec![field_name.to_owned()],
                        vec!["files".to_owned()],
                    )
                })
        })
        .collect()
}

pub(crate) fn resolve_change_source_selection(
    tool_name: &str,
    repo_root: &str,
    args: Option<&serde_json::Value>,
    allow_explicit_files: bool,
) -> std::result::Result<ResolvedChangeSourceSelection, Box<ToolErrorPayload>> {
    let change_source_value = args.and_then(|value| value.get("change_source"));
    let change_source_object = change_source_value.and_then(|value| value.as_object());
    let mut offending_legacy_fields = Vec::new();
    if args.is_some_and(|value| value.get("mode").is_some()) {
        offending_legacy_fields.push("mode".to_owned());
    }
    if args.is_some_and(|value| value.get("files").is_some()) {
        offending_legacy_fields.push("files".to_owned());
    }
    if args.is_some_and(|value| value.get("base").is_some()) {
        offending_legacy_fields.push("base".to_owned());
    }
    if args.is_some_and(|value| value.get("staged").is_some()) {
        offending_legacy_fields.push("staged".to_owned());
    }
    if args.is_some_and(|value| value.get("working_tree").is_some()) {
        offending_legacy_fields.push("working_tree".to_owned());
    }
    if !offending_legacy_fields.is_empty() {
        return Err(change_source_error_payload(
            tool_name,
            "legacy change_source fields are no longer supported",
            "Use change_source={ kind: 'files' | 'base' | 'staged' | 'working_tree', ... } and remove top-level mode/files/base/staged/working_tree fields.",
            allow_explicit_files,
            offending_legacy_fields,
            Vec::new(),
        ));
    }

    if change_source_value.is_some() && change_source_object.is_none() {
        return Err(change_source_error_payload(
            tool_name,
            "invalid change_source selector",
            "change_source must be an object with kind=working_tree, kind=staged, kind=base, or kind=files",
            allow_explicit_files,
            vec!["change_source".to_owned()],
            Vec::new(),
        ));
    }

    if let Some(change_source) = change_source_object {
        let kind = change_source
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                change_source_error_payload(
                    tool_name,
                    "change_source.kind is required",
                    "change_source object requires kind=working_tree, kind=staged, kind=base, or kind=files",
                    allow_explicit_files,
                    vec!["change_source.kind".to_owned()],
                    Vec::new(),
                )
            })?;

        return match kind {
            "working_tree" => Ok(ResolvedChangeSourceSelection {
                kind: ResolvedChangeSourceKind::WorkingTree,
                files: Vec::new(),
                base: None,
                deprecated_input_fields: Vec::new(),
            }),
            "staged" => Ok(ResolvedChangeSourceSelection {
                kind: ResolvedChangeSourceKind::Staged,
                files: Vec::new(),
                base: None,
                deprecated_input_fields: Vec::new(),
            }),
            "base" => {
                let base = change_source
                    .get("base")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        change_source_error_payload(
                            tool_name,
                            "change_source.kind='base' requires non-empty change_source.base",
                            "base change_source selector requires change_source.base",
                            allow_explicit_files,
                            vec![
                                "change_source.kind".to_owned(),
                                "change_source.base".to_owned(),
                            ],
                            vec!["base".to_owned()],
                        )
                    })?
                    .to_owned();
                Ok(ResolvedChangeSourceSelection {
                    kind: ResolvedChangeSourceKind::Base,
                    files: Vec::new(),
                    base: Some(base),
                    deprecated_input_fields: Vec::new(),
                })
            }
            "files" => {
                if !allow_explicit_files {
                    return Err(change_source_error_payload(
                        tool_name,
                        "invalid change_source.kind 'files'",
                        "this tool does not accept change_source.kind='files'",
                        allow_explicit_files,
                        vec!["change_source.kind".to_owned()],
                        vec!["files".to_owned()],
                    ));
                }
                let raw_files = change_source
                    .get("files")
                    .and_then(|value| value.as_array())
                    .ok_or_else(|| {
                        change_source_error_payload(
                            tool_name,
                            "change_source.kind='files' requires non-empty change_source.files",
                            "files change_source selector requires change_source.files array",
                            allow_explicit_files,
                            vec![
                                "change_source.kind".to_owned(),
                                "change_source.files".to_owned(),
                            ],
                            vec!["files".to_owned()],
                        )
                    })?
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>();
                if raw_files.is_empty() {
                    return Err(change_source_error_payload(
                        tool_name,
                        "change_source.kind='files' requires non-empty change_source.files",
                        "files change_source selector requires non-empty change_source.files array",
                        allow_explicit_files,
                        vec![
                            "change_source.kind".to_owned(),
                            "change_source.files".to_owned(),
                        ],
                        vec!["files".to_owned()],
                    ));
                }
                let files = canonicalize_change_source_files(
                    repo_root,
                    &raw_files,
                    tool_name,
                    "change_source.files",
                    allow_explicit_files,
                )?;
                Ok(ResolvedChangeSourceSelection {
                    kind: ResolvedChangeSourceKind::Files,
                    files,
                    base: None,
                    deprecated_input_fields: Vec::new(),
                })
            }
            other => Err(change_source_error_payload(
                tool_name,
                format!("invalid change_source.kind '{other}'"),
                "change_source.kind must be one of: working_tree, staged, base, files",
                allow_explicit_files,
                vec!["change_source.kind".to_owned()],
                Vec::new(),
            )),
        };
    }

    Err(change_source_error_payload(
        tool_name,
        "change_source is required",
        "Provide change_source={ kind: 'files' | 'base' | 'staged' | 'working_tree', ... }.",
        allow_explicit_files,
        vec!["change_source".to_owned()],
        Vec::new(),
    ))
}
