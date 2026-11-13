//! CLI commands that mirror companion content and relationship MCP tools.
//!
//! Every command forwards its arguments to the shared `atlas-mcp` tool
//! implementation, so the CLI and MCP surfaces share one argument contract and
//! one payload shape.

use anyhow::{Context, Result};
use atlas_repo::find_repo_root;
use camino::Utf8Path;
use serde_json::{Value, json};

use crate::cli::{
    AssetDiscoveryArgs, Cli, ClustersArgs, Command, ContentCommand, ExcerptArgs, FilesArgs,
    GrepArgs, LinksArgs, MatchContextArgs, NeighborsArgs,
};

use super::{db_path, print_json, resolve_repo};

pub fn run_content(cli: &Cli) -> Result<()> {
    let Command::Content { subcommand } = &cli.command else {
        anyhow::bail!("content command required");
    };
    match subcommand {
        ContentCommand::Files(args) => run_files(cli, args),
        ContentCommand::Grep(args) => run_grep(cli, args),
        ContentCommand::Excerpt(args) => run_excerpt(cli, args),
        ContentCommand::MatchContext(args) => run_match_context(cli, args),
        ContentCommand::Templates(args) => run_templates(cli, args),
        ContentCommand::TextAssets(args) => run_text_assets(cli, args),
        ContentCommand::Neighbors(args) => run_neighbors(cli, args),
        ContentCommand::Links(args) => run_links(cli, args),
        ContentCommand::Clusters(args) => run_clusters(cli, args),
    }
}

fn repo_root(cli: &Cli) -> Result<String> {
    let repo = resolve_repo(cli)?;
    let root = find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    Ok(root.to_string())
}

fn store_path(cli: &Cli) -> Result<(String, String)> {
    let repo = resolve_repo(cli)?;
    let root = find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let db = db_path(cli, &repo);
    Ok((root.to_string(), db))
}

/// Unwrap the structured payload of a tool response, turning `isError` results
/// into command errors so the CLI exit code reflects the failure.
fn tool_payload(response: Value) -> Result<Value> {
    if response.get("isError").and_then(Value::as_bool) == Some(true) {
        let message = response
            .pointer("/structuredContent/message")
            .and_then(Value::as_str)
            .or_else(|| response.pointer("/content/0/text").and_then(Value::as_str))
            .unwrap_or("tool call failed");
        anyhow::bail!("{message}");
    }
    Ok(response
        .get("structuredContent")
        .cloned()
        .unwrap_or(response))
}

fn emit(cli: &Cli, command: &str, data: &Value, render: fn(&Value) -> String) -> Result<()> {
    if cli.json {
        print_json(command, data.clone())
    } else {
        println!("{}", render(data));
        Ok(())
    }
}

fn array_at<'a>(value: &'a Value, pointer: &str) -> &'a [Value] {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn render_paths(data: &Value, pointer: &str) -> String {
    let mut lines: Vec<String> = array_at(data, pointer)
        .iter()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    if lines.is_empty() {
        lines.push("No matches.".to_owned());
    }
    lines.join("\n")
}

// ── `atlas content files` ───────────────────────────────────────────────────

fn run_files(cli: &Cli, args: &FilesArgs) -> Result<()> {
    let root = repo_root(cli)?;
    let mut payload = json!({
        "pattern": args.pattern,
        "globs": args.globs,
        "exclude_globs": args.exclude_globs,
        "case_sensitive": args.case_sensitive,
    });
    if let Some(subpath) = &args.subpath {
        payload["subpath"] = json!(subpath);
    }
    let data = tool_payload(atlas_mcp::cli_tools::search_files(&payload, &root)?)?;
    emit(cli, "files", &data, |data| render_paths(data, "/matches"))
}

// ── `atlas content grep` ────────────────────────────────────────────────────

fn run_grep(cli: &Cli, args: &GrepArgs) -> Result<()> {
    let root = repo_root(cli)?;
    let mut payload = json!({
        "query": args.query,
        "is_regex": args.regex,
        "globs": args.globs,
        "exclude_globs": args.exclude_globs,
        "exclude_generated": !args.include_generated,
        "rich_snippets": args.rich_snippets,
        "exclude_subpaths": args.exclude_subpaths,
    });
    if let Some(context_lines) = args.context_lines {
        payload["context_lines"] = json!(context_lines);
    }
    if let Some(snippet_context_lines) = args.snippet_context_lines {
        payload["snippet_context_lines"] = json!(snippet_context_lines);
    }
    if let Some(max_results) = args.max_results {
        payload["max_results"] = json!(max_results);
    }
    if let Some(subpath) = &args.subpath {
        payload["subpath"] = json!(subpath);
    }
    if let Some(exact_dir) = &args.exact_dir {
        payload["exact_dir"] = json!(exact_dir);
    }
    let data = tool_payload(atlas_mcp::cli_tools::search_content(&payload, &root)?)?;
    emit(cli, "grep", &data, render_grep)
}

