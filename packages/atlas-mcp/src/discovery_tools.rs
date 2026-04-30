//! MCP tools for file-path and content discovery outside the graph-symbol path.
//!
//! These tools complement the graph-first workflow:
//! - Use `query_graph` for symbol/relationship questions against indexed code.
//! - Use `search_files` when you need files by name/glob (e.g. config, templates, SQL).
//! - Use `search_content` when you need literal or regex matches in file content.

use anyhow::{Context, Result};
use atlas_core::{BudgetManager, BudgetPolicy, BudgetReport};
use atlas_repo::CanonicalRepoPath;
use atlas_review::{DocsSectionSelector, lookup_docs_section};
use atlas_store_sqlite::Store;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use regex::Regex;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::output::{OutputFormat, render_serializable};

/// Validate a user-supplied `subpath` and return the absolute walk root.
///
/// Security: `CanonicalRepoPath::from_repo_relative` rejects any `..` segments
/// that would escape `repo_root`, absolute paths, and empty inputs. Without this
/// guard a caller could supply `../../etc` and the `WalkBuilder` would traverse
/// outside the repository boundary before the `strip_prefix` output filter
/// had any effect.
///
/// Falls back to `repo_root` when the resolved candidate is not a directory.
fn resolve_subpath_walk_root(repo_root: &str, subpath: &str) -> Result<String> {
    let canonical = CanonicalRepoPath::from_repo_relative(subpath)
        .map_err(|e| anyhow::anyhow!("invalid subpath '{subpath}': {e}"))?;
    let candidate = Path::new(repo_root).join(canonical.as_str());
    if candidate.is_dir() {
        Ok(candidate.to_string_lossy().into_owned())
    } else {
        Ok(repo_root.to_owned())
    }
}

fn resolve_repo_file_path(repo_root: &str, path: &str) -> Result<(String, PathBuf)> {
    let canonical = CanonicalRepoPath::from_repo_relative(path)
        .map_err(|error| anyhow::anyhow!("invalid file path '{path}': {error}"))?;
    let absolute = Path::new(repo_root).join(canonical.as_str());
    Ok((canonical.as_str().to_owned(), absolute))
}

fn invalid_search_content_regex_error(query: &str, error: impl std::fmt::Display) -> anyhow::Error {
    let escaped_example = r"Command::Context|Context \{";
    anyhow::anyhow!(
        "invalid regex pattern for search_content query '{query}': {error}. \
         search_content keeps is_regex=true strict and does not fall back to literal search. \
         Set is_regex=false for literal text search, or escape regex metacharacters, e.g. {escaped_example}"
    )
}

// ---------------------------------------------------------------------------
// Generated / vendor patterns excluded by default in search_content
// ---------------------------------------------------------------------------

/// Path patterns (repo-relative) suppressed when `exclude_generated=true`.
const GENERATED_PATTERNS: &[&str] = &[
    "node_modules/**",
    "**/node_modules/**",
    "package-lock.json",
    "**/package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "*.min.js",
    "**/*.min.js",
    "*.min.css",
    "**/*.min.css",
    "*.bundle.js",
    "**/*.bundle.js",
    "dist/**",
    "**/dist/**",
    "build/**",
    "**/build/**",
    "vendor/**",
    "**/vendor/**",
    ".next/**",
    "**/.next/**",
    "target/**",
    "__pycache__/**",
    "**/__pycache__/**",
    ".venv/**",
    "**/.venv/**",
];

// ---------------------------------------------------------------------------
// Shared output types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ContentMatch {
    file: String,
    line: u64,
    text: String,
    /// Present only for context lines: `"before"` or `"after"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
}

#[derive(Serialize)]
struct RichSnippetLine {
    line: u64,
    text: String,
    kind: &'static str,
}

