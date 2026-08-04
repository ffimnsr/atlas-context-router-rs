//! Base `tools/list` registry entries for the `content` tool family.
//!
//! Assembled by `super::base_tool_list_json()`; entry order is
//! irrelevant because descriptors are sorted by name afterwards.

use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::{Value, json};

/// Base registry entry JSON for the content tools.
pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
                "name": "search_files",
                "description": "Discover files by name or path glob. Use as a graph/content companion lookup for non-code assets — docs, config, SQL, Markdown, templates — after graph tools have surfaced structural context. Empty or omitted `subpath` means repo-root scope. Do not use before graph resolution for symbol questions. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "pattern":        { "type": "string",  "description": "Glob pattern matched against file names and repo-relative paths (e.g. '*.sql', '**/*.toml', 'config/*')." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters: only files whose repo-relative path matches at least one of these globs are considered." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters: files matching any of these globs are skipped (e.g. ['**/generated/**', '**/*.min.js'])." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory (e.g. 'packages/api'). Empty or omitted value means repo root." },
                        "case_sensitive": { "type": "boolean", "description": "Match pattern case-sensitively (default false)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["pattern"]
                }
        }),
        json!({
                "name": "search_content",
                "description": "Search file contents by literal string or regex. Use as a graph/content companion lookup when changed symbols depend on non-code text — config keys, SQL queries, prompt content, error messages, comments. Generated and vendored files are excluded by default. Empty or omitted `subpath` means repo-root scope. Do not use before graph resolution for symbol questions; use as companion after graph tools surface relevant context. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":              { "type": "string",  "description": "Text to search for. Literal string by default; set is_regex=true for regex patterns. Invalid regex stays strict and returns an error; for literal metacharacters prefer is_regex=false or escape them, e.g. 'Command::Context|Context \\{' ." },
                        "globs":              { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters: only files matching at least one glob are searched." },
                        "exclude_globs":      { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters: files matching any of these globs are skipped." },
                        "exclude_generated":  { "type": "boolean", "description": "Skip generated/vendor files (node_modules, dist, *.min.js, etc.). Default true." },
                        "is_regex":           { "type": "boolean", "description": "Treat query as a regex pattern (default false). Literal queries are case-insensitive by default. Invalid regex does not fall back to literal search." },
                        "context_lines":      { "type": "integer", "description": "Lines of context to include before and after each match (default 0)." },
                        "rich_snippets":      { "type": "boolean", "description": "When true, also return grouped per-match snippets with before/match/after context lines. Default false to keep payloads compact." },
                        "snippet_context_lines": { "type": "integer", "description": "Context lines per grouped rich snippet (default: max(context_lines, 2) when rich_snippets=true)." },
                        "max_results":        { "type": "integer", "description": "Maximum match lines to return (default 50)." },
                        "subpath":            { "type": "string",  "description": "Scope the walk to a repo sub-directory (e.g. 'services/auth'). Empty or omitted value means repo root." },
                        "output_format":      { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
        }),
        json!({
                "name": "read_file_excerpt",
                "description": "Read bounded file content from a repo-relative path using `selector={ kind: 'range'|'ranges'|'context', ... }`. Use this when you already know the file path and need precise excerpts instead of content search.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string", "description": "Repo-relative file path to read." },
                        "selector": {
                            "type": "object",
                            "description": "Selector object. Use { kind: 'range', start_line: 10, end_line: 20 }, { kind: 'ranges', line_ranges: [{ start_line: 10, end_line: 20 }] }, or { kind: 'context', line: 42, before: 2, after: 2 }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Selector kind: range, ranges, or context." },
                                "start_line": { "type": "integer", "description": "Required when kind='range'. 1-based inclusive start line." },
                                "end_line": { "type": "integer", "description": "Required when kind='range'. 1-based inclusive end line." },
                                "line": { "type": "integer", "description": "Required when kind='context'. 1-based line number." },
                                "before": { "type": "integer", "description": "Optional when kind='context'. Context lines before line." },
                                "after": { "type": "integer", "description": "Optional when kind='context'. Context lines after line." },
                                "line_ranges": {
                                    "type": "array",
                                    "description": "Required when kind='ranges'. Non-empty list of line ranges.",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "start_line": { "type": "integer", "description": "1-based inclusive start line." },
                                            "end_line": { "type": "integer", "description": "1-based inclusive end line." }
                                        },
                                        "required": ["start_line", "end_line"]
                                    }
                                }
                            },
                            "required": ["kind"]
                        },
                        "max_lines": { "type": "integer", "description": "Maximum excerpt lines to return across all ranges (default 200, clamped by policy)." },
                        "repo_root": { "type": "string", "description": "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["file"]
                }
        }),
        json!({
                "name": "get_docs_section",
                "description": "Resolve a Markdown section from a repo-relative documentation file using `selector={ kind: 'heading'|'line', ... }`. Returns the section excerpt with heading metadata and file hash.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string", "description": "Repo-relative Markdown file path to read." },
                        "selector": {
                            "type": "object",
                            "description": "Selector object. Use { kind: 'heading', heading: 'document.install' } or { kind: 'line', line: 42 }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Selector kind: heading or line." },
                                "heading": { "type": "string", "description": "Required when kind='heading'. Heading path, slug, or title to resolve." },
                                "line": { "type": "integer", "description": "Required when kind='line'. 1-based line number." }
                            },
                            "required": ["kind"]
                        },
                        "max_bytes": { "type": "integer", "description": "Maximum bytes of section content to emit before truncating (default 16384)." },
                        "repo_root": { "type": "string", "description": "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["file"]
                }
        }),
        json!({
                "name": "read_file_around_match",
                "description": "Read grouped snippets around literal or regex matches inside one repo-relative file. Use this when the file path is known and you need nearby context around matched lines.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string", "description": "Repo-relative file path to search." },
                        "query": { "type": "string", "description": "Literal string or regex pattern to match within the file." },
                        "is_regex": { "type": "boolean", "description": "Treat query as regex (default false)." },
                        "case_sensitive": { "type": "boolean", "description": "When false, literal matching is case-insensitive by default; regex matching is case-sensitive by default." },
                        "before": { "type": "integer", "description": "Context lines before each match window (default 2)." },
                        "after": { "type": "integer", "description": "Context lines after each match window (default 2)." },
                        "max_matches": { "type": "integer", "description": "Maximum matched lines to consider before truncating (default 20, clamped by policy)." },
                        "max_lines": { "type": "integer", "description": "Maximum lines to emit across returned snippets (default 200, clamped by policy)." },
                        "repo_root": { "type": "string", "description": "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["file", "query"]
                }
        }),
        json!({
                "name": "search_templates",
                "description": "Discover template files (HTML, Jinja2, Handlebars, Tera, Mako, Mustache, Twig, Liquid, ERB, HAML, Pug) by extension. Use as a graph/content companion lookup when changed files or graph evidence suggests a dependency on template behavior. Empty or omitted `subpath` means repo-root scope. Narrows by `kind` when you know the template engine. Prefer this over search_files for template-specific discovery. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind":           { "type": "string",  "description": "Template engine: html, jinja, handlebars, tera, mako, mustache, twig, liquid, erb, haml, pug. Omit to search all template types." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory. Empty or omitted value means repo root." },
                        "case_sensitive": { "type": "boolean", "description": "Match case-sensitively (default false)." },
                        "max_results":    { "type": "integer", "description": "Maximum files to return (default 100)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "search_text_assets",
                "description": "Discover SQL, config (TOML/YAML/INI), environment (.env), and prompt files. Use as a graph/content companion lookup when changed files include SQL, config, or prompt assets, or when graph evidence suggests a non-code dependency. Empty or omitted `subpath` means repo-root scope. Use `kind` to narrow to a specific asset type. These files are not indexed as graph symbols; use query_graph for symbol/relationship questions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind":           { "type": "string",  "description": "Asset type: sql, config, env, prompt. Omit to search all text asset types." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory. Empty or omitted value means repo root." },
                        "case_sensitive": { "type": "boolean", "description": "Match case-sensitively (default false)." },
                        "max_results":    { "type": "integer", "description": "Maximum files to return (default 100)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
    ]
}
