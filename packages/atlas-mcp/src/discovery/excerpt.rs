//! MCP tools for reading file excerpts, around-match snippets, and docs sections.

use anyhow::{Context, Result};
use atlas_core::BudgetManager;
use serde::Serialize;

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
struct FileExcerpt {
    range: [u64; 2],
    lines: Vec<LineTuple>,
}

#[derive(Clone, Debug)]
struct LineSnippetWindow {
    start_line: usize,
    end_line: usize,
    match_lines: Vec<usize>,
}

type LineTuple = (u64, String);

#[derive(Clone, Serialize)]
struct AroundMatchSnippet {
    range: [u64; 2],
    before: Vec<LineTuple>,
    #[serde(rename = "match")]
    matches: Vec<LineTuple>,
    after: Vec<LineTuple>,
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

/// Parse the documented `ranges: [[start, end], ...]` alias, or an object with
/// `start_line`/`end_line`.
fn parse_requested_range_alias(value: &serde_json::Value) -> Result<RequestedLineRange> {
    if let Some(pair) = value.as_array() {
        if pair.len() != 2 {
            anyhow::bail!("selector.ranges entries must be [start_line, end_line] pairs");
        }
        let start_line = pair[0]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("range start must be a positive integer"))?
            as usize;
        let end_line = pair[1]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("range end must be a positive integer"))?
            as usize;
        return validate_requested_range(start_line, end_line);
    }
    parse_requested_range(value)
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

        let first_match_line = *window.match_lines.first().unwrap_or(&window.start_line);
        let visible_match_lines = window
            .match_lines
            .into_iter()
            .filter(|line| *line <= end_line)
            .collect::<Vec<_>>();
        let mut before_lines = Vec::new();
        let mut match_lines = Vec::new();
        let mut after_lines = Vec::new();
        for line_number in window.start_line..=end_line {
            let line_tuple = (line_number as u64, extract_line_text(lines, line_number));
            if visible_match_lines.contains(&line_number) {
                match_lines.push(line_tuple);
            } else if line_number < first_match_line {
                before_lines.push(line_tuple);
            } else {
                after_lines.push(line_tuple);
            }
        }
        remaining_lines = remaining_lines.saturating_sub(end_line - window.start_line + 1);
        snippets.push(AroundMatchSnippet {
            range: [window.start_line as u64, end_line as u64],
            before: before_lines,
            matches: match_lines,
            after: after_lines,
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
                "selector.kind=range".to_owned(),
                "selector.kind=ranges".to_owned(),
                "selector.kind=context".to_owned(),
            ],
            retry_example: Some(retry_example),
            fail_closed_reason: fail_closed_reason.map(str::to_owned),
            retry_guidance: Some(
                "Provide selector={ kind: 'range' | 'ranges' | 'context', ... } and retry."
                    .to_owned(),
            ),
            extra_details,
        },
    ))
}

struct ParsedExcerptSelection {
    ranges: Vec<RequestedLineRange>,
    selection_mode: &'static str,
}

