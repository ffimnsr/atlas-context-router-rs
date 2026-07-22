//! MCP tools for reading file excerpts, around-match snippets, and docs sections.

use anyhow::{Context, Result};
use atlas_core::BudgetManager;
use atlas_review::{DocsSectionSelector, lookup_docs_section};
use atlas_store_sqlite::Store;
use serde::Serialize;
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::{InputShapeErrorSpec, ToolErrorPayload, input_shape_error_payload};

use super::shared::{
    bool_arg, discovery_tool_error_result, inject_budget_metadata, load_budget_policy,
    render_tool_result, repo_path_tool_error_result, resolve_repo_file_path_or_error, str_arg,
    u64_arg, validate_optional_repo_scope_or_error,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RequestedLineRange {
    start_line: usize,
    end_line: usize,
}

#[derive(Clone, Serialize)]
struct ExcerptLine {
    line: u64,
    text: String,
}

#[derive(Clone, Serialize)]
struct FileExcerpt {
    start_line: u64,
    end_line: u64,
    line_count: usize,
    content: String,
    lines: Vec<ExcerptLine>,
}

#[derive(Clone, Debug)]
struct LineSnippetWindow {
    start_line: usize,
    end_line: usize,
    match_lines: Vec<usize>,
}

#[derive(Clone, Serialize)]
struct AroundMatchLine {
    line: u64,
    text: String,
    kind: &'static str,
}

#[derive(Clone, Serialize)]
struct AroundMatchSnippet {
    start_line: u64,
    end_line: u64,
    match_lines: Vec<u64>,
    content: String,
    lines: Vec<AroundMatchLine>,
}

// ---------------------------------------------------------------------------
// Excerpt selection helpers
// ---------------------------------------------------------------------------

fn parse_requested_range(value: &serde_json::Value) -> Result<RequestedLineRange> {
    let start_line = value
        .get("start_line")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("line_ranges entries must include start_line"))?
        as usize;
    let end_line = value
        .get("end_line")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("line_ranges entries must include end_line"))?
        as usize;
    validate_requested_range(start_line, end_line)
}

fn validate_requested_range(start_line: usize, end_line: usize) -> Result<RequestedLineRange> {
    if start_line == 0 || end_line == 0 {
        anyhow::bail!("line numbers are 1-based; got start_line={start_line}, end_line={end_line}");
    }
    if end_line < start_line {
        anyhow::bail!("invalid line range: start_line {start_line} exceeds end_line {end_line}");
    }
    Ok(RequestedLineRange {
        start_line,
        end_line,
    })
}

fn normalize_requested_ranges(mut ranges: Vec<RequestedLineRange>) -> Vec<RequestedLineRange> {
    ranges.sort_unstable_by_key(|range| (range.start_line, range.end_line));

    let mut merged: Vec<RequestedLineRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start_line <= last.end_line.saturating_add(1)
        {
            last.end_line = last.end_line.max(range.end_line);
            continue;
        }
        merged.push(range);
    }

    merged
}

fn extract_line_text(lines: &[&str], line_number: usize) -> String {
    lines[line_number - 1].trim_end_matches('\r').to_owned()
}

fn build_line_windows(matches: &[usize], before: usize, after: usize) -> Vec<LineSnippetWindow> {
    let mut windows: Vec<LineSnippetWindow> = Vec::new();
    for line in matches {
        let start_line = line.saturating_sub(before).max(1);
        let end_line = line.saturating_add(after);
        if let Some(last) = windows.last_mut()
            && start_line <= last.end_line.saturating_add(1)
        {
            last.end_line = last.end_line.max(end_line);
            last.match_lines.push(*line);
            continue;
        }
        windows.push(LineSnippetWindow {
            start_line,
            end_line,
            match_lines: vec![*line],
        });
    }
    windows
}