fn render_grep(data: &Value) -> String {
    let mut lines = Vec::new();
    for group in array_at(data, "/matches") {
        let file = group
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or_default();
        for hit in array_at(group, "/hits") {
            let line = hit.get("line").and_then(Value::as_u64).unwrap_or_default();
            let text = hit.get("text").and_then(Value::as_str).unwrap_or_default();
            lines.push(format!("{file}:{line}: {text}"));
        }
    }
    if lines.is_empty() {
        lines.push("No matches.".to_owned());
    }
    lines.join("\n")
}

// ── `atlas content excerpt` ─────────────────────────────────────────────────

fn run_excerpt(cli: &Cli, args: &ExcerptArgs) -> Result<()> {
    let root = repo_root(cli)?;
    let selector = excerpt_selector(args)?;
    let mut payload = json!({ "file": args.file, "selector": selector });
    if let Some(max_lines) = args.max_lines {
        payload["max_lines"] = json!(max_lines);
    }
    let data = tool_payload(atlas_mcp::cli_tools::read_file_excerpt(&payload, &root)?)?;
    emit(cli, "excerpt", &data, render_excerpt)
}

fn excerpt_selector(args: &ExcerptArgs) -> Result<Value> {
    if !args.ranges.is_empty() {
        let line_ranges = args
            .ranges
            .iter()
            .map(|raw| {
                let (start, end) = raw
                    .split_once(':')
                    .or_else(|| raw.split_once('-'))
                    .ok_or_else(|| anyhow::anyhow!("range '{raw}' must be START:END"))?;
                let start: u64 = start
                    .trim()
                    .parse()
                    .with_context(|| format!("invalid range start in '{raw}'"))?;
                let end: u64 = end
                    .trim()
                    .parse()
                    .with_context(|| format!("invalid range end in '{raw}'"))?;
                Ok(json!({ "start_line": start, "end_line": end }))
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(json!({ "kind": "ranges", "line_ranges": line_ranges }));
    }
    if let Some(line) = args.line {
        return Ok(json!({
            "kind": "context",
            "line": line,
            "before": args.context,
            "after": args.context,
        }));
    }
    match (args.start_line, args.end_line) {
        (Some(start_line), Some(end_line)) => Ok(json!({
            "kind": "range",
            "start_line": start_line,
            "end_line": end_line,
        })),
        _ => anyhow::bail!("provide --start-line/--end-line, --range, or --line"),
    }
}

fn render_excerpt(data: &Value) -> String {
    let mut lines = Vec::new();
    for snippet in array_at(data, "/snippets") {
        for entry in array_at(snippet, "/lines") {
            let line = entry.get(0).and_then(Value::as_u64).unwrap_or_default();
            let text = entry.get(1).and_then(Value::as_str).unwrap_or_default();
            lines.push(format!("{line}: {text}"));
        }
    }
    if lines.is_empty() {
        lines.push("No lines selected.".to_owned());
    }
    lines.join("\n")
}

// ── `atlas content match-context` ───────────────────────────────────────────

fn run_match_context(cli: &Cli, args: &MatchContextArgs) -> Result<()> {
    let root = repo_root(cli)?;
    let mut payload = json!({
        "file": args.file,
        "query": args.query,
        "is_regex": args.regex,
        "before": args.before,
        "after": args.after,
    });
    if args.case_sensitive {
        payload["case_sensitive"] = json!(true);
    }
    if let Some(max_matches) = args.max_matches {
        payload["max_matches"] = json!(max_matches);
    }
    if let Some(max_lines) = args.max_lines {
        payload["max_lines"] = json!(max_lines);
    }
    let data = tool_payload(atlas_mcp::cli_tools::read_file_around_match(
        &payload, &root,
    )?)?;
    emit(cli, "match_context", &data, render_match_context)
}

fn render_match_context(data: &Value) -> String {
    let mut lines = Vec::new();
    for snippet in array_at(data, "/snippets") {
        for (prefix, pointer) in [("  ", "/before"), ("> ", "/match"), ("  ", "/after")] {
            for entry in array_at(snippet, pointer) {
                let line = entry.get(0).and_then(Value::as_u64).unwrap_or_default();
                let text = entry.get(1).and_then(Value::as_str).unwrap_or_default();
                lines.push(format!("{prefix}{line}: {text}"));
            }
        }
    }
    if lines.is_empty() {
        lines.push("No matches.".to_owned());
    }
    lines.join("\n")
}

// ── `atlas content templates` / `atlas content text-assets` ─────────────────

fn asset_args(args: &AssetDiscoveryArgs) -> Value {
    let mut value = json!({
        "globs": args.globs,
        "exclude_globs": args.exclude_globs,
        "case_sensitive": args.case_sensitive,
    });
    if let Some(kind) = &args.kind {
        value["kind"] = json!(kind);
    }
    if let Some(subpath) = &args.subpath {
        value["subpath"] = json!(subpath);
    }
    if let Some(max_results) = args.max_results {
        value["max_results"] = json!(max_results);
    }
    value
}

fn run_templates(cli: &Cli, args: &AssetDiscoveryArgs) -> Result<()> {
    let root = repo_root(cli)?;
    let data = tool_payload(atlas_mcp::cli_tools::search_templates(
        &asset_args(args),
        &root,
    )?)?;
    emit(cli, "templates", &data, |data| {
        render_paths(data, "/matches")
    })
}

fn run_text_assets(cli: &Cli, args: &AssetDiscoveryArgs) -> Result<()> {
    let root = repo_root(cli)?;
    let data = tool_payload(atlas_mcp::cli_tools::search_text_assets(
        &asset_args(args),
        &root,
    )?)?;
    emit(cli, "text_assets", &data, |data| {
        render_paths(data, "/matches")
    })
}

// ── `atlas content neighbors` ───────────────────────────────────────────────

fn run_neighbors(cli: &Cli, args: &NeighborsArgs) -> Result<()> {
    let (root, db) = store_path(cli)?;
    let mut payload = json!({ "qname": args.qname });
    if let Some(limit) = args.limit {
        payload["limit"] = json!(limit);
    }
    let data = tool_payload(atlas_mcp::cli_tools::symbol_neighbors(
        &payload, &root, &db,
    )?)?;
    emit(cli, "neighbors", &data, render_neighbors)
}

fn render_neighbors(data: &Value) -> String {
    let mut lines = vec![format!(
        "{} (found: {})",
        data.pointer("/symbol/qname")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        data.pointer("/symbol/found")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    )];
    for (label, pointer) in [
        ("callers", "/callers"),
        ("callees", "/callees"),
        ("tests", "/tests"),
        ("siblings", "/siblings"),
        ("imports", "/imports"),
    ] {
        for node in array_at(data, pointer) {
            lines.push(format!(
                "{label}: {} [{}]",
                node.get("qn").and_then(Value::as_str).unwrap_or_default(),
                node.get("kind").and_then(Value::as_str).unwrap_or_default()
            ));
        }
    }
    lines.join("\n")
}

// ── `atlas content links` ───────────────────────────────────────────────────

fn run_links(cli: &Cli, args: &LinksArgs) -> Result<()> {
    let (root, db) = store_path(cli)?;
    let mut payload = json!({ "file": args.file, "include_weak": args.include_weak });
    if let Some(limit) = args.limit {
        payload["limit"] = json!(limit);
    }
    let data = tool_payload(atlas_mcp::cli_tools::cross_file_links(
        &payload, &root, &db,
    )?)?;
    emit(cli, "links", &data, render_links)
}

fn render_links(data: &Value) -> String {
    let mut lines = Vec::new();
    for link in array_at(data, "/linked_files") {
        lines.push(format!(
            "{} (coupling {})",
            link.get("file").and_then(Value::as_str).unwrap_or_default(),
            link.get("coupling_metric")
                .and_then(Value::as_f64)
                .unwrap_or_default()
        ));
    }
    if lines.is_empty() {
        lines.push("No linked files.".to_owned());
    }
    lines.join("\n")
}

// ── `atlas content clusters` ────────────────────────────────────────────────

fn run_clusters(cli: &Cli, args: &ClustersArgs) -> Result<()> {
    let (root, db) = store_path(cli)?;
    let mut payload = json!({ "files": args.files });
    if let Some(limit) = args.limit {
        payload["limit"] = json!(limit);
    }
    let data = tool_payload(atlas_mcp::cli_tools::concept_clusters(
        &payload, &root, &db,
    )?)?;
    emit(cli, "clusters", &data, render_clusters)
}

fn render_clusters(data: &Value) -> String {
    let mut lines = Vec::new();
    for cluster in array_at(data, "/clusters") {
        let files: Vec<&str> = array_at(cluster, "/files")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        lines.push(format!(
            "#{} density {}: {}",
            cluster
                .get("rank")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            cluster
                .get("density")
                .and_then(Value::as_f64)
                .unwrap_or_default(),
            files.join(", ")
        ));
    }
    if lines.is_empty() {
        lines.push("No clusters.".to_owned());
    }
    lines.join("\n")
}