#[derive(Serialize)]
struct RichSnippet {
    file: String,
    match_line: u64,
    snippet: String,
    lines: Vec<RichSnippetLine>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RequestedLineRange {
    start_line: usize,
    end_line: usize,
}

#[derive(Serialize)]
struct ExcerptLine {
    line: u64,
    text: String,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
struct AroundMatchLine {
    line: u64,
    text: String,
    kind: &'static str,
}

#[derive(Serialize)]
struct AroundMatchSnippet {
    start_line: u64,
    end_line: u64,
    match_lines: Vec<u64>,
    content: String,
    lines: Vec<AroundMatchLine>,
}

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

fn parse_excerpt_selection(
    args: Option<&serde_json::Value>,
) -> Result<(Vec<RequestedLineRange>, &'static str)> {
    let start_line = u64_arg(args, "start_line").map(|value| value as usize);
    let end_line = u64_arg(args, "end_line").map(|value| value as usize);
    let line = u64_arg(args, "line").map(|value| value as usize);
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
        .transpose()?
        .unwrap_or_default();

    let selectors_used = usize::from(!line_ranges.is_empty())
        + usize::from(start_line.is_some() || end_line.is_some())
        + usize::from(line.is_some());
    if selectors_used != 1 {
        anyhow::bail!(
            "provide exactly one selector: line_ranges, start_line/end_line, or line with optional before/after"
        );
    }

    if !line_ranges.is_empty() {
        return Ok((normalize_requested_ranges(line_ranges), "line_ranges"));
    }

    if let Some(line) = line {
        let start_line = line.saturating_sub(before).max(1);
        let end_line = line.saturating_add(after);
        return Ok((
            vec![validate_requested_range(start_line, end_line)?],
            "line_context",
        ));
    }

    if before > 0 || after > 0 {
        anyhow::bail!("before/after are only valid with line selector");
    }

    let start_line =
        start_line.ok_or_else(|| anyhow::anyhow!("missing required argument: start_line"))?;
    let end_line =
        end_line.ok_or_else(|| anyhow::anyhow!("missing required argument: end_line"))?;
    Ok((
        vec![validate_requested_range(start_line, end_line)?],
        "single_range",
    ))
}

// ---------------------------------------------------------------------------
// search_files
// ---------------------------------------------------------------------------

/// MCP tool: `read_file_excerpt` — bounded file reads by line range or line-with-context.
pub(crate) fn tool_read_file_excerpt(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let file =
        str_arg(args, "file")?.ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?;
    let requested_max_lines = u64_arg(args, "max_lines").unwrap_or(200) as usize;
    let max_lines = budgets.resolve_limit(
        policy.review_context_extraction.nodes,
        "review_context_extraction.max_nodes",
        Some(requested_max_lines),
    );
    let (file, abs_path) = resolve_repo_file_path(repo_root, file)?;
    if !abs_path.is_file() {
        anyhow::bail!("file not found: {file}");
    }

    let (requested_ranges, mode) = parse_excerpt_selection(args)?;
    let contents = std::fs::read_to_string(&abs_path)
        .with_context(|| format!("cannot read UTF-8 text file '{file}'"))?;
    let lines: Vec<&str> = contents.lines().collect();
    let total_lines = lines.len();

    let mut resolved_ranges = Vec::with_capacity(requested_ranges.len());
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
    let mut remaining_lines = max_lines;
    let mut excerpts = Vec::with_capacity(resolved_ranges.len());
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
        excerpts.push(FileExcerpt {
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

    let atlas_hint = if total_lines == 0 {
        Some(format!("{file} is empty."))
    } else if truncated {
        Some(format!(
            "Excerpt truncated to {max_lines} lines. Narrow line_ranges or raise max_lines within policy limits."
        ))
    } else {
        None
    };

    #[derive(Serialize)]
    struct ReadFileExcerptResult {
        file: String,
        mode: &'static str,
        total_lines: usize,
        excerpts: Vec<FileExcerpt>,
        excerpt_count: usize,
        truncated: bool,
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    let result = ReadFileExcerptResult {
        file,
        mode,
        total_lines,
        excerpt_count: excerpts.len(),
        excerpts,
        truncated,
        atlas_result_kind: "file_excerpt",
        atlas_hint,
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
}

/// MCP tool: `get_docs_section` — resolve a Markdown section by heading path/slug or line.
pub(crate) fn tool_get_docs_section(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let file =
        str_arg(args, "file")?.ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?;
    let heading = str_arg(args, "heading")?.map(str::to_owned);
    let line = u64_arg(args, "line").map(|value| value as u32);
    if usize::from(heading.is_some()) + usize::from(line.is_some()) != 1 {
        anyhow::bail!("provide exactly one selector: heading or line");
    }

    let max_bytes = u64_arg(args, "max_bytes").unwrap_or(16_384) as usize;
    let (file, _) = resolve_repo_file_path(repo_root, file)?;
    let store =
        Store::open(db_path).with_context(|| format!("cannot open atlas store at '{db_path}'"))?;
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

    let mut response = render_tool_result(&result, output_format)?;
    response["file"] = serde_json::Value::String(result.file.clone());
    Ok(response)
}

/// MCP tool: `read_file_around_match` — read merged snippets around matches in one file.
pub(crate) fn tool_read_file_around_match(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let file =
        str_arg(args, "file")?.ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?;
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
    let (file, abs_path) = resolve_repo_file_path(repo_root, file)?;
    if !abs_path.is_file() {
        anyhow::bail!("file not found: {file}");
    }

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
        .map_err(|error| invalid_search_content_regex_error(query, error))?;

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
    let atlas_hint = if total_matches == 0 {
        Some(format!("No matches for '{query}' in {file}."))
    } else if truncated {
        Some("Snippets truncated by max_matches or max_lines budget.".to_owned())
    } else {
        None
    };

    #[derive(Serialize)]
    struct AroundMatchResult {
        file: String,
        query: String,
        is_regex: bool,
        case_sensitive: bool,
        total_matches: usize,
        returned_matches: usize,
        snippet_count: usize,
        snippets: Vec<AroundMatchSnippet>,
        truncated: bool,
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    let returned_matches = snippets
        .iter()
        .map(|snippet| snippet.match_lines.len())
        .sum();
    let result = AroundMatchResult {
        file,
        query: query.to_owned(),
        is_regex,
        case_sensitive,
        total_matches,
        returned_matches,
        snippet_count: snippets.len(),
        snippets,
        truncated,
        atlas_result_kind: "file_match_snippets",
        atlas_hint,
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
}

/// MCP tool: `search_files` — file-path discovery by glob pattern.
///
/// Walks the repo honouring `.gitignore` / `.git/info/exclude` / `.atlasignore`
/// rules (via the `ignore` crate `WalkBuilder`). Matches filenames and paths
/// against the required `pattern` glob and optional `globs` include-path filters.
pub(crate) fn tool_search_files(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let pattern = str_arg(args, "pattern")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: pattern"))?
        .to_owned();
    let globs = string_array_arg(args, "globs")?;
    let exclude_globs = string_array_arg(args, "exclude_globs")?;
    let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(false);
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let result_limit = budgets.resolve_limit(
        policy.review_context_extraction.files,
        "review_context_extraction.max_files",
        None,
    );

    let name_matcher = build_globset(&[&pattern], case_sensitive)
        .with_context(|| format!("invalid pattern glob: {pattern}"))?;

    let include_filter = if globs.is_empty() {
        None
    } else {
        Some(build_globset(&globs, case_sensitive).context("invalid globs filter")?)
    };

    let exclude_filter = if exclude_globs.is_empty() {
        None
    } else {
        Some(build_globset(&exclude_globs, false).context("invalid exclude_globs filter")?)
    };

    let walk_root = match subpath.as_deref() {
        Some(sp) => resolve_subpath_walk_root(repo_root, sp)?,
        None => repo_root.to_owned(),
    };

    let atlasignore_path = Path::new(repo_root).join(".atlasignore");
    let mut walker = ignore::WalkBuilder::new(&walk_root);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true);
    if atlasignore_path.exists() {
        walker.add_ignore(&atlasignore_path);
    }

    let mut files: Vec<String> = Vec::new();
    let mut truncated = false;

    for entry in walker.build().flatten() {
        if files.len() >= result_limit {
            truncated = true;
            break;
        }
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if !ft.is_file() {
            continue;
        }

        let full_path = entry.path();
        let rel_path = match full_path.strip_prefix(repo_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        if let Some(ref filter) = include_filter
            && !filter.is_match(&*rel_path)
        {
            continue;
        }

        if let Some(ref excl) = exclude_filter
            && excl.is_match(&*rel_path)
        {
            continue;
        }

        // Match against filename component and full repo-relative path.
        let file_name = full_path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        if !name_matcher.is_match(file_name.as_ref()) && !name_matcher.is_match(&*rel_path) {
            continue;
        }

        files.push(rel_path);
    }

    files.sort_unstable();

    let result_count = files.len();
    if truncated {
        budgets.record_usage(
            policy.review_context_extraction.files,
            "review_context_extraction.max_files",
            result_limit,
            result_limit.saturating_add(1),
            true,
        );
    }
    let atlas_hint = if result_count == 0 {
        Some(
            "No files matched. Try a broader glob (e.g. '*.rs' instead of 'foo*.rs'), \
             verify the pattern syntax, or use search_content for content-based lookup.",
        )
    } else {
        None
    };

    #[derive(Serialize)]
    struct SearchFilesResult<'a> {
        files: Vec<String>,
        result_count: usize,
        truncated: bool,
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<&'a str>,
    }

    let result = SearchFilesResult {
        files,
        result_count,
        truncated,
        atlas_result_kind: "file_paths",
        atlas_hint,
    };

    let mut response = render_tool_result(&result, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "review_context_extraction.max_files",
            result_limit,
            result_count,
        ),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// search_content
// ---------------------------------------------------------------------------

/// MCP tool: `search_content` — content search by literal string or regex.
///
/// Walks the repo with the same ignore semantics as `search_files`. Content
/// matching uses `grep-searcher` + `grep-regex` for robust binary detection,
/// encoding handling, and context-line support.
pub(crate) fn tool_search_content(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let query = str_arg(args, "query")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: query"))?
        .to_owned();
    let globs = string_array_arg(args, "globs")?;
    let exclude_globs = string_array_arg(args, "exclude_globs")?;
    let exclude_generated = bool_arg(args, "exclude_generated").unwrap_or(true);
    let is_regex = bool_arg(args, "is_regex").unwrap_or(false);
    let context_lines = u64_arg(args, "context_lines").unwrap_or(0) as usize;
    let requested_max_results = u64_arg(args, "max_results").unwrap_or(50) as usize;
    let max_results = budgets.resolve_limit(
        policy.review_context_extraction.nodes,
        "review_context_extraction.max_nodes",
        Some(requested_max_results),
    );
    let rich_snippets = bool_arg(args, "rich_snippets").unwrap_or(false);
    let snippet_context_lines = u64_arg(args, "snippet_context_lines")
        .map(|value| value as usize)
        .unwrap_or_else(|| {
            if rich_snippets {
                context_lines.max(2)
            } else {
                0
            }
        });
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);

    // Escape for literal search; use as-is for regex.
    let pattern = if is_regex {
        query.clone()
    } else {
        regex::escape(&query)
    };

    let matcher = RegexMatcherBuilder::new()
        // Literal queries are case-insensitive by default; regex queries respect user intent.
        .case_insensitive(!is_regex)
        .build(&pattern)
        .map_err(|error| invalid_search_content_regex_error(&query, error))?;

    let rich_snippet_regex = if rich_snippets {
        Some(
            regex::RegexBuilder::new(&pattern)
                .case_insensitive(!is_regex)
                .build()
                .map_err(|error| invalid_search_content_regex_error(&query, error))?,
        )
    } else {
        None
    };

    let searcher_proto = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(b'\x00'))
        .before_context(context_lines)
        .after_context(context_lines)
        .build();

    let include_filter = if globs.is_empty() {
        None
    } else {
        Some(build_globset(&globs, false).context("invalid globs filter")?)
    };

    let exclude_filter = if exclude_globs.is_empty() {
        None
    } else {
        Some(build_globset(&exclude_globs, false).context("invalid exclude_globs filter")?)
    };

    let generated_filter = if exclude_generated {
        Some(
            build_globset(GENERATED_PATTERNS, false)
                .context("generated-patterns compile failed")?,
        )
    } else {
        None
    };

    let walk_root = match subpath.as_deref() {
        Some(sp) => resolve_subpath_walk_root(repo_root, sp)?,
        None => repo_root.to_owned(),
    };

    let atlasignore_path = Path::new(repo_root).join(".atlasignore");
    let mut walker = ignore::WalkBuilder::new(&walk_root);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true);
    if atlasignore_path.exists() {
        walker.add_ignore(&atlasignore_path);
    }

    let mut matches: Vec<ContentMatch> = Vec::new();
    let mut rich_snippet_results: Vec<RichSnippet> = Vec::new();
    let mut truncated = false;

    'walk: for entry in walker.build().flatten() {
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if !ft.is_file() {
            continue;
        }

        let full_path = entry.path().to_path_buf();
        let rel_path = match full_path.strip_prefix(repo_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        if let Some(ref filter) = include_filter
            && !filter.is_match(&*rel_path)
        {
            continue;
        }
        if let Some(ref excl) = exclude_filter
            && excl.is_match(&*rel_path)
        {
            continue;
        }
        if let Some(ref generated) = generated_filter
            && generated.is_match(&*rel_path)
        {
            continue;
        }

        let remaining = max_results.saturating_sub(matches.len());
        if remaining == 0 {
            truncated = true;
            break 'walk;
        }

        let file_hits = search_file(
            &full_path,
            &rel_path,
            &matcher,
            searcher_proto.clone(),
            remaining,
        );
        match file_hits {
            Ok(hits) => {
                let would_overflow = matches.len() + hits.len() > max_results;
                for hit in hits {
                    if matches.len() >= max_results {
                        truncated = true;
                        break 'walk;
                    }
                    matches.push(hit);
                }
                if would_overflow {
                    truncated = true;
                    break 'walk;
                }
                if let Some(ref regex) = rich_snippet_regex {
                    let remaining_snippets = max_results.saturating_sub(rich_snippet_results.len());
                    if remaining_snippets > 0
                        && let Ok(snippets) = collect_rich_snippets(
                            &full_path,
                            &rel_path,
                            regex,
                            snippet_context_lines,
                            remaining_snippets,
                        )
                    {
                        rich_snippet_results.extend(snippets);
                    }
                }
            }
            // Skip unreadable / binary files silently.
            Err(_) => continue,
        }
    }

    let result_count = matches.len();

    // Hint: if the query looks like a symbol name, suggest query_graph.
    let looks_like_symbol = !query.contains(' ')
        && !query.contains('.')
        && query.chars().all(|c| c.is_alphanumeric() || c == '_');

    let atlas_hint = if result_count == 0 {
        Some(format!(
            "No matches for '{query}'. Try broadening the query, enabling is_regex=true \
             for pattern matching, or check that the file type is covered by your globs filter.",
        ))
    } else if looks_like_symbol {
        Some(format!(
            "'{query}' looks like a symbol name. For callers, callees, and graph context \
             prefer query_graph or symbol_neighbors.",
        ))
    } else {
        None
    };

    #[derive(Serialize)]
    struct SearchContentResult {
        matches: Vec<ContentMatch>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        rich_snippets: Vec<RichSnippet>,
        result_count: usize,
        truncated: bool,
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    let result = SearchContentResult {
        matches,
        rich_snippets: rich_snippet_results,
        result_count,
        truncated,
        atlas_result_kind: "content_matches",
        atlas_hint,
    };

    if truncated {
        budgets.record_usage(
            policy.review_context_extraction.nodes,
            "review_context_extraction.max_nodes",
            max_results,
            max_results.saturating_add(1),
            true,
        );
    }

    let mut response = render_tool_result(&result, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "review_context_extraction.max_nodes",
            max_results,
            requested_max_results.max(result_count),
        ),
    );
    Ok(response)
}

fn collect_rich_snippets(
    path: &Path,
    rel_path: &str,
    regex: &Regex,
    context_lines: usize,
    max: usize,
) -> Result<Vec<RichSnippet>> {
    let contents = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = contents.lines().collect();
    let mut snippets = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if snippets.len() >= max {
            break;
        }
        if !regex.is_match(line) {
            continue;
        }

        let start = index.saturating_sub(context_lines);
        let end = (index + context_lines + 1).min(lines.len());
        let mut snippet_lines = Vec::new();
        for (line_index, line_text) in lines.iter().enumerate().take(end).skip(start) {
            let kind = if line_index < index {
                "before"
            } else if line_index == index {
                "match"
            } else {
                "after"
            };
            snippet_lines.push(RichSnippetLine {
                line: (line_index + 1) as u64,
                text: (*line_text).to_owned(),
                kind,
            });
        }

        let snippet = snippet_lines
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        snippets.push(RichSnippet {
            file: rel_path.to_owned(),
            match_line: (index + 1) as u64,
            snippet,
            lines: snippet_lines,
        });
    }

    Ok(snippets)
}

// ---------------------------------------------------------------------------
// search_templates
// ---------------------------------------------------------------------------

/// Template file extensions recognised by default.
const TEMPLATE_EXTENSIONS: &[&str] = &[
    "*.html",
    "*.htm",
    "*.j2",
    "*.jinja",
    "*.jinja2",
    "*.hbs",
    "*.handlebars",
    "*.mustache",
    "*.tera",
    "*.mako",
    "*.twig",
    "*.liquid",
    "*.erb",
    "*.haml",
    "*.pug",
];

/// MCP tool: `search_templates` — discover template files by extension.
///
/// Defaults to all common template extensions. Pass `kind` to narrow to a
/// specific template engine (html, jinja, handlebars, tera, mako, mustache,
/// twig, liquid, erb, haml, pug). Supports `subpath` for monorepo scoping
/// and `exclude_globs` for fine-grained exclusion.
pub(crate) fn tool_search_templates(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let globs = string_array_arg(args, "globs")?;
    let exclude_globs = string_array_arg(args, "exclude_globs")?;
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(false);
    let requested_max_results = u64_arg(args, "max_results").unwrap_or(100) as usize;
    let max_results = budgets.resolve_limit(
        policy.review_context_extraction.files,
        "review_context_extraction.max_files",
        Some(requested_max_results),
    );

    // Determine which extension patterns to use based on `kind`.
    let extension_patterns: Vec<&str> = match kind.as_deref() {
        Some("html") => vec!["*.html", "*.htm"],
        Some("jinja") => vec!["*.j2", "*.jinja", "*.jinja2"],
        Some("handlebars") => vec!["*.hbs", "*.handlebars"],
        Some("tera") => vec!["*.tera"],
        Some("mako") => vec!["*.mako"],
        Some("mustache") => vec!["*.mustache"],
        Some("twig") => vec!["*.twig"],
        Some("liquid") => vec!["*.liquid"],
        Some("erb") => vec!["*.erb"],
        Some("haml") => vec!["*.haml"],
        Some("pug") => vec!["*.pug"],
        None | Some(_) => TEMPLATE_EXTENSIONS.to_vec(),
    };

    let name_matcher = build_globset(&extension_patterns, case_sensitive)
        .context("invalid template extension patterns")?;

    let include_filter = if globs.is_empty() {
        None
    } else {
        Some(build_globset(&globs, case_sensitive).context("invalid globs filter")?)
    };

    let exclude_filter = if exclude_globs.is_empty() {
        None
    } else {
        Some(build_globset(&exclude_globs, false).context("invalid exclude_globs filter")?)
    };

    let walk_root = match subpath.as_deref() {
        Some(sp) => resolve_subpath_walk_root(repo_root, sp)?,
        None => repo_root.to_owned(),
    };

    let atlasignore_path = Path::new(repo_root).join(".atlasignore");
    let mut walker = ignore::WalkBuilder::new(&walk_root);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true);
    if atlasignore_path.exists() {
        walker.add_ignore(&atlasignore_path);
    }

    let mut files: Vec<String> = Vec::new();
    let mut truncated = false;

    for entry in walker.build().flatten() {
        if files.len() >= max_results {
            truncated = true;
            break;
        }
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if !ft.is_file() {
            continue;
        }

        let full_path = entry.path();
        let rel_path = match full_path.strip_prefix(repo_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        if let Some(ref filter) = include_filter
            && !filter.is_match(&*rel_path)
        {
            continue;
        }
        if let Some(ref excl) = exclude_filter
            && excl.is_match(&*rel_path)
        {
            continue;
        }

        let file_name = full_path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        if !name_matcher.is_match(file_name.as_ref()) {
            continue;
        }

        files.push(rel_path);
    }

    files.sort_unstable();

    let result_count = files.len();
    if truncated {
        budgets.record_usage(
            policy.review_context_extraction.files,
            "review_context_extraction.max_files",
            max_results,
            max_results.saturating_add(1),
            true,
        );
    }
    let atlas_hint = if result_count == 0 {
        let kind_hint = kind.as_deref().unwrap_or("any template");
        Some(format!(
            "No {kind_hint} template files found. Verify the repo contains template files or \
             widen the search with a broader `kind` or by removing `globs` filters.",
        ))
    } else {
        None
    };

    #[derive(Serialize)]
    struct SearchTemplatesResult {
        files: Vec<String>,
        result_count: usize,
        truncated: bool,
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    let result = SearchTemplatesResult {
        files,
        result_count,
        truncated,
        atlas_result_kind: "template_files",
        atlas_hint,
    };

    let mut response = render_tool_result(&result, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "review_context_extraction.max_files",
            max_results,
            requested_max_results.max(result_count),
        ),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// search_text_assets
// ---------------------------------------------------------------------------

/// Text asset kinds and their associated glob patterns.
const TEXT_ASSET_SQL: &[&str] = &["*.sql"];
const TEXT_ASSET_CONFIG: &[&str] = &["*.toml", "*.yaml", "*.yml", "*.ini", "*.cfg", "*.conf"];
const TEXT_ASSET_ENV: &[&str] = &[".env", ".env.*", "*.env"];
const TEXT_ASSET_PROMPT: &[&str] = &[
    "*.prompt",
    "*.promptfile",
    "*.p8",
    "prompts/*.md",
    "prompts/**/*.md",
    "**/*.prompt.md",
];
const TEXT_ASSET_ALL: &[&str] = &[
    "*.sql",
    "*.toml",
    "*.yaml",
    "*.yml",
    "*.ini",
    "*.cfg",
    "*.conf",
    ".env",
    ".env.*",
    "*.env",
    "*.prompt",
    "*.promptfile",
    "*.p8",
];

/// MCP tool: `search_text_assets` — discover SQL, config, env, and prompt files.
///
/// Use this to locate non-template text assets that are not indexed as graph
/// symbols. Pass `kind` to narrow to a specific asset type: `sql`, `config`,
/// `env`, or `prompt`. Supports `subpath` for monorepo scoping and
/// `exclude_globs` for fine-grained exclusion.
pub(crate) fn tool_search_text_assets(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let globs = string_array_arg(args, "globs")?;
    let exclude_globs = string_array_arg(args, "exclude_globs")?;
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(false);
    let requested_max_results = u64_arg(args, "max_results").unwrap_or(100) as usize;
    let max_results = budgets.resolve_limit(
        policy.review_context_extraction.files,
        "review_context_extraction.max_files",
        Some(requested_max_results),
    );

    let extension_patterns: Vec<&str> = match kind.as_deref() {
        Some("sql") => TEXT_ASSET_SQL.to_vec(),
        Some("config") => TEXT_ASSET_CONFIG.to_vec(),
        Some("env") => TEXT_ASSET_ENV.to_vec(),
        Some("prompt") => TEXT_ASSET_PROMPT.to_vec(),
        None | Some(_) => TEXT_ASSET_ALL.to_vec(),
    };

    let name_matcher = build_globset(&extension_patterns, case_sensitive)
        .context("invalid text-asset extension patterns")?;

    let include_filter = if globs.is_empty() {
        None
    } else {
        Some(build_globset(&globs, case_sensitive).context("invalid globs filter")?)
    };

    let exclude_filter = if exclude_globs.is_empty() {
        None
    } else {
        Some(build_globset(&exclude_globs, false).context("invalid exclude_globs filter")?)
    };

    let walk_root = match subpath.as_deref() {
        Some(sp) => resolve_subpath_walk_root(repo_root, sp)?,
        None => repo_root.to_owned(),
    };

    let atlasignore_path = Path::new(repo_root).join(".atlasignore");
    let mut walker = ignore::WalkBuilder::new(&walk_root);
    walker
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true);
    if atlasignore_path.exists() {
        walker.add_ignore(&atlasignore_path);
    }

    let mut files: Vec<String> = Vec::new();
    let mut truncated = false;

    for entry in walker.build().flatten() {
        if files.len() >= max_results {
            truncated = true;
            break;
        }
        let ft = match entry.file_type() {
            Some(ft) => ft,
            None => continue,
        };
        if !ft.is_file() {
            continue;
        }

        let full_path = entry.path();
        let rel_path = match full_path.strip_prefix(repo_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        if let Some(ref filter) = include_filter
            && !filter.is_match(&*rel_path)
        {
            continue;
        }
        if let Some(ref excl) = exclude_filter
            && excl.is_match(&*rel_path)
        {
            continue;
        }

        let file_name = full_path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        // Also match the full repo-relative path for patterns like `prompts/*.md`.
        if !name_matcher.is_match(file_name.as_ref()) && !name_matcher.is_match(&*rel_path) {
            continue;
        }

        files.push(rel_path);
    }

    files.sort_unstable();

    let result_count = files.len();
    if truncated {
        budgets.record_usage(
            policy.review_context_extraction.files,
            "review_context_extraction.max_files",
            max_results,
            max_results.saturating_add(1),
            true,
        );
    }
    let atlas_hint = if result_count == 0 {
        let kind_hint = kind.as_deref().unwrap_or("any text asset");
        Some(format!(
            "No {kind_hint} files found. Supported kinds: sql, config, env, prompt. \
             Try broadening with `kind` omitted or check the subpath.",
        ))
    } else {
        None
    };

    #[derive(Serialize)]
    struct SearchTextAssetsResult {
        files: Vec<String>,
        result_count: usize,
        truncated: bool,
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    let result = SearchTextAssetsResult {
        files,
        result_count,
        truncated,
        atlas_result_kind: "text_asset_files",
        atlas_hint,
    };

    let mut response = render_tool_result(&result, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "review_context_extraction.max_files",
            max_results,
            requested_max_results.max(result_count),
        ),
    );
    Ok(response)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Run grep-searcher against a single file and collect up to `max` hits.
fn search_file(
    path: &std::path::PathBuf,
    rel_path: &str,
    matcher: &grep_regex::RegexMatcher,
    mut searcher: Searcher,
    max: usize,
) -> Result<Vec<ContentMatch>> {
    struct HitCollector<'a> {
        rel_path: &'a str,
        out: &'a mut Vec<ContentMatch>,
        max: usize,
    }

    impl Sink for HitCollector<'_> {
        type Error = std::io::Error;

        fn matched(
            &mut self,
            _searcher: &Searcher,
            mat: &SinkMatch<'_>,
        ) -> std::result::Result<bool, Self::Error> {
            if self.out.len() >= self.max {
                return Ok(false);
            }
            let line = mat.line_number().unwrap_or(0);
            let text = String::from_utf8_lossy(mat.bytes()).trim_end().to_owned();
            self.out.push(ContentMatch {
                file: self.rel_path.to_owned(),
                line,
                text,
                kind: None,
            });
            Ok(self.out.len() < self.max)
        }

        fn context(
            &mut self,
            _searcher: &Searcher,
            ctx: &SinkContext<'_>,
        ) -> std::result::Result<bool, Self::Error> {
            if self.out.len() >= self.max {
                return Ok(false);
            }
            let line = ctx.line_number().unwrap_or(0);
            let text = String::from_utf8_lossy(ctx.bytes()).trim_end().to_owned();
            let kind = match ctx.kind() {
                grep_searcher::SinkContextKind::Before => Some("before"),
                grep_searcher::SinkContextKind::After => Some("after"),
                grep_searcher::SinkContextKind::Other => Some("other"),
            };
            self.out.push(ContentMatch {
                file: self.rel_path.to_owned(),
                line,
                text,
                kind,
            });
            Ok(self.out.len() < self.max)
        }
    }

    let mut out: Vec<ContentMatch> = Vec::new();
    let mut sink = HitCollector {
        rel_path,
        out: &mut out,
        max,
    };
    searcher.search_path(matcher, path, &mut sink)?;
    Ok(out)
}

/// Build a `GlobSet` from a slice of pattern strings.
fn build_globset(patterns: &[impl AsRef<str>], case_sensitive: bool) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        let glob = GlobBuilder::new(p.as_ref())
            .case_insensitive(!case_sensitive)
            .build()
            .with_context(|| format!("invalid glob pattern: {}", p.as_ref()))?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}

fn str_arg<'a>(args: Option<&'a serde_json::Value>, key: &str) -> Result<Option<&'a str>> {
    Ok(args.and_then(|a| a.get(key)).and_then(|v| v.as_str()))
}

fn u64_arg(args: Option<&serde_json::Value>, key: &str) -> Option<u64> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_u64())
}