fn build_around_match_snippets(
    lines: &[&str],
    matches: &[usize],
    before: usize,
    after: usize,
    max_lines: usize,
) -> (Vec<AroundMatchSnippet>, bool, usize) {
    let mut remaining_lines = max_lines;
    let mut snippets = Vec::new();
    let mut truncated = false;
    let windows = build_line_windows(matches, before, after);
    let observed_lines = windows
        .iter()
        .map(|window| window.end_line.min(lines.len()) - window.start_line + 1)
        .sum();

    for mut window in windows {
        if lines.is_empty() || remaining_lines == 0 {
            truncated = true;
            break;
        }
        window.end_line = window.end_line.min(lines.len());
        let available = window.end_line - window.start_line + 1;
        let end_line = if available > remaining_lines {
            truncated = true;
            window.start_line + remaining_lines - 1
        } else {
            window.end_line
        };

        let snippet_lines: Vec<AroundMatchLine> = (window.start_line..=end_line)
            .map(|line_number| AroundMatchLine {
                line: line_number as u64,
                text: extract_line_text(lines, line_number),
                kind: if window.match_lines.contains(&line_number) {
                    "match"
                } else if line_number < *window.match_lines.first().unwrap_or(&line_number) {
                    "before"
                } else {
                    "after"
                },
            })
            .collect();
        let content = snippet_lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let match_lines = window
            .match_lines
            .into_iter()
            .filter(|line| *line <= end_line)
            .map(|line| line as u64)
            .collect();
        remaining_lines = remaining_lines.saturating_sub(snippet_lines.len());
        snippets.push(AroundMatchSnippet {
            start_line: window.start_line as u64,
            end_line: end_line as u64,
            match_lines,
            content,
            lines: snippet_lines,
        });
    }

    (snippets, truncated, observed_lines)
}

// ---------------------------------------------------------------------------
// Input shape error helpers
// ---------------------------------------------------------------------------

fn read_file_excerpt_input_shape_error(
    message: impl Into<String>,
    detail: impl Into<String>,
    offending_fields: Vec<&str>,
    normalization_performed: Vec<String>,
    retry_example: serde_json::Value,
    fail_closed_reason: Option<&str>,
    extra_details: Option<serde_json::Value>,
) -> Box<ToolErrorPayload> {
    Box::new(input_shape_error_payload(
        "read_file_excerpt",
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields: offending_fields.into_iter().map(str::to_owned).collect(),
            normalization_performed,
            accepted_argument_families: vec![
                "line_ranges".to_owned(),
                "start_line/end_line".to_owned(),
                "line with optional before/after".to_owned(),
            ],
            retry_example: Some(retry_example),
            fail_closed_reason: fail_closed_reason.map(str::to_owned),
            retry_guidance: Some(
                "Provide exactly one selector family and keep wrapper-default fields absent or empty, then retry."
                    .to_owned(),
            ),
            extra_details,
        },
    ))
}

