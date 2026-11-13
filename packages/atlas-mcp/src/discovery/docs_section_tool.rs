//! MCP tool: `get_docs_section` — resolve a Markdown section by heading path/slug or line.

use anyhow::{Context, Result};
use atlas_review::{DocsSectionSelector, lookup_docs_section};
use atlas_store_sqlite::Store;
use serde::Serialize;
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::{InputShapeErrorSpec, ToolErrorPayload, input_shape_error_payload};

use super::shared::{
    discovery_tool_error_result, render_tool_result, repo_path_tool_error_result,
    resolve_repo_file_path_or_error, str_arg, u64_arg, validate_optional_repo_scope_or_error,
};

// ---------------------------------------------------------------------------
// tool_get_docs_section
// ---------------------------------------------------------------------------

struct ParsedDocsSectionSelector {
    selector: DocsSectionSelector,
    selector_mode: &'static str,
}

fn get_docs_section_selector_error(
    message: impl Into<String>,
    detail: impl Into<String>,
    offending_fields: Vec<&str>,
    retry_example: serde_json::Value,
) -> Box<ToolErrorPayload> {
    Box::new(input_shape_error_payload(
        "get_docs_section",
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields: offending_fields.into_iter().map(str::to_owned).collect(),
            normalization_performed: Vec::new(),
            accepted_argument_families: vec![
                "selector.kind=heading".to_owned(),
                "selector.kind=line".to_owned(),
            ],
            retry_example: Some(retry_example),
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting docs section selectors".to_owned(),
            ),
            retry_guidance: Some("Provide exactly one docs section selector and retry.".to_owned()),
            extra_details: Some(serde_json::json!({
                "accepted_selector_shapes": [
                    { "selector": { "kind": "heading", "heading": "document.install" } },
                    { "selector": { "kind": "line", "line": 42 } }
                ]
            })),
        },
    ))
}

fn parse_docs_section_selector(
    args: Option<&serde_json::Value>,
) -> std::result::Result<ParsedDocsSectionSelector, Box<ToolErrorPayload>> {
    if args.is_some_and(|value| value.get("heading").is_some() || value.get("line").is_some()) {
        return Err(get_docs_section_selector_error(
            "legacy docs section selector fields are no longer supported",
            "Use selector={ kind: 'heading' | 'line', ... } and remove top-level heading/line fields.",
            vec!["heading", "line"],
            serde_json::json!({ "selector": { "kind": "heading", "heading": "document.install" } }),
        ));
    }
    let selector_value = args.and_then(|value| value.get("selector"));
    let selector_object = selector_value.and_then(|value| value.as_object());

    if selector_value.is_some() && selector_object.is_none() {
        return Err(get_docs_section_selector_error(
            "invalid selector object",
            "selector must be an object with kind=heading or kind=line",
            vec!["selector"],
            serde_json::json!({ "selector": { "kind": "heading", "heading": "document.install" } }),
        ));
    }
    if let Some(selector) = selector_object {
        let kind = selector
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                get_docs_section_selector_error(
                    "selector.kind is required",
                    "selector object requires kind=heading or kind=line",
                    vec!["selector.kind"],
                    serde_json::json!({ "selector": { "kind": "heading", "heading": "document.install" } }),
                )
            })?;
        return match kind {
            "heading" => {
                let heading = selector
                    .get("heading")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        get_docs_section_selector_error(
                            "selector.kind='heading' requires selector.heading",
                            "heading selector requires non-empty selector.heading",
                            vec!["selector.kind", "selector.heading"],
                            serde_json::json!({ "selector": { "kind": "heading", "heading": "document.install" } }),
                        )
                    })?
                    .to_owned();
                Ok(ParsedDocsSectionSelector {
                    selector: DocsSectionSelector::Heading(heading),
                    selector_mode: "heading",
                })
            }
            "line" => {
                let line = selector
                    .get("line")
                    .and_then(|value| value.as_u64())
                    .ok_or_else(|| {
                        get_docs_section_selector_error(
                            "selector.kind='line' requires selector.line",
                            "line selector requires line >= 1",
                            vec!["selector.kind", "selector.line"],
                            serde_json::json!({ "selector": { "kind": "line", "line": 42 } }),
                        )
                    })?;
                if line == 0 {
                    return Err(get_docs_section_selector_error(
                        "selector.kind='line' requires selector.line",
                        "line selector requires line >= 1",
                        vec!["selector.kind", "selector.line"],
                        serde_json::json!({ "selector": { "kind": "line", "line": 42 } }),
                    ));
                }
                Ok(ParsedDocsSectionSelector {
                    selector: DocsSectionSelector::Line(line as u32),
                    selector_mode: "line",
                })
            }
            other => Err(get_docs_section_selector_error(
                format!("invalid selector.kind '{other}'"),
                "selector.kind must be one of: heading, line",
                vec!["selector.kind"],
                serde_json::json!({ "selector": { "kind": "heading", "heading": "document.install" } }),
            )),
        };
    }
    Err(get_docs_section_selector_error(
        "selector is required",
        "Provide selector={ kind: 'heading' | 'line', ... }.",
        vec!["selector"],
        serde_json::json!({ "selector": { "kind": "heading", "heading": "document.install" } }),
    ))
}

