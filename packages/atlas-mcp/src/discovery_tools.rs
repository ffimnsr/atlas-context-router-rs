//! MCP tools for file-path and content discovery outside the graph-symbol path.
//!
//! These tools complement the graph-first workflow:
//! - Use `query_graph` for symbol/relationship questions against indexed code.
//! - Use `search_files` when you need files by name/glob (e.g. config, templates, SQL).
//! - Use `search_content` when you need literal or regex matches in file content.

use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use serde::Serialize;
use std::path::Path;

use crate::output::{OutputFormat, render_serializable};

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

// ---------------------------------------------------------------------------
// search_files
// ---------------------------------------------------------------------------

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
    let pattern = str_arg(args, "pattern")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: pattern"))?
        .to_owned();
    let globs = string_array_arg(args, "globs")?;
    let exclude_globs = string_array_arg(args, "exclude_globs")?;
    let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(false);
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);

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

    let walk_root = if let Some(ref sp) = subpath {
        let candidate = Path::new(repo_root).join(sp);
        if candidate.is_dir() {
            candidate.to_string_lossy().into_owned()
        } else {
            repo_root.to_owned()
        }
    } else {
        repo_root.to_owned()
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

    for entry in walker.build().flatten() {
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
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<&'a str>,
    }

    let result = SearchFilesResult {
        files,
        result_count,
        atlas_result_kind: "file_paths",
        atlas_hint,
    };

    render_tool_result(&result, output_format)
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
    let query = str_arg(args, "query")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: query"))?
        .to_owned();
    let globs = string_array_arg(args, "globs")?;
    let exclude_globs = string_array_arg(args, "exclude_globs")?;
    let exclude_generated = bool_arg(args, "exclude_generated").unwrap_or(true);
    let is_regex = bool_arg(args, "is_regex").unwrap_or(false);
    let context_lines = u64_arg(args, "context_lines").unwrap_or(0) as usize;
    let max_results = u64_arg(args, "max_results").unwrap_or(50) as usize;
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
        .with_context(|| format!("invalid search pattern: {query}"))?;

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

    let walk_root = if let Some(ref sp) = subpath {
        let candidate = Path::new(repo_root).join(sp);
        if candidate.is_dir() {
            candidate.to_string_lossy().into_owned()
        } else {
            repo_root.to_owned()
        }
    } else {
        repo_root.to_owned()
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
        result_count: usize,
        truncated: bool,
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    let result = SearchContentResult {
        matches,
        result_count,
        truncated,
        atlas_result_kind: "content_matches",
        atlas_hint,
    };

    render_tool_result(&result, output_format)
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
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let globs = string_array_arg(args, "globs")?;
    let exclude_globs = string_array_arg(args, "exclude_globs")?;
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(false);
    let max_results = u64_arg(args, "max_results").unwrap_or(100) as usize;

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

    let walk_root = if let Some(ref sp) = subpath {
        let candidate = Path::new(repo_root).join(sp);
        if candidate.is_dir() {
            candidate.to_string_lossy().into_owned()
        } else {
            repo_root.to_owned()
        }
    } else {
        repo_root.to_owned()
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

    for entry in walker.build().flatten() {
        if files.len() >= max_results {
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
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    let result = SearchTemplatesResult {
        files,
        result_count,
        atlas_result_kind: "template_files",
        atlas_hint,
    };

    render_tool_result(&result, output_format)
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
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let globs = string_array_arg(args, "globs")?;
    let exclude_globs = string_array_arg(args, "exclude_globs")?;
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let case_sensitive = bool_arg(args, "case_sensitive").unwrap_or(false);
    let max_results = u64_arg(args, "max_results").unwrap_or(100) as usize;

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

    let walk_root = if let Some(ref sp) = subpath {
        let candidate = Path::new(repo_root).join(sp);
        if candidate.is_dir() {
            candidate.to_string_lossy().into_owned()
        } else {
            repo_root.to_owned()
        }
    } else {
        repo_root.to_owned()
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

    for entry in walker.build().flatten() {
        if files.len() >= max_results {
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
        atlas_result_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    let result = SearchTextAssetsResult {
        files,
        result_count,
        atlas_result_kind: "text_asset_files",
        atlas_hint,
    };

    render_tool_result(&result, output_format)
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

    // -----------------------------------------------------------------------
    // subpath scoping
    // -----------------------------------------------------------------------

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
