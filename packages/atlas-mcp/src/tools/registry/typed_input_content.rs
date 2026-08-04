//! Typed input-schema arm for the `content` tool family.
//!
//! Dispatched by `super::typed_input_schema_for()`; schema structs
//! come from `super::schemas`.

use super::*;
use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::Value;

pub(super) fn typed_input_schema_for(name: &str) -> Option<Value> {
    match name {
        "search_files" => Some(typed_schema_with_descriptions::<SearchFilesArgsSchema>(&[
            (
                "properties/pattern",
                "Glob pattern matched against file names and repo-relative paths (e.g. '*.sql', '**/*.toml', 'config/*').",
            ),
            (
                "properties/globs",
                "Optional include-path filters: only files whose repo-relative path matches at least one of these globs are considered.",
            ),
            (
                "properties/exclude_globs",
                "Optional exclusion filters: files matching any of these globs are skipped (e.g. ['**/generated/**', '**/*.min.js']).",
            ),
            (
                "properties/subpath",
                "Scope the walk to a repo sub-directory (e.g. 'packages/api'). Empty or omitted value means repo root.",
            ),
            (
                "properties/case_sensitive",
                "Match pattern case-sensitively (default false).",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "search_content" => Some(typed_schema_with_descriptions::<SearchContentArgsSchema>(
            &[
                (
                    "properties/query",
                    "Text to search for. Literal string by default; set is_regex=true for regex patterns. Invalid regex stays strict and returns an error; for literal metacharacters prefer is_regex=false or escape them, e.g. 'Command::Context|Context \\{' .",
                ),
                (
                    "properties/globs",
                    "Optional include-path filters: only files matching at least one glob are searched.",
                ),
                (
                    "properties/exclude_globs",
                    "Optional exclusion filters: files matching any of these globs are skipped.",
                ),
                (
                    "properties/exclude_generated",
                    "Skip generated/vendor files (node_modules, dist, *.min.js, etc.). Default true.",
                ),
                (
                    "properties/is_regex",
                    "Treat query as a regex pattern (default false). Literal queries are case-insensitive by default. Invalid regex does not fall back to literal search.",
                ),
                (
                    "properties/context_lines",
                    "Lines of context to include before and after each match (default 0).",
                ),
                (
                    "properties/rich_snippets",
                    "When true, also return grouped per-match snippets with before/match/after context lines. Default false to keep payloads compact.",
                ),
                (
                    "properties/snippet_context_lines",
                    "Context lines per grouped rich snippet (default: max(context_lines, 2) when rich_snippets=true).",
                ),
                (
                    "properties/max_results",
                    "Maximum match lines to return (default 50).",
                ),
                (
                    "properties/subpath",
                    "Scope the walk to a repo sub-directory (e.g. 'services/auth'). Empty or omitted value means repo root.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "read_file_excerpt" => Some(typed_schema_with_descriptions::<ReadFileExcerptArgsSchema>(
            &[
                ("properties/file", "Repo-relative file path to read."),
                (
                    "properties/selector",
                    "Selector object. Use { kind: 'range', start_line: 10, end_line: 20 }, { kind: 'ranges', line_ranges: [{ start_line: 10, end_line: 20 }] }, or { kind: 'context', line: 42, before: 2, after: 2 }.",
                ),
                (
                    "properties/selector/properties/kind",
                    "Selector kind: range, ranges, or context.",
                ),
                (
                    "properties/selector/properties/start_line",
                    "Required when kind='range'. 1-based inclusive start line.",
                ),
                (
                    "properties/selector/properties/end_line",
                    "Required when kind='range'. 1-based inclusive end line.",
                ),
                (
                    "properties/selector/properties/line",
                    "Required when kind='context'. 1-based line number.",
                ),
                (
                    "properties/selector/properties/before",
                    "Optional when kind='context'. Context lines before line.",
                ),
                (
                    "properties/selector/properties/after",
                    "Optional when kind='context'. Context lines after line.",
                ),
                (
                    "properties/selector/properties/line_ranges",
                    "Required when kind='ranges'. Non-empty list of line ranges.",
                ),
                (
                    "properties/selector/properties/line_ranges/items/properties/start_line",
                    "1-based inclusive start line.",
                ),
                (
                    "properties/selector/properties/line_ranges/items/properties/end_line",
                    "1-based inclusive end line.",
                ),
                (
                    "properties/max_lines",
                    "Maximum excerpt lines to return across all ranges (default 200, clamped by policy).",
                ),
                (
                    "properties/repo_root",
                    "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "get_docs_section" => Some(typed_schema_with_descriptions::<GetDocsSectionArgsSchema>(
            &[
                (
                    "properties/file",
                    "Repo-relative Markdown file path to read.",
                ),
                (
                    "properties/selector",
                    "Selector object. Use { kind: 'heading', heading: 'document.install' } or { kind: 'line', line: 42 }.",
                ),
                (
                    "properties/selector/properties/kind",
                    "Selector kind: heading or line.",
                ),
                (
                    "properties/selector/properties/heading",
                    "Required when kind='heading'. Heading path, slug, or title to resolve.",
                ),
                (
                    "properties/selector/properties/line",
                    "Required when kind='line'. 1-based line number.",
                ),
                (
                    "properties/max_bytes",
                    "Maximum bytes of section content to emit before truncating (default 16384).",
                ),
                (
                    "properties/repo_root",
                    "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "read_file_around_match" => Some(typed_schema_with_descriptions::<
            ReadFileAroundMatchArgsSchema,
        >(&[
            ("properties/file", "Repo-relative file path to search."),
            (
                "properties/query",
                "Literal string or regex pattern to match within the file.",
            ),
            (
                "properties/is_regex",
                "Treat query as regex (default false).",
            ),
            (
                "properties/case_sensitive",
                "When false, literal matching is case-insensitive by default; regex matching is case-sensitive by default.",
            ),
            (
                "properties/before",
                "Context lines before each match window (default 2).",
            ),
            (
                "properties/after",
                "Context lines after each match window (default 2).",
            ),
            (
                "properties/max_matches",
                "Maximum matched lines to consider before truncating (default 20, clamped by policy).",
            ),
            (
                "properties/max_lines",
                "Maximum lines to emit across returned snippets (default 200, clamped by policy).",
            ),
            (
                "properties/repo_root",
                "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "search_templates" => Some(typed_schema_with_descriptions::<SearchTemplatesArgsSchema>(
            &[
                (
                    "properties/kind",
                    "Template engine: html, jinja, handlebars, tera, mako, mustache, twig, liquid, erb, haml, pug. Omit to search all template types.",
                ),
                ("properties/globs", "Optional include-path filters."),
                ("properties/exclude_globs", "Optional exclusion filters."),
                (
                    "properties/subpath",
                    "Scope the walk to a repo sub-directory. Empty or omitted value means repo root.",
                ),
                (
                    "properties/case_sensitive",
                    "Match case-sensitively (default false).",
                ),
                (
                    "properties/max_results",
                    "Maximum files to return (default 100).",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "search_text_assets" => Some(
            typed_schema_with_descriptions::<SearchTextAssetsArgsSchema>(&[
                (
                    "properties/kind",
                    "Asset type: sql, config, env, prompt. Omit to search all text asset types.",
                ),
                ("properties/globs", "Optional include-path filters."),
                ("properties/exclude_globs", "Optional exclusion filters."),
                (
                    "properties/subpath",
                    "Scope the walk to a repo sub-directory. Empty or omitted value means repo root.",
                ),
                (
                    "properties/case_sensitive",
                    "Match case-sensitively (default false).",
                ),
                (
                    "properties/max_results",
                    "Maximum files to return (default 100).",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]),
        ),
        _ => None,
    }
}
