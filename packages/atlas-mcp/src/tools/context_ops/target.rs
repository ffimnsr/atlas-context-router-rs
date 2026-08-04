use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GetContextTargetKind {
    Query,
    File,
    Files,
}

impl GetContextTargetKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::File => "file",
            Self::Files => "files",
        }
    }
}

pub(super) struct ParsedGetContextTarget {
    pub(super) kind: GetContextTargetKind,
    pub(super) target: ContextTarget,
    pub(super) parsed_request: Option<atlas_core::model::ContextRequest>,
    pub(super) query: Option<String>,
    pub(super) file: Option<String>,
    pub(super) files: Vec<String>,
    pub(super) deprecated_input_fields: Vec<String>,
}

pub(super) fn get_context_target_error(
    message: impl Into<String>,
    detail: impl Into<String>,
    offending_fields: Vec<&str>,
    retry_example: Value,
) -> Box<ToolErrorPayload> {
    Box::new(input_shape_error_payload(
        "get_context",
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields: offending_fields.into_iter().map(str::to_owned).collect(),
            normalization_performed: Vec::new(),
            accepted_argument_families: vec![
                "target.kind=query".to_owned(),
                "target.kind=file".to_owned(),
                "target.kind=files".to_owned(),
            ],
            retry_example: Some(retry_example),
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting get_context target selectors"
                    .to_owned(),
            ),
            retry_guidance: Some(
                "Provide exactly one get_context target selector and retry.".to_owned(),
            ),
            extra_details: Some(json!({
                "accepted_target_shapes": [
                    { "target": { "kind": "query", "query": "handle_request" } },
                    { "target": { "kind": "file", "file": "src/lib.rs" } },
                    { "target": { "kind": "files", "files": ["src/lib.rs"] } }
                ]
            })),
        },
    ))
}

pub(super) fn context_query_looks_like_unstructured_description(query: &str) -> bool {
    mcp_query_looks_like_unstructured_description(query)
}

