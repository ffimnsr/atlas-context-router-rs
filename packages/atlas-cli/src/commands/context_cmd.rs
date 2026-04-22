use std::collections::HashMap;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::{Command as ProcessCommand, Stdio};

use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter, extract_context_event};
use atlas_core::SearchQuery;
use atlas_core::model::{ContextIntent, ContextRequest, ContextResult, ContextTarget};
use atlas_impact::analyze as advanced_impact;
use atlas_repo::{DiffTarget, changed_files, find_repo_root, repo_relative};
use atlas_review::{
    ContextEngine, assemble_review_context, normalize_qn_kind_tokens, query_parser,
};
use atlas_search as search;
use atlas_search::semantic as sem;
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use crate::cli::{Cli, Command};

use super::{
    change_tag, colorize, db_path, detect_changes_target, print_json, query_display_path,
    resolve_repo,
};

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

// ---------------------------------------------------------------------------
// Shell mini-argument parser
// ---------------------------------------------------------------------------

/// Parsed shell command arguments.
///
/// Tokens starting with `--` are flags. A flag is a boolean flag when the
/// next token also starts with `--` or there is no next token; otherwise the
/// following token becomes its value. All other tokens are positionals.
struct ShellArgs {
    flags: HashMap<String, Option<String>>,
    positionals: Vec<String>,
}