fn parse_excerpt_selection(
    args: Option<&serde_json::Value>,
) -> std::result::Result<ParsedExcerptSelection, Box<ToolErrorPayload>> {
    let selector_value = args.and_then(|value| value.get("selector"));
    let selector_object = selector_value.and_then(|value| value.as_object());
    let legacy_fields_present = args.is_some_and(|value| {
        value.get("line_ranges").is_some()
            || value.get("start_line").is_some()
            || value.get("end_line").is_some()
            || value.get("line").is_some()
            || value.get("before").is_some()
            || value.get("after").is_some()
    });

    if legacy_fields_present {
        return Err(read_file_excerpt_input_shape_error(
            "legacy excerpt selector fields are no longer supported",
            "Use selector={ kind: 'range' | 'ranges' | 'context', ... } and remove top-level line_ranges/start_line/end_line/line/before/after fields.",
            vec![
                "line_ranges",
                "start_line",
                "end_line",
                "line",
                "before",
                "after",
            ],
            Vec::new(),
            serde_json::json!({
                "file": "src/lib.rs",
                "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
            }),
            None,
            None,
        ));
    }

    if selector_value.is_some() && selector_object.is_none() {
        return Err(read_file_excerpt_input_shape_error(
            "invalid selector object",
            "selector must be an object with kind=range, kind=ranges, or kind=context",
            vec!["selector"],
            Vec::new(),
            serde_json::json!({
                "file": "src/lib.rs",
                "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
            }),
            None,
            None,
        ));
    }

    if let Some(selector) = selector_object {
        let kind = selector
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                read_file_excerpt_input_shape_error(
                    "selector.kind is required",
                    "selector object requires kind=range, kind=ranges, or kind=context",
                    vec!["selector.kind"],
                    Vec::new(),
                    serde_json::json!({
                        "file": "src/lib.rs",
                        "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
                    }),
                    None,
                    None,
                )
            })?;
        match kind {
            "range" => {
                let start_line = selector
                    .get("start_line")
                    .and_then(|value| value.as_u64())
                    .ok_or_else(|| {
                        read_file_excerpt_input_shape_error(
                            "selector.kind='range' requires start_line and end_line",
                            "range selector requires start_line >= 1 and end_line >= 1",
                            vec!["selector.kind", "selector.start_line", "selector.end_line"],
                            Vec::new(),
                            serde_json::json!({
                                "file": "src/lib.rs",
                                "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
                            }),
                            None,
                            None,
                        )
                    })? as usize;
                let end_line = selector
                    .get("end_line")
                    .and_then(|value| value.as_u64())
                    .ok_or_else(|| {
                        read_file_excerpt_input_shape_error(
                            "selector.kind='range' requires start_line and end_line",
                            "range selector requires start_line >= 1 and end_line >= 1",
                            vec!["selector.kind", "selector.start_line", "selector.end_line"],
                            Vec::new(),
                            serde_json::json!({
                                "file": "src/lib.rs",
                                "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
                            }),
                            None,
                            None,
                        )
                    })? as usize;
                let range = validate_requested_range(start_line, end_line).map_err(|error| {
                    read_file_excerpt_input_shape_error(
                        "invalid range selector",
                        error.to_string(),
                        vec!["selector.start_line", "selector.end_line"],
                        Vec::new(),
                        serde_json::json!({
                            "file": "src/lib.rs",
                            "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
                        }),
                        None,
                        None,
                    )
                })?;
                Ok(ParsedExcerptSelection {
                    ranges: vec![range],
                    selection_mode: "range",
                })
            }
            "ranges" => {
                // Canonical form is `line_ranges: [{start_line, end_line}]`; the
                // `ranges: [[start, end]]` alias is accepted for documented use.
                let raw_ranges = selector
                    .get("line_ranges")
                    .and_then(|value| value.as_array())
                    .or_else(|| selector.get("ranges").and_then(|value| value.as_array()))
                    .ok_or_else(|| {
                        read_file_excerpt_input_shape_error(
                            "selector.kind='ranges' requires selector.line_ranges",
                            "ranges selector requires a non-empty selector.line_ranges or selector.ranges array",
                            vec!["selector.kind", "selector.line_ranges"],
                            Vec::new(),
                            serde_json::json!({
                                "file": "src/lib.rs",
                                "selector": { "kind": "ranges", "line_ranges": [{ "start_line": 10, "end_line": 20 }] }
                            }),
                            None,
                            None,
                        )
                    })?;
                let using_alias = selector.get("line_ranges").is_none();
                let ranges = raw_ranges
                    .iter()
                    .map(|value| {
                        if using_alias {
                            parse_requested_range_alias(value)
                        } else {
                            parse_requested_range(value)
                        }
                    })
                    .collect::<Result<Vec<_>>>()
                    .map(normalize_requested_ranges)
                    .map_err(|error| {
                        read_file_excerpt_input_shape_error(
                            "invalid ranges selector",
                            error.to_string(),
                            vec!["selector.line_ranges"],
                            Vec::new(),
                            serde_json::json!({
                                "file": "src/lib.rs",
                                "selector": { "kind": "ranges", "line_ranges": [{ "start_line": 10, "end_line": 20 }] }
                            }),
                            None,
                            None,
                        )
                    })?;
                if ranges.is_empty() {
                    return Err(read_file_excerpt_input_shape_error(
                        "selector.kind='ranges' requires selector.line_ranges",
                        "ranges selector requires non-empty selector.line_ranges array",
                        vec!["selector.kind", "selector.line_ranges"],
                        Vec::new(),
                        serde_json::json!({
                            "file": "src/lib.rs",
                            "selector": { "kind": "ranges", "line_ranges": [{ "start_line": 10, "end_line": 20 }] }
                        }),
                        None,
                        None,
                    ));
                }
                Ok(ParsedExcerptSelection {
                    ranges,
                    selection_mode: "ranges",
                })
            }
            "context" => {
                let line = selector
                    .get("line")
                    .and_then(|value| value.as_u64())
                    .filter(|line| *line >= 1)
                    .ok_or_else(|| {
                        read_file_excerpt_input_shape_error(
                            "selector.kind='context' requires selector.line",
                            "context selector requires line >= 1",
                            vec!["selector.kind", "selector.line"],
                            Vec::new(),
                            serde_json::json!({
                                "file": "src/lib.rs",
                                "selector": { "kind": "context", "line": 42, "before": 2, "after": 2 }
                            }),
                            None,
                            None,
                        )
                    })? as usize;
                let before = selector
                    .get("before")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
                let after = selector
                    .get("after")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0) as usize;
                let range = validate_requested_range(
                    line.saturating_sub(before).max(1),
                    line.saturating_add(after),
                )
                .map_err(|error| {
                    read_file_excerpt_input_shape_error(
                        "invalid context selector",
                        error.to_string(),
                        vec!["selector.line", "selector.before", "selector.after"],
                        Vec::new(),
                        serde_json::json!({
                            "file": "src/lib.rs",
                            "selector": { "kind": "context", "line": 42, "before": 2, "after": 2 }
                        }),
                        None,
                        None,
                    )
                })?;
                Ok(ParsedExcerptSelection {
                    ranges: vec![range],
                    selection_mode: "context",
                })
            }
            other => Err(read_file_excerpt_input_shape_error(
                format!("invalid selector.kind '{other}'"),
                "selector.kind must be one of: range, ranges, context",
                vec!["selector.kind"],
                Vec::new(),
                serde_json::json!({
                    "file": "src/lib.rs",
                    "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
                }),
                None,
                None,
            )),
        }
    } else {
        Err(read_file_excerpt_input_shape_error(
            "selector is required",
            "Provide selector={ kind: 'range' | 'ranges' | 'context', ... }.",
            vec!["selector"],
            Vec::new(),
            serde_json::json!({
                "file": "src/lib.rs",
                "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
            }),
            None,
            None,
        ))
    }
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

        let selection = match parse_excerpt_selection(args) {
            Ok(selection) => selection,
            Err(payload) => return repo_path_tool_error_result(output_format, *payload),
        };
        let contents = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("cannot read UTF-8 text file '{file}'"))?;
        let lines: Vec<&str> = contents.lines().collect();
        let total_lines = lines.len();

        let requested_range_count = selection.ranges.len();
        let mut resolved_ranges = Vec::with_capacity(requested_range_count);
        for range in selection.ranges {
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
        struct ReadFileExcerptSummary {
            snippets: usize,
            ranges: ReadFileExcerptRangeSummary,
            lines: ReadFileExcerptLineSummary,
            has_matches: bool,
        }

        #[derive(Serialize)]
        struct ReadFileExcerptRangeSummary {
            requested: usize,
            emitted: usize,
        }

        #[derive(Serialize)]
        struct ReadFileExcerptLineSummary {
            total: usize,
            /// Lines covered by the requested selection before truncation.
            selected: usize,
            /// Lines actually emitted in `snippets`.
            emitted: usize,
        }

        #[derive(Serialize)]
        struct ReadFileExcerptResult {
            schema_version: u8,
            tool: &'static str,
            file: String,
            selection_mode: &'static str,
            snippets: Vec<FileExcerpt>,
            summary: ReadFileExcerptSummary,
            truncated: bool,
            warnings: Vec<String>,
        }

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

            let excerpt_lines: Vec<LineTuple> = (range.start_line..=excerpt_end)
                .map(|line_number| {
                    (
                        line_number as u64,
                        lines[line_number - 1].trim_end_matches('\r').to_owned(),
                    )
                })
                .collect();
            remaining_lines = remaining_lines.saturating_sub(excerpt_lines.len());
            snippets.push(FileExcerpt {
                range: [range.start_line as u64, excerpt_end as u64],
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
            schema_version: 2,
            tool: "read_file_excerpt",
            file,
            selection_mode: selection.selection_mode,
            summary: ReadFileExcerptSummary {
                snippets: snippets.len(),
                ranges: ReadFileExcerptRangeSummary {
                    requested: requested_range_count,
                    emitted: snippets.len(),
                },
                lines: ReadFileExcerptLineSummary {
                    total: total_lines,
                    selected: total_selected_lines,
                    emitted: snippets.iter().map(|snippet| snippet.lines.len()).sum(),
                },
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
        struct ReadFileAroundMatchContext {
            before: usize,
            after: usize,
        }

        #[derive(Serialize)]
        struct ReadFileAroundMatchSummary {
            snippets: usize,
            matches: ReadFileAroundMatchMatchSummary,
            lines: ReadFileAroundMatchLineSummary,
        }

        #[derive(Serialize)]
        struct ReadFileAroundMatchMatchSummary {
            returned: usize,
            total: usize,
        }

        #[derive(Serialize)]
        struct ReadFileAroundMatchLineSummary {
            observed: usize,
        }

        #[derive(Serialize)]
        struct AroundMatchResult {
            schema_version: u8,
            tool: &'static str,
            query: String,
            file: String,
            match_mode: &'static str,
            context: ReadFileAroundMatchContext,
            snippets: Vec<AroundMatchSnippet>,
            summary: ReadFileAroundMatchSummary,
            truncated: bool,
            warnings: Vec<String>,
        }

        let returned_matches = snippets.iter().map(|snippet| snippet.matches.len()).sum();
        let snippet_count = snippets.len();
        let result = AroundMatchResult {
            schema_version: 2,
            tool: "read_file_around_match",
            query: query.to_owned(),
            file,
            match_mode: if is_regex { "regex" } else { "literal" },
            context: ReadFileAroundMatchContext { before, after },
            snippets,
            summary: ReadFileAroundMatchSummary {
                snippets: snippet_count,
                matches: ReadFileAroundMatchMatchSummary {
                    returned: returned_matches,
                    total: total_matches,
                },
                lines: ReadFileAroundMatchLineSummary {
                    observed: observed_lines,
                },
            },
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
