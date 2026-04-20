use std::io::{self, BufRead, IsTerminal, Write};
use std::process::{Command as ProcessCommand, Stdio};

use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter, extract_context_event};
use atlas_core::SearchQuery;
use atlas_core::model::{ContextIntent, ContextRequest, ContextResult, ContextTarget};
use atlas_review::{ContextEngine, query_parser};
use atlas_search as search;
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command};

use super::{colorize, db_path, print_json, query_display_path, resolve_repo};

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
    use atlas_session::{SessionId, SessionStore};
    use std::path::Path;

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

fn format_context_output(result: &ContextResult) -> String {
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

pub fn run_context(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("context");
    }
    let db_path = db_path(cli, &repo);

    let result = (|| -> Result<()> {
        let (
            query,
            file,
            files,
            intent_override,
            max_nodes,
            max_edges,
            max_files,
            depth,
            code_spans,
            tests,
            imports,
            neighbors,
            semantic,
        ) = match &cli.command {
            Command::Context {
                query,
                file,
                files,
                intent,
                max_nodes,
                max_edges,
                max_files,
                depth,
                code_spans,
                tests,
                imports,
                neighbors,
                semantic,
            } => (
                query.clone(),
                file.clone(),
                files.clone(),
                intent.clone(),
                *max_nodes,
                *max_edges,
                *max_files,
                *depth,
                *code_spans,
                *tests,
                *imports,
                *neighbors,
                *semantic,
            ),
            _ => unreachable!(),
        };

        // Build the base request: parse from free-text query or structured flags.
        let mut request = if !files.is_empty() {
            // Explicit changed-file list → review/impact.
            let intent = parse_intent_override(intent_override.as_deref(), ContextIntent::Review);
            ContextRequest {
                intent,
                target: ContextTarget::ChangedFiles { paths: files },
                ..ContextRequest::default()
            }
        } else if let Some(path) = file {
            // Explicit file target.
            let intent = parse_intent_override(intent_override.as_deref(), ContextIntent::File);
            ContextRequest {
                intent,
                target: ContextTarget::FilePath { path },
                ..ContextRequest::default()
            }
        } else if let Some(q) = query {
            // Free-text or symbol/qualified name — route through query parser.
            let mut parsed = query_parser::parse_query(&q);
            if let Some(intent) = intent_override.as_deref().and_then(parse_intent_str) {
                parsed.intent = intent;
            }
            parsed
        } else {
            anyhow::bail!("provide a TARGET query, --file <path>, or --files <paths...>");
        };

        // Apply explicit limit/depth overrides.
        if max_nodes.is_some() {
            request.max_nodes = max_nodes;
        }
        if max_edges.is_some() {
            request.max_edges = max_edges;
        }
        if max_files.is_some() {
            request.max_files = max_files;
        }
        if depth.is_some() {
            request.depth = depth;
        }
        if code_spans {
            request.include_code_spans = true;
        }
        if tests {
            request.include_tests = true;
        }
        if imports {
            request.include_imports = true;
        }
        if neighbors {
            request.include_neighbors = true;
        }

        let store =
            Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

        // --semantic: when the target is a free-text / symbol query, run a
        // graph-aware semantic search first to find the best matching qualified
        // name.  If a session is active, prior-context files from recent
        // events are used to boost relevance (context_boosted_search).
        // The context engine then builds around the resolved qualified name
        // instead of doing a fuzzier name lookup internally.
        if semantic && let ContextTarget::SymbolName { ref name } = request.target {
            let sq = SearchQuery {
                text: name.clone(),
                limit: 5,
                graph_expand: true,
                graph_max_hops: 1,
                ..Default::default()
            };
            let (ctx_files, ctx_symbols) = context_session_hints(&repo, "cli");
            let hits =
                search::semantic::context_boosted_search(&store, &sq, &ctx_files, &ctx_symbols)
                    .unwrap_or_default();
            if let Some(top) = hits.into_iter().next() {
                request.target = ContextTarget::QualifiedName {
                    qname: top.node.qualified_name,
                };
            }
        }

        let engine = ContextEngine::new(&store);
        let result = engine.build(&request).context("context engine failed")?;

        if cli.json {
            print_json("context", serde_json::to_value(&result)?)?;
        } else {
            println!("{}", format_context_output(&result));
        }

        Ok(())
    })();

    if result.is_ok()
        && let Some(ref mut a) = adapter
    {
        a.record(extract_context_event("cli:context", 0));
    }
    if let Some(ref mut a) = adapter {
        a.after_command("context", result.is_ok());
    }
    result
}

pub fn run_shell(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let (fuzzy, paging) = match &cli.command {
        Command::Shell { fuzzy, paging } => (*fuzzy, *paging),
        _ => unreachable!(),
    };

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let engine = ContextEngine::new(&store);
    let stdin = io::stdin();
    let mut line = String::new();

    emit_shell_output(
        &format!(
            "{}\n{}\n{}",
            colorize("Atlas shell", "1;36"),
            "Type natural-language graph questions or `/query <text>`.",
            "Use `help`, `exit`, or `quit` to leave."
        ),
        paging,
    )?;

    loop {
        if io::stdout().is_terminal() {
            print!("{}", colorize("atlas> ", "1;34"));
            io::stdout().flush().context("flush shell prompt")?;
        }

        line.clear();
        if stdin
            .lock()
            .read_line(&mut line)
            .context("read shell line")?
            == 0
        {
            break;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if matches!(input, "exit" | "quit") {
            break;
        }
        if input == "help" {
            emit_shell_output(
                "Examples:\n  where is greet_twice used\n  what calls helper\n  what breaks if I change helper\n  /query greter",
                paging,
            )?;
            continue;
        }

        let rendered = if let Some(query_text) = input.strip_prefix("/query ") {
            render_shell_query_output(&store, query_text.trim(), fuzzy)?
        } else {
            let request = query_parser::parse_query(input);
            let result = engine.build(&request).context("context engine failed")?;
            format_context_output(&result)
        };

        emit_shell_output(&rendered, paging)?;
    }

    Ok(())
}