impl ShellArgs {
    fn parse(input: &str) -> Self {
        let mut flags: HashMap<String, Option<String>> = HashMap::new();
        let mut positionals = Vec::new();
        let tokens: Vec<&str> = input.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            if let Some(flag) = tokens[i].strip_prefix("--") {
                let has_value = tokens.get(i + 1).is_some_and(|t| !t.starts_with("--"));
                if has_value {
                    flags.insert(flag.to_string(), Some(tokens[i + 1].to_string()));
                    i += 2;
                } else {
                    flags.insert(flag.to_string(), None);
                    i += 1;
                }
            } else {
                positionals.push(tokens[i].to_string());
                i += 1;
            }
        }
        Self { flags, positionals }
    }

    fn flag_bool(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    fn flag_val(&self, name: &str) -> Option<&str> {
        self.flags.get(name)?.as_deref()
    }

    fn flag_u32(&self, name: &str, default: u32) -> u32 {
        self.flag_val(name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    fn flag_usize(&self, name: &str, default: usize) -> usize {
        self.flag_val(name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
}

// ---------------------------------------------------------------------------
// Shell render functions
// ---------------------------------------------------------------------------

fn render_shell_stats_output(store: &Store) -> Result<String> {
    let stats = store.stats().context("stats query failed")?;
    let mut lines = vec![colorize("Graph stats:", "1;36")];
    lines.push(format!("  Files        : {}", stats.file_count));
    lines.push(format!("  Nodes        : {}", stats.node_count));
    lines.push(format!("  Edges        : {}", stats.edge_count));
    if !stats.languages.is_empty() {
        lines.push(format!("  Languages    : {}", stats.languages.join(", ")));
    }
    if !stats.nodes_by_kind.is_empty() {
        lines.push(colorize("  By kind:", "1;33"));
        for (kind, count) in &stats.nodes_by_kind {
            lines.push(format!("    {kind:<18} {count}"));
        }
    }
    if let Some(ts) = &stats.last_indexed_at {
        lines.push(format!("  Last indexed : {ts}"));
    }
    Ok(lines.join("\n"))
}

fn resolve_shell_files(repo: &str, args: &ShellArgs) -> Result<Vec<String>> {
    let base = args.flag_val("base").map(str::to_owned);
    let staged = args.flag_bool("staged");

    if !args.positionals.is_empty() {
        let repo_root_path =
            find_repo_root(Utf8Path::new(repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        return Ok(args
            .positionals
            .iter()
            .map(|p| {
                let abs = Utf8Path::new(p);
                if abs.is_absolute() {
                    repo_relative(repo_root, abs)
                        .unwrap_or_else(|_| abs.to_owned())
                        .to_string()
                } else {
                    p.clone()
                }
            })
            .collect());
    }

    let repo_root_path =
        find_repo_root(Utf8Path::new(repo)).context("cannot find git repo root")?;
    let diff_target = detect_changes_target(&base, staged);
    let changes = changed_files(repo_root_path.as_path(), &diff_target)
        .context("cannot detect changed files")?;
    Ok(changes
        .into_iter()
        .filter(|cf| cf.change_type != atlas_core::model::ChangeType::Deleted)
        .map(|cf| cf.path)
        .collect())
}

fn render_shell_changes_output(store: &Store, repo: &str, args: &ShellArgs) -> Result<String> {
    let base = args.flag_val("base").map(str::to_owned);
    let staged = args.flag_bool("staged");
    let repo_root_path =
        find_repo_root(Utf8Path::new(repo)).context("cannot find git repo root")?;

    let diff_target = if staged {
        DiffTarget::Staged
    } else if let Some(b) = &base {
        DiffTarget::BaseRef(b.clone())
    } else {
        DiffTarget::WorkingTree
    };
    let changes = changed_files(repo_root_path.as_path(), &diff_target)
        .context("cannot detect changed files")?;

    if changes.is_empty() {
        return Ok("No changed files detected.".to_string());
    }

    let mut lines = vec![colorize("Changed files:", "1;36")];
    for cf in &changes {
        let node_info = store
            .nodes_by_file(&cf.path)
            .ok()
            .map(|ns| format!(" [{} nodes]", ns.len()))
            .unwrap_or_default();
        if let Some(old) = &cf.old_path {
            lines.push(format!(
                "  {}  {old} -> {}{}",
                change_tag(cf.change_type),
                cf.path,
                node_info
            ));
        } else {
            lines.push(format!(
                "  {}  {}{}",
                change_tag(cf.change_type),
                cf.path,
                node_info
            ));
        }
    }
    lines.push(format!("\n{} file(s) changed.", changes.len()));
    Ok(lines.join("\n"))
}

fn render_shell_impact_output(store: &Store, repo: &str, args: &ShellArgs) -> Result<String> {
    let max_depth = args.flag_u32("max-depth", 5);
    let max_nodes = args.flag_usize("max-nodes", 200);
    let target_files = resolve_shell_files(repo, args)?;

    if target_files.is_empty() {
        return Ok("No changed files detected.".to_string());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(&path_refs, max_depth, max_nodes)
        .context("impact radius query failed")?;
    let advanced = advanced_impact(result);

    let mut lines = vec![colorize("Impact analysis:", "1;36")];
    lines.push(format!("  Changed files : {}", target_files.len()));
    lines.push(format!(
        "  Changed nodes : {}",
        advanced.base.changed_nodes.len()
    ));
    lines.push(format!(
        "  Impacted nodes: {}",
        advanced.base.impacted_nodes.len()
    ));
    lines.push(format!(
        "  Impacted files: {}",
        advanced.base.impacted_files.len()
    ));
    lines.push(format!("  Risk level    : {}", advanced.risk_level));

    if !advanced.base.impacted_files.is_empty() {
        lines.push(colorize("  Impacted files:", "1;33"));
        for f in advanced.base.impacted_files.iter().take(15) {
            lines.push(format!("    {f}"));
        }
    }
    if !advanced.scored_nodes.is_empty() {
        lines.push(colorize("  Top impacted nodes:", "1;33"));
        for sn in advanced.scored_nodes.iter().take(15) {
            let ck = sn
                .change_kind
                .map(|c| format!(" [{c}]"))
                .unwrap_or_default();
            lines.push(format!(
                "    {:>6.2}  {} {}{}",
                sn.impact_score,
                sn.node.kind.as_str(),
                sn.node.qualified_name,
                ck
            ));
        }
    }
    if !advanced.boundary_violations.is_empty() {
        lines.push(colorize("  Boundary violations:", "1;31"));
        for v in &advanced.boundary_violations {
            lines.push(format!("    [{}] {}", v.kind, v.description));
        }
    }
    Ok(lines.join("\n"))
}

fn render_shell_explain_output(store: &Store, repo: &str, args: &ShellArgs) -> Result<String> {
    let max_depth = args.flag_u32("max-depth", 5);
    let max_nodes = args.flag_usize("max-nodes", 200);
    let target_files = resolve_shell_files(repo, args)?;

    if target_files.is_empty() {
        return Ok("No changed files detected.".to_string());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(&path_refs, max_depth, max_nodes)
        .context("impact radius query failed")?;
    let advanced = advanced_impact(result);

    let workflow_request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: target_files.clone(),
        },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        ..ContextRequest::default()
    };
    let workflow_result = ContextEngine::new(store)
        .build(&workflow_request)
        .context("workflow summary failed")?;
    let workflow = workflow_result.workflow;

    let mut lines = vec![colorize("Change explanation:", "1;36")];
    lines.push(format!("  Risk level      : {}", advanced.risk_level));
    lines.push(format!("  Changed files   : {}", target_files.len()));
    lines.push(format!(
        "  Changed nodes   : {}",
        advanced.base.changed_nodes.len()
    ));
    lines.push(format!(
        "  Impacted nodes  : {}",
        advanced.base.impacted_nodes.len()
    ));

    // Changed symbols by kind.
    let mut api = 0usize;
    let mut sig = 0usize;
    let mut internal = 0usize;
    for sn in &advanced.scored_nodes {
        match sn.change_kind {
            Some(atlas_core::ChangeKind::ApiChange) => api += 1,
            Some(atlas_core::ChangeKind::SignatureChange) => sig += 1,
            Some(atlas_core::ChangeKind::InternalChange) => internal += 1,
            None => {}
        }
    }
    lines.push(format!(
        "  Change kinds    : api {api} | signature {sig} | internal {internal}"
    ));

    let changed_symbols: Vec<_> = advanced
        .scored_nodes
        .iter()
        .filter(|sn| sn.change_kind.is_some())
        .take(20)
        .collect();
    if !changed_symbols.is_empty() {
        lines.push(colorize("  Changed symbols:", "1;33"));
        for sn in &changed_symbols {
            let ck = sn.change_kind.map(|c| format!("{c}")).unwrap_or_default();
            lines.push(format!(
                "    [{}] {} {} ({}:{})",
                ck,
                sn.node.kind.as_str(),
                sn.node.qualified_name,
                sn.node.file_path,
                sn.node.line_start
            ));
        }
    }

    if !advanced.boundary_violations.is_empty() {
        lines.push(colorize("  Boundary violations:", "1;31"));
        for v in &advanced.boundary_violations {
            lines.push(format!("    [{}] {}", v.kind, v.description));
        }
    }

    if let Some(wf) = &workflow {
        if !wf.call_chains.is_empty() {
            lines.push(colorize("  Call chains:", "1;35"));
            for chain in wf.call_chains.iter().take(5) {
                lines.push(format!("    {}", chain.summary));
            }
        }
        if !wf.ripple_effects.is_empty() {
            lines.push(colorize("  Ripple effects:", "1;35"));
            for ripple in wf.ripple_effects.iter().take(5) {
                lines.push(format!("    {ripple}"));
            }
        }
    }

    let affected_tests = advanced.test_impact.affected_tests.len();
    if affected_tests > 0 {
        lines.push(format!("  Affected tests  : {affected_tests}"));
    }
    Ok(lines.join("\n"))
}

fn render_shell_review_output(store: &Store, repo: &str, args: &ShellArgs) -> Result<String> {
    let max_depth = args.flag_u32("max-depth", 3);
    let max_nodes = args.flag_usize("max-nodes", 200);
    let target_files = resolve_shell_files(repo, args)?;

    if target_files.is_empty() {
        return Ok("No changed files detected.".to_string());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let impact = store
        .impact_radius(&path_refs, max_depth, max_nodes)
        .context("impact radius query failed")?;
    let ctx = assemble_review_context(&impact, &target_files, max_depth, max_nodes);

    let mut lines = vec![colorize("Review context:", "1;36")];
    lines.push(colorize(
        &format!("  Changed files ({}):", ctx.changed_files.len()),
        "1;32",
    ));
    for f in &ctx.changed_files {
        lines.push(format!("    {f}"));
    }

    let risk = &ctx.risk_summary;
    lines.push(colorize("  Risk summary:", "1;33"));
    lines.push(format!(
        "    Changed symbols: {} | Public API changes: {}",
        risk.changed_symbol_count, risk.public_api_changes
    ));
    lines.push(format!(
        "    Affected tests : {} | Uncovered: {}",
        risk.affected_test_count, risk.uncovered_changed_symbol_count
    ));
    lines.push(format!(
        "    Cross-module: {} | Cross-package: {}",
        risk.cross_module_impact, risk.cross_package_impact
    ));

    let overview = &ctx.impact_overview;
    lines.push(format!(
        "  Impact: {} nodes, {} files, {} edges (depth {})",
        overview.impacted_node_count,
        overview.impacted_file_count,
        overview.relevant_edge_count,
        overview.max_depth
    ));

    if !ctx.changed_symbol_summaries.is_empty() {
        lines.push(colorize(
            &format!(
                "  Changed symbols ({}):",
                ctx.changed_symbol_summaries.len()
            ),
            "1;32",
        ));
        for sym in ctx.changed_symbol_summaries.iter().take(10) {
            lines.push(format!(
                "    {} {} ({}:{})",
                sym.node.kind.as_str(),
                sym.node.qualified_name,
                sym.node.file_path,
                sym.node.line_start
            ));
            if !sym.callers.is_empty() {
                lines.push(format!(
                    "      callers: {}",
                    sym.callers
                        .iter()
                        .take(3)
                        .map(|n| n.qualified_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }

    if !ctx.impacted_neighbors.is_empty() {
        lines.push(colorize(
            &format!("  Impacted neighbors ({}):", ctx.impacted_neighbors.len()),
            "1;32",
        ));
        for n in ctx.impacted_neighbors.iter().take(15) {
            lines.push(format!(
                "    {} {} ({})",
                n.kind.as_str(),
                n.qualified_name,
                n.file_path
            ));
        }
    }
    Ok(lines.join("\n"))
}

fn render_shell_neighbors_output(store: &Store, qname: &str) -> Result<String> {
    let qname = normalize_qn_kind_tokens(qname);
    let nbhd = sem::symbol_neighborhood(store, &qname, 10).context("symbol_neighborhood failed")?;
    let caller_pairs = store
        .direct_callers(&qname, 10)
        .context("direct_callers failed")?;
    let callee_pairs = store
        .direct_callees(&qname, 10)
        .context("direct_callees failed")?;

    let mut lines = vec![colorize(&format!("Neighbors of `{qname}`:"), "1;36")];

    if !caller_pairs.is_empty() {
        lines.push(colorize(
            &format!("  Callers ({}):", caller_pairs.len()),
            "1;32",
        ));
        for (n, _) in &caller_pairs {
            lines.push(format!(
                "    {} {} ({}:{})",
                n.kind.as_str(),
                n.qualified_name,
                n.file_path,
                n.line_start
            ));
        }
    }

    if !callee_pairs.is_empty() {
        lines.push(colorize(
            &format!("  Callees ({}):", callee_pairs.len()),
            "1;32",
        ));
        for (n, _) in &callee_pairs {
            lines.push(format!(
                "    {} {} ({}:{})",
                n.kind.as_str(),
                n.qualified_name,
                n.file_path,
                n.line_start
            ));
        }
    }

    if !nbhd.tests.is_empty() {
        lines.push(colorize(
            &format!("  Tests ({}):", nbhd.tests.len()),
            "1;32",
        ));
        for n in &nbhd.tests {
            lines.push(format!(
                "    {} {} ({}:{})",
                n.kind.as_str(),
                n.qualified_name,
                n.file_path,
                n.line_start
            ));
        }
    }

    if !nbhd.siblings.is_empty() {
        lines.push(colorize(
            &format!("  Siblings ({}):", nbhd.siblings.len()),
            "1;33",
        ));
        for n in nbhd.siblings.iter().take(8) {
            lines.push(format!("    {} {}", n.kind.as_str(), n.qualified_name));
        }
    }

    if !nbhd.import_neighbors.is_empty() {
        lines.push(colorize(
            &format!("  Import neighbors ({}):", nbhd.import_neighbors.len()),
            "1;33",
        ));
        for n in nbhd.import_neighbors.iter().take(8) {
            lines.push(format!("    {} {}", n.kind.as_str(), n.qualified_name));
        }
    }

    if caller_pairs.is_empty() && callee_pairs.is_empty() && nbhd.tests.is_empty() {
        lines.push("  No neighbors found. Check qualified name.".to_string());
    }
    Ok(lines.join("\n"))
}

fn render_shell_traverse_output(store: &Store, args: &ShellArgs) -> Result<String> {
    let qname = match args.positionals.first() {
        Some(q) => normalize_qn_kind_tokens(q),
        None => {
            return Ok("Usage: /traverse <qualified_name> [--depth N] [--max-nodes N]".to_string());
        }
    };
    let max_depth = args.flag_u32("depth", 3);
    let max_nodes = args.flag_usize("max-nodes", 100);

    let result = store
        .traverse_from_qnames(&[qname.as_str()], max_depth, max_nodes)
        .context("traverse_from_qnames failed")?;

    let mut lines = vec![colorize(&format!("Traverse from `{qname}`:"), "1;36")];
    lines.push(format!("  Changed nodes : {}", result.changed_nodes.len()));
    lines.push(format!("  Impacted nodes: {}", result.impacted_nodes.len()));
    lines.push(format!("  Impacted files: {}", result.impacted_files.len()));
    lines.push(format!("  Relevant edges: {}", result.relevant_edges.len()));

    if !result.impacted_nodes.is_empty() {
        lines.push(colorize("  Reachable nodes:", "1;32"));
        for n in result.impacted_nodes.iter().take(20) {
            lines.push(format!(
                "    {} {} ({}:{})",
                n.kind.as_str(),
                n.qualified_name,
                n.file_path,
                n.line_start
            ));
        }
    }
    if !result.impacted_files.is_empty() {
        lines.push(colorize("  Reachable files:", "1;33"));
        for f in result.impacted_files.iter().take(15) {
            lines.push(format!("    {f}"));
        }
    }
    Ok(lines.join("\n"))
}

fn render_shell_context_output(store: &Store, text: &str) -> Result<String> {
    let request = query_parser::parse_query(text);
    let result = ContextEngine::new(store)
        .build(&request)
        .context("context engine failed")?;
    Ok(format_context_output(&result))
}

fn render_shell_session_output(repo: &str, subcmd: &str) -> Result<String> {
    use atlas_session::{SessionId, SessionStore};
    use std::path::Path;

    let store = SessionStore::open_in_repo(Path::new(repo)).context("cannot open session store")?;
    let session_id = SessionId::derive(repo, "", "cli");

    match subcmd {
        "status" | "" => {
            let mut lines = vec![colorize("Session status:", "1;36")];
            match store.get_session_meta(&session_id) {
                Ok(Some(meta)) => {
                    lines.push(format!("  ID       : {}", session_id));
                    lines.push(format!("  Frontend : {}", meta.frontend));
                    lines.push(format!("  Updated  : {}", meta.updated_at));
                    let events = store.list_events(&session_id).unwrap_or_default();
                    lines.push(format!("  Events   : {}", events.len()));
                    let has_snap = store
                        .get_resume_snapshot(&session_id)
                        .ok()
                        .flatten()
                        .is_some();
                    lines.push(format!(
                        "  Snapshot : {}",
                        if has_snap { "yes" } else { "none" }
                    ));
                }
                Ok(None) => lines.push("  No active session.".to_string()),
                Err(e) => lines.push(format!("  Error: {e}")),
            }
            Ok(lines.join("\n"))
        }
        "list" => {
            let sessions = store.list_sessions().context("cannot list sessions")?;
            if sessions.is_empty() {
                return Ok("No sessions found.".to_string());
            }
            let mut lines = vec![colorize(&format!("Sessions ({}):", sessions.len()), "1;36")];
            for s in &sessions {
                lines.push(format!(
                    "  {}  {}  {}",
                    s.updated_at, s.frontend, s.session_id
                ));
            }
            Ok(lines.join("\n"))
        }
        other => Ok(format!(
            "Unknown session subcommand: `{other}`. Use `status` or `list`."
        )),
    }
}

fn render_shell_flows_output(store: &Store, args: &ShellArgs) -> Result<String> {
    let subcmd = args
        .positionals
        .first()
        .map(String::as_str)
        .unwrap_or("list");

    match subcmd {
        "list" | "" => {
            let flows = store.list_flows().context("cannot list flows")?;
            if flows.is_empty() {
                return Ok("No flows defined.".to_string());
            }
            let mut lines = vec![colorize(&format!("Flows ({}):", flows.len()), "1;36")];
            for f in &flows {
                let desc = f.description.as_deref().unwrap_or("");
                let kind = f.kind.as_deref().unwrap_or("-");
                lines.push(format!("  [{}] {} — {}", kind, f.name, desc));
            }
            Ok(lines.join("\n"))
        }
        "members" => {
            let name = match args.positionals.get(1) {
                Some(n) => n.as_str(),
                None => return Ok("Usage: /flows members <name>".to_string()),
            };
            let flow = store
                .get_flow_by_name(name)
                .context("flow lookup failed")?
                .ok_or_else(|| anyhow::anyhow!("flow not found: {name}"))?;
            let members = store
                .get_flow_members(flow.id)
                .context("cannot get flow members")?;
            if members.is_empty() {
                return Ok(format!("Flow `{name}` has no members."));
            }
            let mut lines = vec![colorize(
                &format!("Flow `{name}` members ({}):", members.len()),
                "1;36",
            )];
            for m in &members {
                let pos = m.position.map(|p| format!("[{p}] ")).unwrap_or_default();
                let role = m
                    .role
                    .as_deref()
                    .map(|r| format!(" ({r})"))
                    .unwrap_or_default();
                lines.push(format!("  {}{}{}", pos, m.node_qualified_name, role));
            }
            Ok(lines.join("\n"))
        }
        other => Ok(format!(
            "Unknown flows subcommand: `{other}`. Use `list` or `members <name>`."
        )),
    }
}

fn render_shell_communities_output(store: &Store, args: &ShellArgs) -> Result<String> {
    let subcmd = args
        .positionals
        .first()
        .map(String::as_str)
        .unwrap_or("list");

    match subcmd {
        "list" | "" => {
            let communities = store
                .list_communities()
                .context("cannot list communities")?;
            if communities.is_empty() {
                return Ok("No communities defined.".to_string());
            }
            let mut lines = vec![colorize(
                &format!("Communities ({}):", communities.len()),
                "1;36",
            )];
            for c in &communities {
                let algo = c.algorithm.as_deref().unwrap_or("-");
                let level = c.level.map(|l| format!(" L{l}")).unwrap_or_default();
                lines.push(format!("  [{}{}] {}", algo, level, c.name));
            }
            Ok(lines.join("\n"))
        }
        "nodes" => {
            let name = match args.positionals.get(1) {
                Some(n) => n.as_str(),
                None => return Ok("Usage: /communities nodes <name>".to_string()),
            };
            let community = store
                .get_community_by_name(name)
                .context("community lookup failed")?
                .ok_or_else(|| anyhow::anyhow!("community not found: {name}"))?;
            let nodes = store
                .get_community_nodes(community.id)
                .context("cannot get community nodes")?;
            if nodes.is_empty() {
                return Ok(format!("Community `{name}` has no nodes."));
            }
            let mut lines = vec![colorize(
                &format!("Community `{name}` nodes ({}):", nodes.len()),
                "1;36",
            )];
            for n in &nodes {
                lines.push(format!("  {}", n.node_qualified_name));
            }
            Ok(lines.join("\n"))
        }
        other => Ok(format!(
            "Unknown communities subcommand: `{other}`. Use `list` or `nodes <name>`."
        )),
    }
}

const SHELL_HELP: &str = "\
Slash commands:
  /query <text>                        full-text graph search
  /stats                               node/edge counts and language breakdown
  /changes [--base <ref>] [--staged]   changed files from git diff
  /impact [--base <ref>] [--staged] [--max-depth N] [--max-nodes N] [files...]
                                       blast radius from changed files
  /explain [--base <ref>] [--staged] [--max-depth N] [--max-nodes N] [files...]
                                       change explanation with risk analysis
  /review [--base <ref>] [--staged] [--max-depth N] [--max-nodes N] [files...]
                                       review context: symbols, neighbors, risk
  /neighbors <qualified_name>          callers, callees, tests, siblings
  /traverse <qualified_name> [--depth N] [--max-nodes N]
                                       graph traversal from a symbol
  /context <text>                      natural-language context query
  /session [status|list]               session metadata (read-only)
  /flows [list|members <name>]         flow listing
  /communities [list|nodes <name>]     community listing

Natural language (no slash prefix):
  who calls handle_request
  what breaks if I change helper
  where is AuthService used

Other:
  help     show this message
  exit     quit shell";

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
            "Type natural-language graph questions or use slash commands.",
            "Type `help` for command list, `exit` or `quit` to leave."
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
            emit_shell_output(SHELL_HELP, paging)?;
            continue;
        }

        let rendered = if let Some(query_text) = input.strip_prefix("/query ") {
            render_shell_query_output(&store, query_text.trim(), fuzzy)?
        } else if input == "/stats" {
            render_shell_stats_output(&store)?
        } else if let Some(rest) = input.strip_prefix("/changes") {
            render_shell_changes_output(&store, &repo, &ShellArgs::parse(rest.trim()))?
        } else if let Some(rest) = input.strip_prefix("/impact") {
            render_shell_impact_output(&store, &repo, &ShellArgs::parse(rest.trim()))?
        } else if let Some(rest) = input.strip_prefix("/explain") {
            render_shell_explain_output(&store, &repo, &ShellArgs::parse(rest.trim()))?
        } else if let Some(rest) = input.strip_prefix("/review") {
            render_shell_review_output(&store, &repo, &ShellArgs::parse(rest.trim()))?
        } else if let Some(rest) = input.strip_prefix("/neighbors") {
            let qname = rest.trim();
            if qname.is_empty() {
                "Usage: /neighbors <qualified_name>".to_string()
            } else {
                render_shell_neighbors_output(&store, qname)?
            }
        } else if let Some(rest) = input.strip_prefix("/traverse") {
            render_shell_traverse_output(&store, &ShellArgs::parse(rest.trim()))?
        } else if let Some(rest) = input.strip_prefix("/context") {
            let text = rest.trim();
            if text.is_empty() {
                "Usage: /context <natural-language query>".to_string()
            } else {
                render_shell_context_output(&store, text)?
            }
        } else if let Some(rest) = input.strip_prefix("/session") {
            render_shell_session_output(&repo, rest.trim())?
        } else if let Some(rest) = input.strip_prefix("/flows") {
            render_shell_flows_output(&store, &ShellArgs::parse(rest.trim()))?
        } else if let Some(rest) = input.strip_prefix("/communities") {
            render_shell_communities_output(&store, &ShellArgs::parse(rest.trim()))?
        } else {
            let request = query_parser::parse_query(input);
            let result = engine.build(&request).context("context engine failed")?;
            format_context_output(&result)
        };

        emit_shell_output(&rendered, paging)?;
    }

    Ok(())
}