pub(super) fn parse_get_context_target(
    args: Option<&Value>,
) -> std::result::Result<ParsedGetContextTarget, Box<ToolErrorPayload>> {
    if args.is_some_and(|value| {
        value.get("query").is_some() || value.get("file").is_some() || value.get("files").is_some()
    }) {
        return Err(get_context_target_error(
            "legacy get_context target fields are no longer supported",
            "Use target={ kind: 'query' | 'file' | 'files', ... } and remove top-level query/file/files fields.",
            vec!["query", "file", "files"],
            json!({ "target": { "kind": "query", "query": "handle_request" } }),
        ));
    }

    let target_value = args.and_then(|value| value.get("target"));
    let target_object = target_value.and_then(|value| value.as_object());

    if target_value.is_some() && target_object.is_none() {
        return Err(get_context_target_error(
            "invalid target selector",
            "target must be an object with kind=query, kind=file, or kind=files",
            vec!["target"],
            json!({ "target": { "kind": "query", "query": "handle_request" } }),
        ));
    }

    if let Some(target) = target_object {
        let kind = target
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                get_context_target_error(
                    "target.kind is required",
                    "target object requires kind=query, kind=file, or kind=files",
                    vec!["target.kind"],
                    json!({ "target": { "kind": "query", "query": "handle_request" } }),
                )
            })?;
        match kind {
            "query" => {
                let query = target
                    .get("query")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        get_context_target_error(
                            "target.kind='query' requires non-empty target.query",
                            "query target requires target.query",
                            vec!["target.kind", "target.query"],
                            json!({ "target": { "kind": "query", "query": "handle_request" } }),
                        )
                    })?
                    .to_owned();
                if context_query_looks_like_unstructured_description(&query) {
                    return Err(get_context_target_error(
                        "target.query must be exact identifier, qualified name, or supported intent phrase",
                        format!(
                            "query target does not accept natural-language-only descriptions. Supported grammar: {}.",
                            mcp_supported_query_grammar_examples().join(", ")
                        ),
                        vec!["target.query"],
                        json!({ "target": { "kind": "query", "query": "who calls handle_request" } }),
                    ));
                }
                let parsed = parse_mcp_query_grammar(&query);
                Ok(ParsedGetContextTarget {
                    kind: GetContextTargetKind::Query,
                    target: parsed.parsed_request.target.clone(),
                    parsed_request: Some(parsed.parsed_request),
                    query: Some(query),
                    file: None,
                    files: Vec::new(),
                    deprecated_input_fields: Vec::new(),
                })
            }
            "file" => {
                let file = target
                    .get("file")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        get_context_target_error(
                            "target.kind='file' requires non-empty target.file",
                            "file target requires target.file",
                            vec!["target.kind", "target.file"],
                            json!({ "target": { "kind": "file", "file": "src/lib.rs" } }),
                        )
                    })?;
                let file = CanonicalRepoPath::from_repo_relative(file)
                    .map_err(|error| {
                        get_context_target_error(
                            "invalid target.file path",
                            error.to_string(),
                            vec!["target.file"],
                            json!({ "target": { "kind": "file", "file": "src/lib.rs" } }),
                        )
                    })?
                    .as_str()
                    .to_owned();
                Ok(ParsedGetContextTarget {
                    kind: GetContextTargetKind::File,
                    target: ContextTarget::FilePath { path: file.clone() },
                    parsed_request: None,
                    query: None,
                    file: Some(file),
                    files: Vec::new(),
                    deprecated_input_fields: Vec::new(),
                })
            }
            "files" => {
                let files = target
                    .get("files")
                    .and_then(|value| value.as_array())
                    .ok_or_else(|| {
                        get_context_target_error(
                            "target.kind='files' requires non-empty target.files",
                            "files target requires target.files array",
                            vec!["target.kind", "target.files"],
                            json!({ "target": { "kind": "files", "files": ["src/lib.rs"] } }),
                        )
                    })?
                    .iter()
                    .filter_map(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|path| {
                        CanonicalRepoPath::from_repo_relative(path)
                            .map(|path| path.as_str().to_owned())
                            .map_err(|error| {
                                get_context_target_error(
                                    "invalid target.files path",
                                    error.to_string(),
                                    vec!["target.files"],
                                    json!({ "target": { "kind": "files", "files": ["src/lib.rs"] } }),
                                )
                            })
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if files.is_empty() {
                    return Err(get_context_target_error(
                        "target.kind='files' requires non-empty target.files",
                        "files target requires target.files array",
                        vec!["target.kind", "target.files"],
                        json!({ "target": { "kind": "files", "files": ["src/lib.rs"] } }),
                    ));
                }
                Ok(ParsedGetContextTarget {
                    kind: GetContextTargetKind::Files,
                    target: ContextTarget::ChangedFiles {
                        paths: files.clone(),
                    },
                    parsed_request: None,
                    query: None,
                    file: None,
                    files,
                    deprecated_input_fields: Vec::new(),
                })
            }
            other => Err(get_context_target_error(
                format!("invalid target.kind '{other}'"),
                "target.kind must be one of: query, file, files",
                vec!["target.kind"],
                json!({ "target": { "kind": "query", "query": "handle_request" } }),
            )),
        }
    } else {
        Err(get_context_target_error(
            "get_context requires target",
            "Provide target={ kind: 'query' | 'file' | 'files', ... }.",
            vec!["target"],
            json!({ "target": { "kind": "query", "query": "handle_request" } }),
        ))
    }
}

pub(super) fn count_change_kinds(changes: &[ChangedFile]) -> (usize, usize, usize, usize, usize) {
    let mut added = 0;
    let mut modified = 0;
    let mut deleted = 0;
    let mut renamed = 0;
    let mut copied = 0;
    for change in changes {
        match change.change_type {
            ChangeType::Added => added += 1,
            ChangeType::Modified => modified += 1,
            ChangeType::Deleted => deleted += 1,
            ChangeType::Renamed => renamed += 1,
            ChangeType::Copied => copied += 1,
        }
    }
    (added, modified, deleted, renamed, copied)
}
