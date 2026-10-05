//! `list_symbols` MCP tool: deterministic, paginated symbol inventory.
//!
//! Backed by `Store::list_nodes` so ordering, filters, and totals come from
//! one stable SQL pass. Responses trim to the configured MCP response-byte
//! budget by dropping trailing symbols and reporting `next_offset`, which
//! keeps every page safe to consume and resumable. Multi-repo databases list
//! every indexed repo; each symbol carries `repo_id` when the graph records it.

use anyhow::{Context, Result};
use atlas_core::BudgetReport;
use atlas_store_sqlite::NodeListFilter;
use serde::Serialize;

use crate::output::OutputFormat;
use crate::tool_result::normalized_tool_result_value;

use super::shared::{
    inject_budget_metadata, load_budget_policy, open_store, resolve_kind_alias, str_arg, u64_arg,
};

/// Default page size when `limit` is omitted.
const DEFAULT_LIST_LIMIT: usize = 100;
/// Hard page-size ceiling; larger requests clamp and report the applied limit.
const MAX_LIST_LIMIT: usize = 500;
/// Reserve for response wrapper metadata (provenance, freshness, budget keys)
/// that is added after the structured payload is measured.
const RESPONSE_WRAPPER_RESERVE_BYTES: usize = 2048;
const LIST_BUDGET_NAME: &str = "mcp_cli_payload_serialization.max_mcp_response_bytes";

#[derive(Serialize)]
struct ListedSymbol<'a> {
    qualified_name: &'a str,
    name: &'a str,
    kind: &'a str,
    language: &'a str,
    file: &'a str,
    line_start: u32,
    line_end: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    return_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo_id: Option<&'a str>,
    is_test: bool,
}

fn node_repo_id(node: &atlas_core::Node) -> Option<&str> {
    node.extra_json
        .as_object()
        .and_then(|extra| extra.get("repo_id"))
        .and_then(serde_json::Value::as_str)
}

fn listed_symbol(node: &atlas_core::Node) -> ListedSymbol<'_> {
    ListedSymbol {
        qualified_name: &node.qualified_name,
        name: &node.name,
        kind: node.kind.as_str(),
        language: &node.language,
        file: &node.file_path,
        line_start: node.line_start,
        line_end: node.line_end,
        parent: node.parent_name.as_deref(),
        signature: node.params.as_deref(),
        return_type: node.return_type.as_deref(),
        repo_id: node_repo_id(node),
        is_test: node.is_test,
    }
}

#[allow(clippy::too_many_arguments)]
fn list_symbols_payload(
    symbols: &[ListedSymbol<'_>],
    total: u64,
    limit: usize,
    offset: usize,
    kind: Option<&str>,
    language: Option<&str>,
    subpath: Option<&str>,
    repo_id: Option<&str>,
    truncated: bool,
) -> serde_json::Value {
    let returned = symbols.len();
    let next_offset = if truncated {
        Some(offset.saturating_add(returned))
    } else {
        let consumed = offset.saturating_add(returned);
        if (consumed as u64) < total {
            Some(consumed)
        } else {
            None
        }
    };
    serde_json::json!({
        "tool": "list_symbols",
        "symbols": symbols,
        "total": total,
        "limit": limit,
        "offset": offset,
        "returned": returned,
        "has_more": next_offset.is_some(),
        "next_offset": next_offset,
        "filters": {
            "kind": kind,
            "language": language,
            "subpath": subpath,
            "repo_id": repo_id,
        },
        "truncated": truncated,
        "warnings": [],
    })
}

pub(super) fn tool_list_symbols(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    // Best-effort subpath normalization: root-prefixed and absolute-under-root
    // forms collapse to canonical repo-relative prefixes; unmatched inputs stay
    // as typed so the prefix filter can still match literally.
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let subpath = subpath
        .map(|raw| {
            atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), &raw)
                .map(|resolved| resolved.canonical)
                .unwrap_or(raw)
        })
        .filter(|prefix| !prefix.trim().is_empty());
    let requested_kind = str_arg(args, "kind")?.map(resolve_kind_alias);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let repo_id = str_arg(args, "repo_id")?.map(str::to_owned);
    let requested_limit = u64_arg(args, "limit").unwrap_or(DEFAULT_LIST_LIMIT as u64) as usize;
    let limit = requested_limit.clamp(1, MAX_LIST_LIMIT);
    let offset = u64_arg(args, "offset").unwrap_or(0) as usize;

    let store = open_store(db_path)?;
    let filter = NodeListFilter {
        kind: requested_kind.as_deref(),
        language: language.as_deref(),
        subpath: subpath.as_deref(),
        repo_id: repo_id.as_deref(),
    };
    let page = store
        .list_nodes(&filter, limit, offset)
        .context("symbol listing failed")?;

    let mut symbols = page.nodes.iter().map(listed_symbol).collect::<Vec<_>>();
    let mut truncated = false;

    let policy = load_budget_policy(repo_root)?;
    let max_bytes = policy
        .mcp_cli_payload_serialization
        .mcp_response_bytes
        .default_limit as usize;
    let byte_budget = max_bytes.saturating_sub(RESPONSE_WRAPPER_RESERVE_BYTES);

    let payload = loop {
        let candidate = list_symbols_payload(
            &symbols,
            page.total,
            limit,
            offset,
            requested_kind.as_deref(),
            language.as_deref(),
            subpath.as_deref(),
            repo_id.as_deref(),
            truncated,
        );
        let rendered = serde_json::to_vec(&candidate)?.len();
        if rendered <= byte_budget {
            break candidate;
        }
        if symbols.is_empty() {
            anyhow::bail!(
                "list_symbols response exceeds {LIST_BUDGET_NAME} even after trimming; \
                 raise mcp.max_mcp_response_bytes or lower the page size"
            );
        }
        symbols.pop();
        truncated = true;
    };
    let emitted_bytes = serde_json::to_vec(&payload)?.len();

    let budget = if truncated {
        BudgetReport::partial_result(LIST_BUDGET_NAME, byte_budget, emitted_bytes, true)
    } else {
        BudgetReport::within_budget(LIST_BUDGET_NAME, byte_budget, emitted_bytes)
    };

    let mut response = normalized_tool_result_value(&payload, output_format)?;
    inject_budget_metadata(&mut response, &budget);
    Ok(response)
}
