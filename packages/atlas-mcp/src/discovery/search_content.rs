//! MCP tool: `search_content` — content search by literal string or regex.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use atlas_core::BudgetManager;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use regex::Regex;
use serde::Serialize;

use crate::output::OutputFormat;

use super::shared::{
    bool_arg, build_globset, discovery_tool_error_result, inject_budget_metadata,
    load_budget_policy, normalized_optional_subpath, render_tool_result, resolve_subpath_walk_root,
    str_arg, string_array_arg, u64_arg,
};

// ---------------------------------------------------------------------------
// Generated / vendor patterns excluded by default
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
// Types
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

// ---------------------------------------------------------------------------
// Single-file search helper
// ---------------------------------------------------------------------------

/// Run grep-searcher against a single file and collect up to `max` hits.
fn search_file(
    path: &PathBuf,
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

// ---------------------------------------------------------------------------
// Rich snippet collector
// ---------------------------------------------------------------------------

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
// tool_search_content
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
    (|| {
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
        let subpath = normalized_optional_subpath(str_arg(args, "subpath")?);

        let pattern = if is_regex {
            query.clone()
        } else {
            regex::escape(&query)
        };

        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(!is_regex)
            .build(&pattern)
            .map_err(|error| super::shared::invalid_search_content_regex_error(&query, error))?;

        let rich_snippet_regex = if rich_snippets {
            Some(
                regex::RegexBuilder::new(&pattern)
                    .case_insensitive(!is_regex)
                    .build()
                    .map_err(|error| {
                        super::shared::invalid_search_content_regex_error(&query, error)
                    })?,
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
                        let remaining_snippets =
                            max_results.saturating_sub(rich_snippet_results.len());
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
                Err(_) => continue,
            }
        }

        let result_count = matches.len();
        let looks_like_symbol = !query.contains(' ')
            && !query.contains('.')
            && query.chars().all(|c| c.is_alphanumeric() || c == '_');

        let warnings = if result_count == 0 {
            vec![format!(
                "No matches for '{query}'. Try broadening query, enabling regex mode, or checking file filters."
            )]
        } else if looks_like_symbol {
            vec![format!(
                "'{query}' looks like symbol name. For callers, callees, and graph context prefer query_graph or symbol_neighbors."
            )]
        } else {
            Vec::new()
        };

        #[derive(Serialize)]
        struct SearchContentHit {
            line: u64,
            text: String,
            kind: String,
        }

        #[derive(Serialize)]
        struct SearchContentSnippetLine {
            line: u64,
            text: String,
            kind: String,
        }

        #[derive(Serialize)]
        struct SearchContentSnippet {
            match_line: u64,
            snippet: String,
            lines: Vec<SearchContentSnippetLine>,
        }

        #[derive(Serialize)]
        struct SearchContentMatchGroup {
            file: String,
            hits: Vec<SearchContentHit>,
            snippets: Vec<SearchContentSnippet>,
        }

        #[derive(Serialize)]
        struct SearchContentQuery {
            query: String,
            globs: Vec<String>,
            exclude_globs: Vec<String>,
            exclude_generated: bool,
            context_lines: usize,
            rich_snippets: bool,
            snippet_context_lines: usize,
            case_sensitive: bool,
        }

        #[derive(Serialize)]
        struct SearchContentSummary {
            match_group_count: usize,
            hit_count: usize,
            result_limit: usize,
            scope: &'static str,
            has_matches: bool,
        }

        #[derive(Serialize)]
        struct SearchContentResult {
            tool: &'static str,
            query: SearchContentQuery,
            mode: &'static str,
            subpath: Option<String>,
            matches: Vec<SearchContentMatchGroup>,
            summary: SearchContentSummary,
            truncated: bool,
            warnings: Vec<String>,
        }

        let mut grouped_hits: BTreeMap<String, Vec<SearchContentHit>> = BTreeMap::new();
        for hit in matches {
            grouped_hits
                .entry(hit.file)
                .or_default()
                .push(SearchContentHit {
                    line: hit.line,
                    text: hit.text,
                    kind: hit.kind.unwrap_or("match").to_owned(),
                });
        }

        let mut grouped_snippets: BTreeMap<String, Vec<SearchContentSnippet>> = BTreeMap::new();
        for snippet in rich_snippet_results {
            grouped_snippets
                .entry(snippet.file)
                .or_default()
                .push(SearchContentSnippet {
                    match_line: snippet.match_line,
                    snippet: snippet.snippet,
                    lines: snippet
                        .lines
                        .into_iter()
                        .map(|line| SearchContentSnippetLine {
                            line: line.line,
                            text: line.text,
                            kind: line.kind.to_owned(),
                        })
                        .collect(),
                });
        }

        let mut grouped_matches = grouped_hits
            .into_iter()
            .map(|(file, hits)| SearchContentMatchGroup {
                snippets: grouped_snippets.remove(&file).unwrap_or_default(),
                file,
                hits,
            })
            .collect::<Vec<_>>();
        grouped_matches.sort_unstable_by(|left, right| left.file.cmp(&right.file));

        let result = SearchContentResult {
            tool: "search_content",
            query: SearchContentQuery {
                query: query.clone(),
                globs,
                exclude_globs,
                exclude_generated,
                context_lines,
                rich_snippets,
                snippet_context_lines,
                case_sensitive: is_regex,
            },
            mode: if is_regex { "regex" } else { "literal" },
            subpath,
            summary: SearchContentSummary {
                match_group_count: grouped_matches.len(),
                hit_count: result_count,
                result_limit: max_results,
                scope: if walk_root == repo_root { "repo_root" } else { "subpath" },
                has_matches: result_count > 0,
            },
            matches: grouped_matches,
            truncated,
            warnings,
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
    })()
    .or_else(|error| discovery_tool_error_result("search_content", output_format, error))
}
