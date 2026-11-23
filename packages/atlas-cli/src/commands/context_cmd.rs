use std::collections::HashMap;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};

use anyhow::{Context, Result};
use atlas_adapters::{
    AdapterHooks, CliAdapter, extract_context_event, extract_decision_event_with_details,
};
use atlas_core::GraphToolRequirement;
use atlas_core::SearchQuery;
use atlas_core::model::{
    ContextIntent, ContextRequest, ContextResult, ContextTarget, SelectionReason,
};
use atlas_impact::analyze as advanced_impact;
use atlas_repo::{DiffTarget, changed_files, find_repo_root};
use atlas_review::{ContextEngine, decision_lookup_query, normalize_qn_kind_tokens, query_parser};
use atlas_search as search;
use atlas_search::semantic as sem;
use atlas_session::{DecisionSearchHit, SessionId, SessionStore};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use crate::cli::{Cli, Command};

use super::{
    change_tag, check_graph_readiness, colorize, db_path, derive_graph_readiness,
    derive_graph_readiness_open_failed, detect_changes_target, load_budget_policy,
    load_token_counter, payload_accounting_text, print_json, query_display_path,
    readiness_overrides, resolve_repo,
};

use super::shell_symbols::render_shell_symbols_output;

fn parse_intent_str(s: &str) -> Option<ContextIntent> {
    match s {
        "symbol" => Some(ContextIntent::Symbol),
        "file" => Some(ContextIntent::File),
        "review" => Some(ContextIntent::Review),
        "impact" => Some(ContextIntent::Impact),
        "usage_lookup" | "usage" => Some(ContextIntent::UsageLookup),
        "refactor_safety" | "refactor" => Some(ContextIntent::RefactorSafety),
        "dead_code_check" | "dead_code" => Some(ContextIntent::DeadCodeCheck),
        "rename_preview" | "rename" => Some(ContextIntent::RenamePreview),
        "dependency_removal" | "deps" => Some(ContextIntent::DependencyRemoval),
        _ => None,
    }
}

fn parse_intent_override(override_str: Option<&str>, default: ContextIntent) -> ContextIntent {
    override_str.and_then(parse_intent_str).unwrap_or(default)
}

/// Extract prior-context file hints from the active CLI (or MCP) session.
///
/// Opens the session store best-effort; returns empty vecs on any failure so
/// callers can proceed without context boosting.  Scans the most recent 20
/// events and collects file paths found in `"files"` arrays inside the event
/// payload JSON (emitted by MCP continuity tools).
fn context_session_hints(repo: &str, frontend: &str) -> (Vec<String>, Vec<String>) {
    let store = match SessionStore::open_in_repo(Path::new(repo)) {
        Ok(s) => s,
        Err(_) => return (vec![], vec![]),
    };
    let session_id = SessionId::derive(repo, "", frontend);
    let events = match store.list_events(&session_id) {
        Ok(e) => e,
        Err(_) => return (vec![], vec![]),
    };

    let mut files: Vec<String> = Vec::new();
    for event in events.iter().rev().take(20) {
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.payload_json) else {
            continue;
        };
        if let Some(arr) = payload.get("files").and_then(|v| v.as_array()) {
            for f in arr {
                if let Some(s) = f.as_str() {
                    let owned = s.to_owned();
                    if !files.contains(&owned) {
                        files.push(owned);
                    }
                }
            }
        }
    }

    (files, vec![])
}

fn search_linked_decisions(repo: &str, request: &ContextRequest) -> Vec<DecisionSearchHit> {
    let Some(query) = decision_lookup_query(request) else {
        return Vec::new();
    };
    let store = match SessionStore::open_in_repo(Path::new(repo)) {
        Ok(store) => store,
        Err(_) => return Vec::new(),
    };
    let session_id = SessionId::derive(repo, "", "cli");
    let current = store
        .search_decisions(repo, &query, Some(session_id.as_str()), 5)
        .unwrap_or_default();
    if !current.is_empty() {
        return current;
    }
    store
        .search_decisions(repo, &query, None, 5)
        .unwrap_or_default()
}

fn format_context_output(result: &ContextResult) -> String {
    format_context_output_with_decisions(result, &[])
}