fn parse_excerpt_selection(
    args: Option<&serde_json::Value>,
) -> std::result::Result<(Vec<RequestedLineRange>, &'static str), Box<ToolErrorPayload>> {
    let start_line_raw = u64_arg(args, "start_line");
    let end_line_raw = u64_arg(args, "end_line");
    let line_raw = u64_arg(args, "line");
    let before = u64_arg(args, "before").unwrap_or(0) as usize;
    let after = u64_arg(args, "after").unwrap_or(0) as usize;
    let line_ranges_value = args.and_then(|value| value.get("line_ranges"));
    let line_ranges = line_ranges_value
        .and_then(|value| value.as_array())
        .map(|ranges| {
            ranges
                .iter()
                .map(parse_requested_range)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()
        .map_err(|error| {
            read_file_excerpt_input_shape_error(
                "invalid line_ranges selector",
                error.to_string(),
                vec!["line_ranges"],
                Vec::new(),
                serde_json::json!({
                    "file": "src/lib.rs",
                    "line_ranges": [{ "start_line": 10, "end_line": 20 }]
                }),
                None,
                None,
            )
        })?
        .unwrap_or_default();

    let start_line = start_line_raw
        .filter(|value| *value > 0)
        .map(|value| value as usize);
    let end_line = end_line_raw
        .filter(|value| *value > 0)
        .map(|value| value as usize);
    let line = line_raw
        .filter(|value| *value > 0)
        .map(|value| value as usize);

    let line_ranges_used = !line_ranges.is_empty();
    let single_range_used = start_line_raw.is_some_and(|value| value > 0)
        || end_line_raw.is_some_and(|value| value > 0);
    let line_context_used = line_raw.is_some_and(|value| value > 0) || before > 0 || after > 0;

    let selectors_used = usize::from(line_ranges_used)
        + usize::from(single_range_used)
        + usize::from(line_context_used);
    if selectors_used != 1 {
        let seen_families = [
            line_ranges_used.then_some("line_ranges"),
            single_range_used.then_some("start_line/end_line"),
            line_context_used.then_some("line with optional before/after"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        let seen_families = if seen_families.is_empty() {
            "none".to_owned()
        } else {
            seen_families
        };
        let mut normalization_performed = Vec::new();
        if start_line_raw == Some(0) {
            normalization_performed.push("ignored start_line=0 wrapper field".to_owned());
        }
        if end_line_raw == Some(0) {
            normalization_performed.push("ignored end_line=0 wrapper field".to_owned());
        }
        if line_raw == Some(0) {
            normalization_performed.push("ignored line=0 wrapper field".to_owned());
        }
        if line_ranges_value
            .and_then(|value| value.as_array())
            .is_some_and(|ranges| ranges.is_empty())
        {
            normalization_performed.push("ignored empty line_ranges wrapper field".to_owned());
        }
        return Err(read_file_excerpt_input_shape_error(
            "provide exactly one selector: line_ranges, start_line/end_line, or line with optional before/after",
            format!(
                "selector families seen: {seen_families}. Atlas refused to guess between conflicting selector families."
            ),
            vec![
                "line_ranges",
                "start_line",
                "end_line",
                "line",
                "before",
                "after",
            ],
            normalization_performed,
            serde_json::json!({ "file": "src/lib.rs", "start_line": 10, "end_line": 20 }),
            Some("Atlas refused to guess between conflicting selector families"),
            Some(serde_json::json!({ "selector_families_seen": seen_families })),
        ));
    }

    if line_ranges_used {
        return Ok((normalize_requested_ranges(line_ranges), "line_ranges"));
    }

    if line_context_used {
        let line = line.ok_or_else(|| {
            read_file_excerpt_input_shape_error(
                "line must be >= 1 when using line-context selector",
                "line-context selector requires line >= 1",
                vec!["line", "before", "after"],
                Vec::new(),
                serde_json::json!({ "file": "src/lib.rs", "line": 42, "before": 2, "after": 2 }),
                None,
                None,
            )
        })?;
        let start_line = line.saturating_sub(before).max(1);
        let end_line = line.saturating_add(after);
        let range = validate_requested_range(start_line, end_line).map_err(|error| {
            read_file_excerpt_input_shape_error(
                "invalid line-context selector",
                error.to_string(),
                vec!["line", "before", "after"],
                Vec::new(),
                serde_json::json!({ "file": "src/lib.rs", "line": 42, "before": 2, "after": 2 }),
                None,
                None,
            )
        })?;
        return Ok((vec![range], "line_context"));
    }

    if start_line_raw.is_some_and(|value| value == 0)
        || end_line_raw.is_some_and(|value| value == 0)
    {
        return Err(read_file_excerpt_input_shape_error(
            "start_line/end_line must be >= 1 when using single-range selector",
            "single-range selector requires start_line >= 1 and end_line >= 1",
            vec!["start_line", "end_line"],
            Vec::new(),
            serde_json::json!({ "file": "src/lib.rs", "start_line": 10, "end_line": 20 }),
            None,
            None,
        ));
    }

    let start_line = start_line.ok_or_else(|| {
        read_file_excerpt_input_shape_error(
            "missing required argument: start_line",
            "single-range selector requires both start_line and end_line",
            vec!["start_line", "end_line"],
            Vec::new(),
            serde_json::json!({ "file": "src/lib.rs", "start_line": 10, "end_line": 20 }),
            None,
            None,
        )
    })?;
    let end_line = end_line.ok_or_else(|| {
        read_file_excerpt_input_shape_error(
            "missing required argument: end_line",
            "single-range selector requires both start_line and end_line",
            vec!["start_line", "end_line"],
            Vec::new(),
            serde_json::json!({ "file": "src/lib.rs", "start_line": 10, "end_line": 20 }),
            None,
            None,
        )
    })?;
    let range = validate_requested_range(start_line, end_line).map_err(|error| {
        read_file_excerpt_input_shape_error(
            "invalid single-range selector",
            error.to_string(),
            vec!["start_line", "end_line"],
            Vec::new(),
            serde_json::json!({ "file": "src/lib.rs", "start_line": 10, "end_line": 20 }),
            None,
            None,
        )
    })?;
    Ok((vec![range], "single_range"))
}

// ---------------------------------------------------------------------------
// tool_read_file_excerpt
// ---------------------------------------------------------------------------

/// MCP tool: `read_file_excerpt` — bounded file reads by line range or line-with-context.
pub(crate) fn tool_read_file_excerpt(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    (|| {
        if let Err(payload) = validate_optional_repo_scope_or_error("read_file_excerpt", args, repo_root) {
            return repo_path_tool_error_result(output_format, *payload);
        }
        let policy = load_budget_policy(repo_root)?;
        let mut budgets = BudgetManager::new();
        let file = str_arg(args, "file")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?;
        let requested_max_lines = u64_arg(args, "max_lines").unwrap_or(200) as usize;
        let max_lines = budgets.resolve_limit(
            policy.review_context_extraction.nodes,
            "review_context_extraction.max_nodes",
            Some(requested_max_lines),
        );
        let (file, abs_path) = match resolve_repo_file_path_or_error(
            "read_file_excerpt",
            repo_root,
            file,
            true,
        ) {
            Ok(resolved) => resolved,
            Err(payload) => return repo_path_tool_error_result(output_format, *payload),
        };

        let (requested_ranges, mode) = match parse_excerpt_selection(args) {
            Ok(selection) => selection,
            Err(payload) => return repo_path_tool_error_result(output_format, *payload),
        };
        let contents = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("cannot read UTF-8 text file '{file}'"))?;
        let lines: Vec<&str> = contents.lines().collect();
        let total_lines = lines.len();

        let requested_range_count = requested_ranges.len();
        let mut resolved_ranges = Vec::with_capacity(requested_range_count);
        for range in requested_ranges {
            if total_lines == 0 {
                break;
            }
            if range.start_line > total_lines {
                anyhow::bail!(
                    "requested start_line {} exceeds file length {} for {}",
                    range.start_line,
                    total_lines,
                    file
                );
            }
            resolved_ranges.push(RequestedLineRange {
                start_line: range.start_line,
                end_line: range.end_line.min(total_lines),
            });
        }

        let total_selected_lines: usize = resolved_ranges
            .iter()
            .map(|range| range.end_line - range.start_line + 1)
            .sum();
        #[derive(Serialize)]
        struct ExcerptRange {
            start_line: u64,
            end_line: u64,
        }

        #[derive(Serialize)]
        struct ReadFileExcerptSummary {
            total_lines: usize,
            requested_range_count: usize,
            returned_snippet_count: usize,
            total_selected_lines: usize,
            has_matches: bool,
        }

        #[derive(Serialize)]
        struct ReadFileExcerptResult {
            tool: &'static str,
            file: String,
            selection_mode: &'static str,
            ranges: Vec<ExcerptRange>,
            snippets: Vec<FileExcerpt>,
            summary: ReadFileExcerptSummary,
            truncated: bool,
            warnings: Vec<String>,
        }

        let normalized_ranges = resolved_ranges
            .iter()
            .map(|range| ExcerptRange {
                start_line: range.start_line as u64,
                end_line: range.end_line as u64,
            })
            .collect::<Vec<_>>();

        let mut remaining_lines = max_lines;
        let mut snippets = Vec::with_capacity(resolved_ranges.len());
        let mut truncated = false;

        for range in resolved_ranges {
            if remaining_lines == 0 {
                truncated = true;
                break;
            }

            let available_lines = range.end_line - range.start_line + 1;
            let excerpt_end = if available_lines > remaining_lines {
                truncated = true;
                range.start_line + remaining_lines - 1
            } else {
                range.end_line
            };

            let excerpt_lines: Vec<ExcerptLine> = (range.start_line..=excerpt_end)
                .map(|line_number| ExcerptLine {
                    line: line_number as u64,
                    text: lines[line_number - 1].trim_end_matches('\r').to_owned(),
                })
                .collect();
            let content = excerpt_lines
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            remaining_lines = remaining_lines.saturating_sub(excerpt_lines.len());
            snippets.push(FileExcerpt {
                start_line: range.start_line as u64,
                end_line: excerpt_end as u64,
                line_count: excerpt_lines.len(),
                content,
                lines: excerpt_lines,
            });
        }

        if truncated {
            budgets.record_usage(
                policy.review_context_extraction.nodes,
                "review_context_extraction.max_nodes",
                max_lines,
                total_selected_lines,
                true,
            );
        }

        let warnings = if total_lines == 0 {
            vec![format!("{file} is empty.")]
        } else if truncated {
            vec![format!(
                "Excerpt truncated to {max_lines} lines. Narrow selection or raise max_lines within policy limits."
            )]
        } else {
            Vec::new()
        };
        let result = ReadFileExcerptResult {
            tool: "read_file_excerpt",
            file,
            selection_mode: mode,
            ranges: normalized_ranges,
            summary: ReadFileExcerptSummary {
                total_lines,
                requested_range_count,
                returned_snippet_count: snippets.len(),
                total_selected_lines,
                has_matches: !snippets.is_empty(),
            },
            snippets,
            truncated,
            warnings,
        };

        let mut response = render_tool_result(&result, output_format)?;
        inject_budget_metadata(
            &mut response,
            &budgets.summary(
                "review_context_extraction.max_nodes",
                max_lines,
                requested_max_lines.max(total_selected_lines),
            ),
        );
        Ok(response)
    })()
    .or_else(|error| discovery_tool_error_result("read_file_excerpt", output_format, error))
}

// ---------------------------------------------------------------------------
// tool_get_docs_section
// ---------------------------------------------------------------------------

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
        let heading = str_arg(args, "heading")?.map(str::to_owned);
        let line = u64_arg(args, "line").map(|value| value as u32);
        if usize::from(heading.is_some()) + usize::from(line.is_some()) != 1 {
            anyhow::bail!("provide exactly one selector: heading or line");
        }

        let max_bytes = u64_arg(args, "max_bytes").unwrap_or(16_384) as usize;
        let (file, _) =
            match resolve_repo_file_path_or_error("get_docs_section", repo_root, file, false) {
                Ok(resolved) => resolved,
                Err(payload) => return repo_path_tool_error_result(output_format, *payload),
            };
        let store = Store::open(db_path)
            .with_context(|| format!("cannot open atlas store at '{db_path}'"))?;
        let selector = if let Some(selector) = heading {
            DocsSectionSelector::Heading(selector)
        } else {
            DocsSectionSelector::Line(line.expect("validated line selector"))
        };
        let result = lookup_docs_section(
            &store,
            camino::Utf8Path::new(repo_root),
            &file,
            selector,
            max_bytes,
        )?;

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
        let warnings = if !resolved {
            vec!["Heading selector resolved ambiguously. Inspect candidates and retry with exact heading path.".to_owned()]
        } else if result.truncated {
            vec![format!("Section content truncated; omitted {} bytes.", result.omitted_byte_count)]
        } else {
            Vec::new()
        };
        let normalized = GetDocsSectionResult {
            tool: "get_docs_section",
            file: result.file.clone(),
            selector_mode: result.selector_kind.clone(),
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
        Ok(response)
    })()
    .or_else(|error| discovery_tool_error_result("get_docs_section", output_format, error))
}

// ---------------------------------------------------------------------------
// tool_read_file_around_match
// ---------------------------------------------------------------------------

/// MCP tool: `read_file_around_match` — read merged snippets around matches in one file.
pub(crate) fn tool_read_file_around_match(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    (|| {
        if let Err(payload) =
            validate_optional_repo_scope_or_error("read_file_around_match", args, repo_root)
        {
            return repo_path_tool_error_result(output_format, *payload);
        }
        let policy = load_budget_policy(repo_root)?;
        let mut budgets = BudgetManager::new();
        let file = str_arg(args, "file")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?;
        let query = str_arg(args, "query")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: query"))?;
        let is_regex = bool_arg(args, "is_regex").unwrap_or(false);
        let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(is_regex);
        let before = u64_arg(args, "before").unwrap_or(2) as usize;
        let after = u64_arg(args, "after").unwrap_or(2) as usize;
        let requested_max_matches = u64_arg(args, "max_matches").unwrap_or(20) as usize;
        let max_matches = budgets.resolve_limit(
            policy.review_context_extraction.nodes,
            "review_context_extraction.max_nodes",
            Some(requested_max_matches),
        );
        let requested_max_lines = u64_arg(args, "max_lines").unwrap_or(200) as usize;
        let max_lines = budgets.resolve_limit(
            policy.review_context_extraction.nodes,
            "review_context_extraction.max_nodes",
            Some(requested_max_lines),
        );
        let (file, abs_path) = match resolve_repo_file_path_or_error(
            "read_file_around_match",
            repo_root,
            file,
            true,
        ) {
            Ok(resolved) => resolved,
            Err(payload) => return repo_path_tool_error_result(output_format, *payload),
        };

        let contents = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("cannot read UTF-8 text file '{file}'"))?;
        let lines: Vec<&str> = contents.lines().collect();
        let pattern = if is_regex {
            query.to_owned()
        } else {
            regex::escape(query)
        };
        let matcher = regex::RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|error| super::shared::invalid_search_content_regex_error(query, error))?;

        let mut match_lines = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| matcher.is_match(line).then_some(index + 1))
            .collect::<Vec<_>>();
        let total_matches = match_lines.len();
        let match_limit_hit = match_lines.len() > max_matches;
        if match_limit_hit {
            match_lines.truncate(max_matches);
            budgets.record_usage(
                policy.review_context_extraction.nodes,
                "review_context_extraction.max_nodes",
                max_matches,
                total_matches,
                true,
            );
        }

        let (snippets, line_limit_hit, observed_lines) =
            build_around_match_snippets(&lines, &match_lines, before, after, max_lines);
        if line_limit_hit {
            budgets.record_usage(
                policy.review_context_extraction.nodes,
                "review_context_extraction.max_nodes",
                max_lines,
                observed_lines,
                true,
            );
        }

        let truncated = match_limit_hit || line_limit_hit;
        let warnings = if total_matches == 0 {
            vec![format!("No matches for '{query}' in {file}.")]
        } else if truncated {
            vec!["Snippets truncated by max_matches or max_lines budget.".to_owned()]
        } else {
            Vec::new()
        };

        #[derive(Serialize)]
        struct ReadFileAroundMatchSummary {
            total_matches: usize,
            returned_matches: usize,
            snippet_count: usize,
            observed_lines: usize,
        }

        #[derive(Serialize)]
        struct AroundMatchResult {
            tool: &'static str,
            file: String,
            match_mode: &'static str,
            query: String,
            before: usize,
            after: usize,
            matches: Vec<AroundMatchSnippet>,
            summary: ReadFileAroundMatchSummary,
            truncated: bool,
            warnings: Vec<String>,
        }

        let returned_matches = snippets
            .iter()
            .map(|snippet| snippet.match_lines.len())
            .sum();
        let result = AroundMatchResult {
            tool: "read_file_around_match",
            file,
            match_mode: if is_regex { "regex" } else { "literal" },
            query: query.to_owned(),
            before,
            after,
            summary: ReadFileAroundMatchSummary {
                total_matches,
                returned_matches,
                snippet_count: snippets.len(),
                observed_lines,
            },
            matches: snippets,
            truncated,
            warnings,
        };

        let mut response = render_tool_result(&result, output_format)?;
        inject_budget_metadata(
            &mut response,
            &budgets.summary(
                "review_context_extraction.max_nodes",
                max_matches.min(max_lines),
                requested_max_matches
                    .max(requested_max_lines)
                    .max(total_matches)
                    .max(observed_lines),
            ),
        );
        Ok(response)
    })()
    .or_else(|error| discovery_tool_error_result("read_file_around_match", output_format, error))
}
