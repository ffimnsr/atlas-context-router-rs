//! `/symbols` shell command: paginated symbol listing over the open graph.
//!
//! Reuses `Store::list_nodes` so the shell sees the same deterministic
//! ordering, filters, and totals as `atlas symbols` and MCP `list_symbols`.

use anyhow::{Context, Result};
use atlas_core::kinds::normalize_kind_alias;
use atlas_repo::normalize_repo_file_path;
use atlas_store_sqlite::{NodeListFilter, Store};
use camino::Utf8Path;

use super::colorize;
use super::context_cmd::ShellArgs;

pub(crate) const SHELL_SYMBOLS_DEFAULT_LIMIT: usize = 20;
const SHELL_SYMBOLS_MAX_LIMIT: usize = 500;

fn push_filter(hint: &mut String, name: &str, value: &Option<String>) {
    if let Some(value) = value {
        hint.push_str(&format!(" --{name} {value}"));
    }
}

pub(crate) fn render_shell_symbols_output(
    store: &Store,
    repo: &str,
    args: &ShellArgs,
) -> Result<String> {
    let kind = args.flag_val("kind").map(normalize_kind_alias);
    let language = args.flag_val("language").map(str::to_owned);
    // Same best-effort normalization as `atlas symbols`: root-prefixed and
    // absolute-under-root forms collapse to canonical repo-relative prefixes.
    let subpath = args
        .flag_val("subpath")
        .map(|raw| {
            normalize_repo_file_path(Utf8Path::new(repo), raw)
                .map(|resolved| resolved.canonical)
                .unwrap_or_else(|_| raw.to_owned())
        })
        .filter(|prefix| !prefix.trim().is_empty());
    let repo_id = args.flag_val("repo-id").map(str::to_owned);
    let limit = args
        .flag_usize("limit", SHELL_SYMBOLS_DEFAULT_LIMIT)
        .clamp(1, SHELL_SYMBOLS_MAX_LIMIT);
    let offset = args.flag_usize("offset", 0);

    let filter = NodeListFilter {
        kind: kind.as_deref(),
        language: language.as_deref(),
        subpath: subpath.as_deref(),
        repo_id: repo_id.as_deref(),
    };
    let page = store
        .list_nodes(&filter, limit, offset)
        .context("symbol listing failed")?;

    if page.nodes.is_empty() {
        return Ok("No symbols matched.".to_owned());
    }

    let mut lines = vec![format!(
        "{} total {}  showing {} at offset {}",
        colorize("Symbols:", "1;36"),
        page.total,
        page.nodes.len(),
        page.offset
    )];
    for node in &page.nodes {
        lines.push(format!(
            "  {}  {} {}:{}-{}",
            node.qualified_name,
            node.kind.as_str(),
            node.file_path,
            node.line_start,
            node.line_end
        ));
    }
    if let Some(next) = page.next_offset() {
        let mut hint = format!("next: /symbols --offset {next} --limit {limit}");
        push_filter(&mut hint, "kind", &kind);
        push_filter(&mut hint, "language", &language);
        push_filter(&mut hint, "subpath", &subpath);
        push_filter(&mut hint, "repo-id", &repo_id);
        lines.push(hint);
    }
    Ok(lines.join("\n"))
}