fn format_context_output_with_decisions(
    result: &ContextResult,
    linked_decisions: &[DecisionSearchHit],
) -> String {
    use atlas_core::model::SelectionReason;

    if let Some(ambiguity) = &result.ambiguity {
        let mut out = Vec::new();
        out.push(colorize(
            &format!("Ambiguous target: {}", ambiguity.query),
            "1;33",
        ));
        out.push(format!("Candidates ({}):", ambiguity.candidates.len()));
        out.extend(
            ambiguity
                .candidates
                .iter()
                .map(|candidate| format!("  {candidate}")),
        );
        out.push("Use qualified name or --file to narrow target.".to_string());
        return out.join("\n");
    }

    if result.nodes.is_empty() && result.files.is_empty() {
        return "No context found. Check target name or path.".to_string();
    }

    let mut out = Vec::new();
    if let Some(workflow) = &result.workflow {
        if let Some(headline) = &workflow.headline {
            out.push(colorize(headline, "1;36"));
        }
        if !workflow.high_impact_nodes.is_empty() {
            out.push(colorize("High-impact nodes:", "1;35"));
            out.extend(workflow.high_impact_nodes.iter().map(|node| {
                format!(
                    "  [{:.1}] {} {} ({})",
                    node.relevance_score, node.kind, node.qualified_name, node.file_path
                )
            }));
        }
        if !workflow.call_chains.is_empty() {
            out.push(colorize("Call chains:", "1;35"));
            out.extend(
                workflow
                    .call_chains
                    .iter()
                    .take(5)
                    .map(|chain| format!("  {}", chain.summary)),
            );
        }
        if !workflow.ripple_effects.is_empty() {
            out.push(colorize("Ripple effects:", "1;35"));
            out.extend(
                workflow
                    .ripple_effects
                    .iter()
                    .map(|ripple| format!("  {ripple}")),
            );
        }
    }

    out.push(colorize(
        &format!("Nodes ({}):", result.nodes.len()),
        "1;32",
    ));
    out.extend(result.nodes.iter().take(20).map(|node| {
        format!(
            "  [{:?}] {} {} ({}:{})",
            node.selection_reason,
            node.node.kind.as_str(),
            node.node.qualified_name,
            node.node.file_path,
            node.node.line_start,
        )
    }));

    if !result.edges.is_empty() {
        out.push(colorize(
            &format!("Edges ({}):", result.edges.len()),
            "1;32",
        ));
        out.extend(result.edges.iter().take(20).map(|edge| {
            format!(
                "  {} --{}--> {}",
                edge.edge.source_qn,
                edge.edge.kind.as_str(),
                edge.edge.target_qn,
            )
        }));
    }

    out.push(colorize(
        &format!("Files ({}):", result.files.len()),
        "1;32",
    ));
    out.extend(result.files.iter().map(|file| {
        let ranges: Vec<String> = file
            .line_ranges
            .iter()
            .map(|(start, end)| format!("{start}-{end}"))
            .collect();
        if ranges.is_empty() {
            format!("  {} [{:?}]", file.path, file.selection_reason)
        } else {
            format!(
                "  {} [{:?}] lines {}",
                file.path,
                file.selection_reason,
                ranges.join(", ")
            )
        }
    }));

    if result.truncation.truncated {
        out.push(format!(
            "[truncated: {} nodes, {} edges, {} files dropped]",
            result.truncation.nodes_dropped,
            result.truncation.edges_dropped,
            result.truncation.files_dropped,
        ));
    }
    if let Some(payload) = &result.truncation.payload {
        out.push(payload_accounting_text(payload));
    }

    let direct_count = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .count();
    let caller_count = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::Caller)
        .count();
    let callee_count = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::Callee)
        .count();
    out.push(format!(
        "Summary: {} target, {} callers, {} callees",
        direct_count, caller_count, callee_count
    ));

    if !linked_decisions.is_empty() {
        out.push(colorize(
            &format!("Prior decisions ({}):", linked_decisions.len()),
            "1;33",
        ));
        out.extend(linked_decisions.iter().take(5).map(|hit| {
            let conclusion = hit
                .decision
                .conclusion
                .as_deref()
                .unwrap_or("no conclusion recorded");
            let refs = if hit.decision.source_ids.is_empty() {
                String::new()
            } else {
                format!(" | artifacts: {}", hit.decision.source_ids.join(","))
            };
            format!(
                "  [{:.1}] {} -> {}{}",
                hit.relevance_score, hit.decision.summary, conclusion, refs
            )
        }));
    }

    out.join("\n")
}

fn render_shell_query_output(store: &Store, text: &str, fuzzy: bool) -> Result<String> {
    let query = SearchQuery {
        text: text.to_string(),
        limit: 10,
        fuzzy_match: fuzzy,
        ..SearchQuery::default()
    };
    let results = search::search(store, &query).context("search failed")?;
    if results.is_empty() {
        return Ok("No query results.".to_string());
    }

    let mut lines = vec![colorize("Query results:", "1;36")];
    lines.extend(results.iter().map(|result| {
        format!(
            "  [{:.3}] {} {} ({}:{})",
            result.score,
            result.node.kind.as_str(),
            result.node.qualified_name,
            query_display_path(&result.node),
            result.node.line_start,
        )
    }));
    Ok(lines.join("\n"))
}

fn emit_shell_output(text: &str, paging: bool) -> Result<()> {
    if paging && io::stdout().is_terminal() && text.lines().count() > 24 && try_page_output(text)? {
        return Ok(());
    }
    println!("{text}");
    Ok(())
}

fn try_page_output(text: &str) -> Result<bool> {
    let pager = std::env::var("ATLAS_PAGER")
        .ok()
        .or_else(|| std::env::var("PAGER").ok())
        .unwrap_or_else(|| "less".to_string());
    let mut child = match ProcessCommand::new(&pager)
        .arg("-R")
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Ok(false),
    };
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .with_context(|| format!("write pager input to {pager}"))?;
    }
    child
        .wait()
        .with_context(|| format!("wait for pager {pager}"))?;
    Ok(true)
}

mod run;
mod shell;

pub use self::run::run_context;
pub(crate) use self::shell::ShellArgs;
pub use self::shell::run_shell;