fn bool_arg(args: Option<&serde_json::Value>, key: &str) -> Option<bool> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_bool())
}

fn string_array_arg(args: Option<&serde_json::Value>, key: &str) -> Result<Vec<String>> {
    Ok(args
        .and_then(|a| a.get(key))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

fn load_budget_policy(repo_root: &str) -> Result<BudgetPolicy> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    config.budget_policy()
}

fn inject_budget_metadata(response: &mut serde_json::Value, budget: &BudgetReport) {
    response["budget_status"] = serde_json::json!(budget.budget_status);
    response["budget_hit"] = serde_json::json!(budget.budget_hit);
    response["budget_name"] = serde_json::json!(&budget.budget_name);
    response["budget_limit"] = serde_json::json!(budget.budget_limit);
    response["budget_observed"] = serde_json::json!(budget.budget_observed);
    response["partial"] = serde_json::json!(budget.partial);
    response["safe_to_answer"] = serde_json::json!(budget.safe_to_answer);
}

fn render_tool_result<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let rendered = render_serializable(value, output_format)?;
    let mut response = serde_json::json!({
        "content": [{
            "type": "text",
            "text": rendered.text,
            "mimeType": rendered.actual_format.mime_type(),
        }],
        "atlas_output_format": rendered.actual_format.as_str(),
        "atlas_requested_output_format": rendered.requested_format.as_str(),
    });
    if let Some(reason) = rendered.fallback_reason {
        response["atlas_fallback_reason"] = serde_json::Value::String(reason);
    }
    Ok(response)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{NodeId, kinds::NodeKind, model::Node};
    use atlas_store_sqlite::Store;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    /// Build a minimal fake repo with a `.git` dir (so `ignore::WalkBuilder`
    /// applies gitignore rules) and the given files.
    fn make_repo(files: &[(&str, &str)]) -> (TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap().to_owned();
        fs::create_dir_all(format!("{root}/.git")).unwrap();
        for (rel, content) in files {
            let full = format!("{root}/{rel}");
            if let Some(parent) = Path::new(&full).parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full, content).unwrap();
        }
        (dir, root)
    }

    fn markdown_heading(title: &str, path: &str, level: u32, start_line: u32) -> Node {
        Node {
            id: NodeId::UNSET,
            kind: NodeKind::Module,
            name: title.to_owned(),
            qualified_name: format!("README.md::heading::{path}"),
            file_path: "README.md".to_owned(),
            line_start: start_line,
            line_end: start_line,
            language: "markdown".to_owned(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "hash:README.md".to_owned(),
            extra_json: json!({ "level": level, "path": path }),
        }
    }

    fn seed_docs_index(root: &str, nodes: &[Node]) -> String {
        let db_path = format!("{root}/atlas.db");
        let mut store = Store::open(&db_path).expect("open store");
        store
            .replace_file_graph(
                "README.md",
                "hash:README.md",
                Some("markdown"),
                Some(32),
                nodes,
                &[],
            )
            .expect("replace README graph");
        db_path
    }

    // -----------------------------------------------------------------------
    // search_files
    // -----------------------------------------------------------------------

    #[test]
    fn search_files_finds_markdown() {
        let (_dir, root) = make_repo(&[
            ("README.md", "# hello"),
            ("docs/guide.md", "# guide"),
            ("src/main.rs", "fn main() {}"),
        ]);
        let args = serde_json::json!({ "pattern": "*.md" });
        let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(files.iter().any(|f| f.ends_with("README.md")), "{files:?}");
        assert!(files.iter().any(|f| f.ends_with("guide.md")), "{files:?}");
        assert!(!files.iter().any(|f| f.ends_with("main.rs")), "{files:?}");
        assert_eq!(v["atlas_result_kind"], "file_paths");
    }

    #[test]
    fn search_files_finds_sql_config_template() {
        let (_dir, root) = make_repo(&[
            ("schema.sql", "CREATE TABLE foo;"),
            ("config/app.toml", "[section]"),
            ("templates/index.html", "<html></html>"),
            ("src/lib.rs", ""),
        ]);
        for (pattern, expected) in [
            ("*.sql", "schema.sql"),
            ("*.toml", "app.toml"),
            ("*.html", "index.html"),
        ] {
            let args = serde_json::json!({ "pattern": pattern });
            let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
            let v: serde_json::Value =
                serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
            let files: Vec<&str> = v["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap())
                .collect();
            assert!(
                files.iter().any(|f| f.ends_with(expected)),
                "pattern={pattern} expected={expected} got={files:?}"
            );
        }
    }

    #[test]
    fn search_files_gitignore_excludes_node_modules() {
        let (_dir, root) = make_repo(&[
            (".gitignore", "node_modules/\n"),
            ("node_modules/index.js", "// vendor"),
            ("src/main.js", "// src"),
        ]);
        let args = serde_json::json!({ "pattern": "*.js" });
        let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            !files.iter().any(|f| f.contains("node_modules")),
            "node_modules leaked: {files:?}"
        );
        assert!(files.iter().any(|f| f.ends_with("main.js")), "{files:?}");
    }

    #[test]
    fn search_files_atlasignore_respected() {
        let (_dir, root) = make_repo(&[
            (".atlasignore", "secret.rs\n"),
            ("secret.rs", ""),
            ("public.rs", ""),
        ]);
        let args = serde_json::json!({ "pattern": "*.rs" });
        let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            !files.iter().any(|f| f.ends_with("secret.rs")),
            "secret.rs leaked: {files:?}"
        );
        assert!(files.iter().any(|f| f.ends_with("public.rs")), "{files:?}");
    }

    #[test]
    fn search_files_no_results_hint() {
        let (_dir, root) = make_repo(&[("src/main.rs", "")]);
        let args = serde_json::json!({ "pattern": "*.nonexistent" });
        let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(v["result_count"], 0);
        assert!(v["atlas_hint"].is_string(), "expected hint on empty result");
    }

    // -----------------------------------------------------------------------
    // read_file_excerpt
    // -----------------------------------------------------------------------

    #[test]
    fn read_file_excerpt_reads_single_range() {
        let (_dir, root) = make_repo(&[(
            "src/lib.rs",
            "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
        )]);
        let args = serde_json::json!({
            "file": "src/lib.rs",
            "start_line": 2,
            "end_line": 3,
        });
        let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(v["atlas_result_kind"], "file_excerpt");
        assert_eq!(v["mode"], "single_range");
        assert_eq!(v["excerpt_count"], 1);
        assert_eq!(v["excerpts"][0]["start_line"], 2);
        assert_eq!(v["excerpts"][0]["end_line"], 3);
        assert!(v["excerpts"][0]["content"].as_str().is_some_and(|text| {
            text.contains("fn two() {}") && text.contains("fn three() {}")
        }));
    }

    #[test]
    fn read_file_excerpt_supports_line_with_context() {
        let (_dir, root) = make_repo(&[(
            "src/lib.rs",
            "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
        )]);
        let args = serde_json::json!({
            "file": "src/lib.rs",
            "line": 3,
            "before": 1,
            "after": 1,
        });
        let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(v["mode"], "line_context");
        assert_eq!(v["excerpts"][0]["start_line"], 2);
        assert_eq!(v["excerpts"][0]["end_line"], 4);
        let lines = v["excerpts"][0]["lines"].as_array().expect("excerpt lines");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1]["line"], 3);
        assert_eq!(lines[1]["text"], "fn three() {}");
    }

    #[test]
    fn read_file_excerpt_merges_overlapping_ranges() {
        let (_dir, root) = make_repo(&[(
            "src/lib.rs",
            "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
        )]);
        let args = serde_json::json!({
            "file": "src/lib.rs",
            "line_ranges": [
                { "start_line": 1, "end_line": 2 },
                { "start_line": 2, "end_line": 4 }
            ],
        });
        let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(v["mode"], "line_ranges");
        assert_eq!(v["excerpt_count"], 1);
        assert_eq!(v["excerpts"][0]["start_line"], 1);
        assert_eq!(v["excerpts"][0]["end_line"], 4);
    }

    #[test]
    fn read_file_excerpt_truncates_to_budgeted_max_lines() {
        let (_dir, root) = make_repo(&[(
            "src/lib.rs",
            "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
        )]);
        let args = serde_json::json!({
            "file": "src/lib.rs",
            "start_line": 1,
            "end_line": 4,
            "max_lines": 2,
        });
        let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(resp["budget_name"], "review_context_extraction.max_nodes");
        assert_eq!(resp["budget_limit"], 2);
        assert_eq!(resp["budget_observed"], 4);
        assert_eq!(resp["budget_hit"], true);
        assert_eq!(v["truncated"], true);
        assert_eq!(v["excerpts"][0]["end_line"], 2);
    }

    #[test]
    fn read_file_excerpt_path_traversal_is_rejected() {
        let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}\n")]);
        for bad in &["../", "../../etc/passwd", "/etc/passwd"] {
            let args = serde_json::json!({
                "file": bad,
                "start_line": 1,
                "end_line": 1,
            });
            let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json);
            assert!(
                result.is_err(),
                "file path '{bad}' should be rejected as traversal attempt"
            );
        }
    }

    // -----------------------------------------------------------------------
    // get_docs_section
    // -----------------------------------------------------------------------

    #[test]
    fn get_docs_section_by_heading_path_returns_section() {
        let (_dir, root) = make_repo(&[(
            "README.md",
            "# Overview\nintro\n## Install\nstep one\n## Usage\nrun it\n",
        )]);
        let db_path = seed_docs_index(
            &root,
            &[
                markdown_heading("Overview", "document.overview", 1, 1),
                markdown_heading("Install", "document.overview.install", 2, 3),
                markdown_heading("Usage", "document.overview.usage", 2, 5),
            ],
        );
        let args = serde_json::json!({
            "file": "README.md",
            "heading": "document.overview.install",
        });
        let resp = tool_get_docs_section(Some(&args), &root, &db_path, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(v["atlas_result_kind"], "docs_section");
        assert_eq!(v["heading_path"], "document.overview.install");
        assert_eq!(v["heading_level"], 2);
        assert!(
            v["content"]
                .as_str()
                .is_some_and(|text| text.contains("step one"))
        );
    }

    #[test]
    fn get_docs_section_by_line_returns_containing_section() {
        let (_dir, root) = make_repo(&[(
            "README.md",
            "# Overview\nintro\n## Install\nstep one\n## Usage\nrun it\n",
        )]);
        let db_path = seed_docs_index(
            &root,
            &[
                markdown_heading("Overview", "document.overview", 1, 1),
                markdown_heading("Install", "document.overview.install", 2, 3),
                markdown_heading("Usage", "document.overview.usage", 2, 5),
            ],
        );
        let args = serde_json::json!({
            "file": "README.md",
            "line": 4,
        });
        let resp = tool_get_docs_section(Some(&args), &root, &db_path, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(v["heading_path"], "document.overview.install");
        assert_eq!(v["start_line"], 3);
        assert_eq!(v["end_line"], 4);
    }

    #[test]
    fn get_docs_section_returns_candidates_for_ambiguous_slug() {
        let (_dir, root) =
            make_repo(&[("README.md", "# One\n## Install\na\n# Two\n## Install\nb\n")]);
        let db_path = seed_docs_index(
            &root,
            &[
                markdown_heading("One", "document.one", 1, 1),
                markdown_heading("Install", "document.one.install", 2, 2),
                markdown_heading("Two", "document.two", 1, 4),
                markdown_heading("Install", "document.two.install", 2, 5),
            ],
        );
        let args = serde_json::json!({
            "file": "README.md",
            "heading": "install",
        });
        let resp = tool_get_docs_section(Some(&args), &root, &db_path, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(v["resolved"], false);
        assert_eq!(v["candidates"][0]["heading_path"], "document.one.install");
        assert_eq!(v["candidates"][1]["heading_path"], "document.two.install");
    }

    // -----------------------------------------------------------------------
    // read_file_around_match
    // -----------------------------------------------------------------------

    #[test]
    fn read_file_around_match_groups_nearby_matches() {
        let (_dir, root) = make_repo(&[(
            "src/lib.rs",
            "zero\nalpha target\nbeta\ngamma target\nomega\n",
        )]);
        let args = serde_json::json!({
            "file": "src/lib.rs",
            "query": "target",
            "before": 1,
            "after": 1,
        });
        let resp = tool_read_file_around_match(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(v["atlas_result_kind"], "file_match_snippets");
        assert_eq!(v["total_matches"], 2);
        assert_eq!(v["snippet_count"], 1);
        let snippet = &v["snippets"][0];
        assert_eq!(snippet["start_line"], 1);
        assert_eq!(snippet["end_line"], 5);
        assert_eq!(snippet["match_lines"][0], 2);
        assert_eq!(snippet["match_lines"][1], 4);
    }

    #[test]
    fn read_file_around_match_supports_regex() {
        let (_dir, root) = make_repo(&[("src/lib.rs", "alpha\nBeta\ngamma\n")]);
        let args = serde_json::json!({
            "file": "src/lib.rs",
            "query": "^[A-Z][a-z]+$",
            "is_regex": true,
            "before": 0,
            "after": 0,
        });
        let resp = tool_read_file_around_match(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(v["total_matches"], 1);
        assert_eq!(v["snippets"][0]["match_lines"][0], 2);
    }

    #[test]
    fn read_file_around_match_path_traversal_is_rejected() {
        let (_dir, root) = make_repo(&[("src/lib.rs", "target\n")]);
        let args = serde_json::json!({
            "file": "../etc/passwd",
            "query": "target",
        });
        let result = tool_read_file_around_match(Some(&args), &root, OutputFormat::Json);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // search_content
    // -----------------------------------------------------------------------

    #[test]
    fn search_content_literal_match() {
        let (_dir, root) = make_repo(&[
            (
                "src/auth.rs",
                "fn verify_token(tok: &str) -> bool {\n    true\n}\n",
            ),
            ("src/other.rs", "fn unrelated() {}\n"),
        ]);
        let args = serde_json::json!({ "query": "verify_token", "exclude_generated": false });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let ms = v["matches"].as_array().unwrap();
        assert!(!ms.is_empty(), "expected at least one match");
        assert!(
            ms.iter()
                .any(|m| m["file"].as_str().unwrap().ends_with("auth.rs")),
            "{ms:?}"
        );
        assert_eq!(v["atlas_result_kind"], "content_matches");
    }

    #[test]
    fn search_content_regex_match() {
        let (_dir, root) = make_repo(&[("src/lib.rs", "pub fn foo() {}\npub fn bar() {}\n")]);
        let args = serde_json::json!({
            "query": r"pub fn \w+",
            "is_regex": true,
            "exclude_generated": false
        });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(
            v["result_count"].as_u64().unwrap() >= 2,
            "expected ≥2 matches: {v}"
        );
    }

    #[test]
    fn search_content_invalid_regex_returns_guidance() {
        let (_dir, root) = make_repo(&[(
            "src/lib.rs",
            "pub enum Command {\n    Context { value: String },\n}\n",
        )]);
        let args = serde_json::json!({
            "query": "Command::Context|Context {",
            "is_regex": true,
            "exclude_generated": false
        });

        let error = tool_search_content(Some(&args), &root, OutputFormat::Json)
            .expect_err("invalid regex must return error");
        let message = error.to_string();

        assert!(
            message.contains("invalid regex pattern for search_content"),
            "expected invalid regex guidance, got: {message}"
        );
        assert!(
            message.contains("Set is_regex=false for literal text search"),
            "expected literal-search guidance, got: {message}"
        );
        assert!(
            message.contains(r"Command::Context|Context \{"),
            "expected escaped regex guidance, got: {message}"
        );
    }

    #[test]
    fn search_content_exclude_generated_node_modules() {
        let (_dir, root) = make_repo(&[
            ("node_modules/vendor.js", "function secret() {}"),
            ("src/main.js", "function app() {}"),
        ]);
        let args = serde_json::json!({ "query": "function", "exclude_generated": true });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let ms = v["matches"].as_array().unwrap();
        assert!(
            !ms.iter()
                .any(|m| m["file"].as_str().unwrap().contains("node_modules")),
            "node_modules leaked: {ms:?}"
        );
    }

    #[test]
    fn search_content_min_js_suppressed_by_default() {
        let (_dir, root) = make_repo(&[
            ("dist/app.min.js", "var x=1;function a(){return x}"),
            ("src/main.js", "function main() {}"),
        ]);
        let args = serde_json::json!({ "query": "function" });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let ms = v["matches"].as_array().unwrap();
        assert!(
            !ms.iter()
                .any(|m| m["file"].as_str().unwrap().ends_with(".min.js")),
            "min.js leaked: {ms:?}"
        );
    }

    #[test]
    fn search_content_max_results_truncates() {
        let files: Vec<(String, String)> = (0..10)
            .map(|i| (format!("src/f{i}.rs"), format!("fn target_{i}() {{}}")))
            .collect();
        let file_refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let (_dir, root) = make_repo(&file_refs);
        let args = serde_json::json!({
            "query": "target",
            "max_results": 3,
            "exclude_generated": false
        });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(
            v["result_count"].as_u64().unwrap() <= 3,
            "result_count exceeded max: {v}"
        );
        assert!(v["truncated"].as_bool().unwrap(), "expected truncated=true");
    }

    #[test]
    fn search_content_max_results_is_clamped_by_central_budget_policy() {
        let files: Vec<(String, String)> = (0..210)
            .map(|i| (format!("src/f{i}.rs"), format!("fn target_{i}() {{}}")))
            .collect();
        let file_refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let (_dir, root) = make_repo(&file_refs);
        let args = serde_json::json!({
            "query": "target",
            "max_results": 9999,
            "exclude_generated": false
        });

        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let body: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

        assert_eq!(resp["budget_status"], "partial_result");
        assert_eq!(resp["budget_hit"], true);
        assert_eq!(resp["budget_name"], "review_context_extraction.max_nodes");
        assert_eq!(resp["budget_limit"], 200);
        assert_eq!(resp["budget_observed"], 201);
        assert_eq!(body["result_count"], 200);
        assert_eq!(body["truncated"], true);
    }

    #[test]
    fn search_content_symbol_hint_present() {
        let (_dir, root) = make_repo(&[("src/lib.rs", "fn my_func() {}")]);
        let args = serde_json::json!({ "query": "my_func", "exclude_generated": false });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(v["atlas_hint"].is_string(), "expected symbol hint: {v}");
        assert!(
            v["atlas_hint"].as_str().unwrap().contains("query_graph"),
            "hint should mention query_graph: {v}"
        );
    }

    #[test]
    fn search_content_rich_snippets_are_opt_in() {
        let (_dir, root) = make_repo(&[(
            "src/lib.rs",
            "fn before() {}\nfn target() {}\nfn after() {}\n",
        )]);
        let args = serde_json::json!({
            "query": "target",
            "exclude_generated": false,
            "rich_snippets": true,
            "snippet_context_lines": 1
        });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let snippets = v["rich_snippets"].as_array().expect("rich snippets array");
        assert_eq!(snippets.len(), 1, "expected one grouped snippet: {v}");
        assert_eq!(snippets[0]["match_line"], 2);
        assert!(
            snippets[0]["snippet"]
                .as_str()
                .is_some_and(|text| text.contains("fn before()") && text.contains("fn after()"))
        );
        let lines = snippets[0]["lines"]
            .as_array()
            .expect("snippet lines array");
        assert_eq!(lines[0]["kind"], "before");
        assert_eq!(lines[1]["kind"], "match");
        assert_eq!(lines[2]["kind"], "after");
    }

    #[test]
    fn search_content_default_payload_omits_rich_snippets() {
        let (_dir, root) = make_repo(&[("src/lib.rs", "fn target() {}\n")]);
        let args = serde_json::json!({ "query": "target", "exclude_generated": false });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(
            v.get("rich_snippets").is_none(),
            "default payload should stay compact: {v}"
        );
    }

    // -----------------------------------------------------------------------
    // subpath scoping
    // -----------------------------------------------------------------------

    #[test]
    fn search_files_subpath_path_traversal_is_rejected() {
        let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}")]);
        // ".." and absolute paths must be rejected rather than allowed to walk outside repo.
        for bad in &["../", "../../etc", "/etc", "../sibling"] {
            let args = serde_json::json!({ "pattern": "*.rs", "subpath": bad });
            let result = tool_search_files(Some(&args), &root, OutputFormat::Json);
            assert!(
                result.is_err(),
                "subpath '{bad}' should be rejected as traversal attempt"
            );
        }
    }

    #[test]
    fn search_content_subpath_path_traversal_is_rejected() {
        let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}")]);
        for bad in &["../", "../../etc", "/etc"] {
            let args = serde_json::json!({
                "query": "fn",
                "subpath": bad,
                "exclude_generated": false
            });
            let result = tool_search_content(Some(&args), &root, OutputFormat::Json);
            assert!(
                result.is_err(),
                "subpath '{bad}' should be rejected as traversal attempt"
            );
        }
    }

    #[test]
    fn search_files_subpath_limits_to_subdir() {
        let (_dir, root) = make_repo(&[
            ("services/auth/schema.sql", "CREATE TABLE users;"),
            ("services/billing/schema.sql", "CREATE TABLE invoices;"),
            ("root.sql", "SELECT 1;"),
        ]);
        let args = serde_json::json!({ "pattern": "*.sql", "subpath": "services/auth" });
        let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            files.iter().any(|f| f.contains("auth/schema.sql")),
            "expected auth file: {files:?}"
        );
        assert!(
            !files.iter().any(|f| f.contains("billing")),
            "billing should be excluded by subpath: {files:?}"
        );
    }

    #[test]
    fn search_files_exclude_globs_skips_matched() {
        let (_dir, root) = make_repo(&[
            ("generated/schema.sql", "-- auto"),
            ("src/manual.sql", "-- hand"),
        ]);
        let args = serde_json::json!({
            "pattern": "*.sql",
            "exclude_globs": ["generated/**"]
        });
        let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            !files.iter().any(|f| f.contains("generated")),
            "generated leaked: {files:?}"
        );
        assert!(
            files.iter().any(|f| f.ends_with("manual.sql")),
            "manual.sql missing: {files:?}"
        );
    }

    #[test]
    fn search_content_subpath_limits_to_subdir() {
        let (_dir, root) = make_repo(&[
            ("services/auth/main.rs", "fn auth_token() {}"),
            ("services/billing/main.rs", "fn auth_token() {}"),
        ]);
        let args = serde_json::json!({
            "query": "auth_token",
            "subpath": "services/auth",
            "exclude_generated": false
        });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let ms = v["matches"].as_array().unwrap();
        assert!(
            !ms.iter()
                .any(|m| m["file"].as_str().unwrap().contains("billing")),
            "billing should be excluded by subpath: {ms:?}"
        );
    }

    #[test]
    fn search_content_exclude_globs_skips_matched() {
        let (_dir, root) = make_repo(&[
            ("generated/api.rs", "fn do_thing() {}"),
            ("src/lib.rs", "fn do_thing() {}"),
        ]);
        let args = serde_json::json!({
            "query": "do_thing",
            "exclude_globs": ["generated/**"],
            "exclude_generated": false
        });
        let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let ms = v["matches"].as_array().unwrap();
        assert!(
            !ms.iter()
                .any(|m| m["file"].as_str().unwrap().contains("generated")),
            "generated leaked: {ms:?}"
        );
        assert!(
            ms.iter()
                .any(|m| m["file"].as_str().unwrap().ends_with("lib.rs")),
            "lib.rs missing: {ms:?}"
        );
    }

    // -----------------------------------------------------------------------
    // search_templates
    // -----------------------------------------------------------------------

    #[test]
    fn search_templates_finds_html_files() {
        let (_dir, root) = make_repo(&[
            ("templates/index.html", "<html></html>"),
            ("templates/base.html", "<html></html>"),
            ("src/main.rs", "fn main() {}"),
        ]);
        let args = serde_json::json!({ "kind": "html" });
        let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(files.iter().any(|f| f.ends_with("index.html")), "{files:?}");
        assert!(!files.iter().any(|f| f.ends_with("main.rs")), "{files:?}");
        assert_eq!(v["atlas_result_kind"], "template_files");
    }

    #[test]
    fn search_templates_finds_jinja_files() {
        let (_dir, root) = make_repo(&[
            ("templates/email.j2", "Hello {{ name }}"),
            ("templates/layout.jinja2", "{% block %}{% endblock %}"),
            ("src/lib.rs", ""),
        ]);
        let args = serde_json::json!({ "kind": "jinja" });
        let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(files.iter().any(|f| f.ends_with("email.j2")), "{files:?}");
        assert!(
            files.iter().any(|f| f.ends_with("layout.jinja2")),
            "{files:?}"
        );
    }

    #[test]
    fn search_templates_no_results_hint() {
        let (_dir, root) = make_repo(&[("src/main.rs", "fn main() {}")]);
        let args = serde_json::json!({ "kind": "haml" });
        let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(v["result_count"], 0);
        assert!(v["atlas_hint"].is_string(), "expected hint on empty result");
    }

    #[test]
    fn search_templates_default_finds_multiple_kinds() {
        let (_dir, root) = make_repo(&[
            ("a.html", "<html/>"),
            ("b.hbs", "{{> partial}}"),
            ("c.j2", "{{ var }}"),
            ("d.tera", "{% if x %}{% endif %}"),
        ]);
        let args = serde_json::json!({});
        let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(v["result_count"].as_u64().unwrap() >= 4, "{v}");
    }

    #[test]
    fn search_templates_exclude_globs() {
        let (_dir, root) = make_repo(&[
            ("generated/page.html", "<html/>"),
            ("src/index.html", "<html/>"),
        ]);
        let args = serde_json::json!({
            "kind": "html",
            "exclude_globs": ["generated/**"]
        });
        let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            !files.iter().any(|f| f.contains("generated")),
            "generated leaked: {files:?}"
        );
        assert!(
            files.iter().any(|f| f.ends_with("index.html")),
            "index.html missing: {files:?}"
        );
    }

    #[test]
    fn search_templates_gitignore_excluded() {
        let (_dir, root) = make_repo(&[
            (".gitignore", "vendor/\n"),
            ("vendor/base.html", "<html/>"),
            ("src/index.html", "<html/>"),
        ]);
        let args = serde_json::json!({ "kind": "html" });
        let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            !files.iter().any(|f| f.contains("vendor")),
            "vendor leaked: {files:?}"
        );
    }

    // -----------------------------------------------------------------------
    // search_text_assets
    // -----------------------------------------------------------------------

    #[test]
    fn search_text_assets_finds_sql_files() {
        let (_dir, root) = make_repo(&[
            ("migrations/001_init.sql", "CREATE TABLE users;"),
            ("src/main.rs", "fn main() {}"),
        ]);
        let args = serde_json::json!({ "kind": "sql" });
        let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            files.iter().any(|f| f.ends_with("001_init.sql")),
            "{files:?}"
        );
        assert!(!files.iter().any(|f| f.ends_with("main.rs")), "{files:?}");
        assert_eq!(v["atlas_result_kind"], "text_asset_files");
    }

    #[test]
    fn search_text_assets_finds_config_files() {
        let (_dir, root) = make_repo(&[
            ("config/app.toml", "[server]"),
            ("config/db.yaml", "host: localhost"),
            ("src/lib.rs", ""),
        ]);
        let args = serde_json::json!({ "kind": "config" });
        let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(files.iter().any(|f| f.ends_with("app.toml")), "{files:?}");
        assert!(files.iter().any(|f| f.ends_with("db.yaml")), "{files:?}");
    }

    #[test]
    fn search_text_assets_finds_prompt_files() {
        let (_dir, root) = make_repo(&[
            ("prompts/review.md", "Review this code"),
            ("docs/guide.md", "# Guide"),
            ("system.prompt", "You are an assistant"),
        ]);
        let args = serde_json::json!({ "kind": "prompt" });
        let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            files.iter().any(|f| f.ends_with("system.prompt")),
            "system.prompt missing: {files:?}"
        );
        // prompts/*.md should match
        assert!(
            files.iter().any(|f| f.contains("prompts/review.md")),
            "prompts/review.md missing: {files:?}"
        );
        // docs/guide.md should NOT match (not in prompts/ and not a .prompt file)
        assert!(
            !files.iter().any(|f| f.ends_with("guide.md")),
            "guide.md leaked: {files:?}"
        );
    }

    #[test]
    fn search_text_assets_no_results_hint() {
        let (_dir, root) = make_repo(&[("src/main.rs", "fn main() {}")]);
        let args = serde_json::json!({ "kind": "sql" });
        let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(v["result_count"], 0);
        assert!(v["atlas_hint"].is_string(), "expected hint");
    }

    #[test]
    fn search_text_assets_default_finds_multiple_kinds() {
        let (_dir, root) = make_repo(&[
            ("schema.sql", "CREATE TABLE x;"),
            ("config.toml", "[section]"),
            ("deploy.yaml", "service: web"),
        ]);
        let args = serde_json::json!({});
        let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(v["result_count"].as_u64().unwrap() >= 3, "{v}");
    }

    #[test]
    fn search_text_assets_subpath_scoping() {
        let (_dir, root) = make_repo(&[
            ("services/auth/db.sql", "SELECT 1;"),
            ("services/billing/db.sql", "SELECT 2;"),
        ]);
        let args = serde_json::json!({ "kind": "sql", "subpath": "services/auth" });
        let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            files.iter().any(|f| f.contains("auth/db.sql")),
            "auth/db.sql missing: {files:?}"
        );
        assert!(
            !files.iter().any(|f| f.contains("billing")),
            "billing leaked: {files:?}"
        );
    }

    #[test]
    fn search_text_assets_atlasignore_respected() {
        let (_dir, root) = make_repo(&[
            (".atlasignore", "secret.sql\n"),
            ("secret.sql", "DROP TABLE users;"),
            ("public.sql", "SELECT 1;"),
        ]);
        let args = serde_json::json!({ "kind": "sql" });
        let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
        let files: Vec<&str> = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert!(
            !files.iter().any(|f| f.ends_with("secret.sql")),
            "secret.sql leaked: {files:?}"
        );
        assert!(
            files.iter().any(|f| f.ends_with("public.sql")),
            "public.sql missing: {files:?}"
        );
    }
}