/// MCP tool: `get_docs_section` — resolve a Markdown section by heading path/slug or line.
pub(crate) fn tool_get_docs_section(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    (|| {
        if let Err(payload) =
            validate_optional_repo_scope_or_error("get_docs_section", args, repo_root)
        {
            return repo_path_tool_error_result(output_format, *payload);
        }
        let file = str_arg(args, "file")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?;
        let selector = match parse_docs_section_selector(args) {
            Ok(selector) => selector,
            Err(payload) => return repo_path_tool_error_result(output_format, *payload),
        };

        let max_bytes = u64_arg(args, "max_bytes").unwrap_or(16_384) as usize;
        let (file, _) =
            match resolve_repo_file_path_or_error("get_docs_section", repo_root, file, false) {
                Ok(resolved) => resolved,
                Err(payload) => return repo_path_tool_error_result(output_format, *payload),
            };
        let store = Store::open(db_path)
            .with_context(|| format!("cannot open atlas store at '{db_path}'"))?;
        let heading_query = match &selector.selector {
            DocsSectionSelector::Heading(query) => Some(query.clone()),
            DocsSectionSelector::Line(_) => None,
        };
        let mut missed_heading = false;
        let result = match lookup_docs_section(
            &store,
            camino::Utf8Path::new(repo_root),
            &file,
            selector.selector,
            max_bytes,
        ) {
            Ok(result) => result,
            Err(error) => {
                let message = error.to_string();
                let Some(query) = heading_query.as_deref() else {
                    return Err(error.into());
                };
                if !message.starts_with("heading not found in ") {
                    return Err(error.into());
                }
                // A miss is not an operational failure: return the near-match
                // candidates so the caller can retry with an exact heading.
                missed_heading = true;
                super::docs_miss::unresolved_heading_lookup(
                    &store, repo_root, &file, query,
                )
            }
        };

        #[derive(Serialize)]
        struct DocsHeading {
            title: String,
            path: String,
            level: u64,
        }

        #[derive(Serialize)]
        struct GetDocsSectionSummary {
            resolved: bool,
            candidate_count: usize,
            line_count: usize,
        }

        #[derive(Serialize)]
        struct GetDocsSectionResult {
            tool: &'static str,
            file: String,
            selector_mode: String,
            heading: Option<DocsHeading>,
            slug: Option<String>,
            line_start: Option<u64>,
            line_end: Option<u64>,
            content: Option<String>,
            file_hash: Option<String>,
            resolved: bool,
            query: Option<String>,
            candidates: Value,
            lines: Value,
            summary: GetDocsSectionSummary,
            truncated: bool,
            warnings: Vec<String>,
        }

        let resolved = result.resolved;
        let heading = match (
            result.title.clone(),
            result.heading_path.clone(),
            result.heading_level,
        ) {
            (Some(title), Some(path), Some(level)) => Some(DocsHeading {
                title,
                path,
                level: u64::from(level),
            }),
            _ => None,
        };
        let warnings = if missed_heading {
            vec![
                "No heading matched the selector. Inspect candidates and retry with an exact heading path.".to_owned(),
            ]
        } else if !resolved {
            vec!["Heading selector resolved ambiguously. Inspect candidates and retry with exact heading path.".to_owned()]
        } else if result.truncated {
            vec![format!("Section content truncated; omitted {} bytes.", result.omitted_byte_count)]
        } else {
            Vec::new()
        };
        let normalized = GetDocsSectionResult {
            tool: "get_docs_section",
            file: result.file.clone(),
            selector_mode: selector.selector_mode.to_owned(),
            heading,
            slug: result.heading_slug.clone(),
            line_start: result.start_line.map(u64::from),
            line_end: result.end_line.map(u64::from),
            content: result.content.clone(),
            file_hash: result.file_hash.clone(),
            resolved,
            query: result.query.clone(),
            candidates: serde_json::to_value(&result.candidates)?,
            lines: serde_json::to_value(&result.lines)?,
            summary: GetDocsSectionSummary {
                resolved,
                candidate_count: result.candidates.len(),
                line_count: result.line_count.unwrap_or_default() as usize,
            },
            truncated: result.truncated,
            warnings,
        };

        let mut response = render_tool_result(&normalized, output_format)?;
        response["file"] = serde_json::Value::String(normalized.file.clone());
        // Flat aliases keep the CLI contract stable while the canonical
        // nested `heading` object remains the MCP shape.
        let structured = response
            .get_mut("structuredContent")
            .expect("docs section payload must be an object");
        structured["selector_kind"] =
            serde_json::Value::String(normalized.selector_mode.clone());
        structured["heading_path"] = normalized
            .heading
            .as_ref()
            .map(|heading| serde_json::Value::String(heading.path.clone()))
            .unwrap_or(serde_json::Value::Null);
        structured["heading_slug"] = normalized
            .slug
            .clone()
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null);
        structured["start_line"] = normalized
            .line_start
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null);
        structured["end_line"] = normalized
            .line_end
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null);
        structured["omitted_byte_count"] = serde_json::Value::from(result.omitted_byte_count);
        structured["heading_level"] = normalized
            .heading
            .as_ref()
            .map(|heading| serde_json::Value::from(heading.level))
            .unwrap_or(serde_json::Value::Null);
        structured["line_count"] = serde_json::Value::from(normalized.summary.line_count);
        structured["atlas_result_kind"] = serde_json::Value::String("docs_section".to_owned());

        Ok(response)
    })()
    .or_else(|error| discovery_tool_error_result("get_docs_section", output_format, error))
}
