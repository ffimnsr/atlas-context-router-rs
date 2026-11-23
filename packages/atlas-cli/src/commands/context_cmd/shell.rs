use super::*;

// ---------------------------------------------------------------------------
// Shell mini-argument parser
// ---------------------------------------------------------------------------

/// Parsed shell command arguments.
///
/// Tokens starting with `--` are flags. A flag is a boolean flag when the
/// next token also starts with `--` or there is no next token; otherwise the
/// following token becomes its value. All other tokens are positionals.
pub(crate) struct ShellArgs {
    flags: HashMap<String, Option<String>>,
    positionals: Vec<String>,
}

impl ShellArgs {
    pub(crate) fn parse(input: &str) -> Self {
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

    pub(crate) fn flag_bool(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    pub(crate) fn flag_val(&self, name: &str) -> Option<&str> {
        self.flags.get(name)?.as_deref()
    }

    pub(crate) fn flag_u32(&self, name: &str, default: u32) -> u32 {
        self.flag_val(name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    pub(crate) fn flag_usize(&self, name: &str, default: usize) -> usize {
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
        return args
            .positionals
            .iter()
            .map(|path| {
                atlas_repo::normalize_repo_file_path(repo_root, path)
                    .map(|resolved| resolved.canonical)
                    .with_context(|| format!("invalid explicit file path '{path}'"))
            })
            .collect();
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
    let policy = load_budget_policy(repo)?;

    if target_files.is_empty() {
        return Ok("No changed files detected.".to_string());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(
            &path_refs,
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
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
    let policy = load_budget_policy(repo)?;

    if target_files.is_empty() {
        return Ok("No changed files detected.".to_string());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(
            &path_refs,
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
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
    let token_counter = load_token_counter(repo)?;
    let workflow_result = ContextEngine::new(store)
        .with_budget_policy(policy)
        .with_token_counter(token_counter.counter)
        .with_token_fallback(token_counter.fallback_used, token_counter.fallback_reason)
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

    let request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: target_files.clone(),
        },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        ..ContextRequest::default()
    };
    let token_counter = load_token_counter(repo)?;
    let ctx = ContextEngine::new(store)
        .with_token_counter(token_counter.counter)
        .with_token_fallback(token_counter.fallback_used, token_counter.fallback_reason)
        .build(&request)
        .context("context engine failed")?;

    let mut lines = vec![colorize("Review context:", "1;36")];
    lines.push(colorize(
        &format!("  Changed files ({}):", target_files.len()),
        "1;32",
    ));
    for f in &target_files {
        lines.push(format!("    {f}"));
    }

    lines.push(format!(
        "  Context: {} nodes, {} files, {} edges (depth {})",
        ctx.nodes.len(),
        ctx.files.len(),
        ctx.edges.len(),
        ctx.request.depth.unwrap_or(max_depth)
    ));

    let changed_symbols: Vec<_> = ctx
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .collect();

    if !changed_symbols.is_empty() {
        lines.push(colorize(
            &format!("  Changed symbols ({}):", changed_symbols.len()),
            "1;32",
        ));
        for sym in changed_symbols.iter().take(10) {
            lines.push(format!(
                "    {} {} ({}:{})",
                sym.node.kind.as_str(),
                sym.node.qualified_name,
                sym.node.file_path,
                sym.node.line_start
            ));
        }
    }

    if let Some(workflow) = &ctx.workflow {
        let cross_package_impact = workflow
            .impacted_components
            .iter()
            .filter(|component| component.kind == "package")
            .count()
            > 1;
        if let Some(headline) = &workflow.headline {
            lines.push(colorize(&format!("  Focus: {headline}"), "1;33"));
        }
        lines.push(colorize("  Risk summary:", "1;33"));
        lines.push(format!("    Cross-package impact: {cross_package_impact}"));
        if !workflow.high_impact_nodes.is_empty() {
            lines.push(colorize(
                &format!(
                    "  High-impact nodes ({}):",
                    workflow.high_impact_nodes.len()
                ),
                "1;32",
            ));
            for node in workflow.high_impact_nodes.iter().take(10) {
                lines.push(format!(
                    "    [{:.1}] {} {} ({})",
                    node.relevance_score, node.kind, node.qualified_name, node.file_path
                ));
            }
        }
        if !workflow.call_chains.is_empty() {
            lines.push(colorize("  Call chains:", "1;35"));
            for chain in workflow.call_chains.iter().take(5) {
                lines.push(format!("    {}", chain.summary));
            }
        }
        if !workflow.ripple_effects.is_empty() {
            lines.push(colorize("  Ripple effects:", "1;35"));
            for ripple in workflow.ripple_effects.iter().take(5) {
                lines.push(format!("    {ripple}"));
            }
        }
        lines.push(colorize("  Noise reduction:", "1;33"));
        lines.push(format!(
            "    Retained nodes: {} | Dropped nodes: {}",
            workflow.noise_reduction.retained_nodes, workflow.noise_reduction.dropped_nodes
        ));
        lines.push(format!(
            "    Retained edges: {} | Dropped edges: {}",
            workflow.noise_reduction.retained_edges, workflow.noise_reduction.dropped_edges
        ));
        lines.push(format!(
            "    Retained files: {} | Dropped files: {}",
            workflow.noise_reduction.retained_files, workflow.noise_reduction.dropped_files
        ));
    }

    if ctx.truncation.truncated {
        lines.push(colorize("  Truncation:", "1;33"));
        lines.push(format!(
            "    Nodes dropped: {} | Edges dropped: {} | Files dropped: {}",
            ctx.truncation.nodes_dropped,
            ctx.truncation.edges_dropped,
            ctx.truncation.files_dropped
        ));
    }
    if let Some(payload) = &ctx.truncation.payload {
        lines.push(colorize("  Payload:", "1;33"));
        lines.push(format!("    {}", payload_accounting_text(payload)));
    }

    if !ctx.files.is_empty() {
        lines.push(colorize(
            &format!("  Selected files ({}):", ctx.files.len()),
            "1;32",
        ));
        for file in ctx.files.iter().take(15) {
            lines.push(format!(
                "    {} [{}] nodes={} reason={}",
                file.path,
                file.language.as_deref().unwrap_or("unknown"),
                file.node_count_included,
                file.selection_reason.as_str()
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

fn render_shell_traverse_output(store: &Store, repo: &str, args: &ShellArgs) -> Result<String> {
    let qname = match args.positionals.first() {
        Some(q) => normalize_qn_kind_tokens(q),
        None => {
            return Ok("Usage: /traverse <qualified_name> [--depth N] [--max-nodes N]".to_string());
        }
    };
    let max_depth = args.flag_u32("depth", 3);
    let max_nodes = args.flag_usize("max-nodes", 100);
    let policy = load_budget_policy(repo)?;

    let result = store
        .traverse_from_qnames(
            &[qname.as_str()],
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
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

fn render_shell_context_output(store: &Store, repo: &str, text: &str) -> Result<String> {
    let request = query_parser::parse_query(text);
    let token_counter = load_token_counter(repo)?;
    let result = ContextEngine::new(store)
        .with_token_counter(token_counter.counter)
        .with_token_fallback(token_counter.fallback_used, token_counter.fallback_reason)
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
  /symbols [--kind <K>] [--limit N] [--offset N]
                                       paginated symbol listing
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

    let store = match Store::open(&db_path) {
        Err(e) => {
            let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &e.to_string());
            check_graph_readiness(
                &readiness,
                GraphToolRequirement::Analysis,
                readiness_overrides(false, false),
                "shell",
                cli,
            )?;
            return Err(e).with_context(|| format!("cannot open database at {db_path}"));
        }
        Ok(s) => s,
    };
    let readiness = derive_graph_readiness(&store, &repo, &db_path);
    if let Some(warning) = check_graph_readiness(
        &readiness,
        GraphToolRequirement::Analysis,
        readiness_overrides(false, false),
        "shell",
        cli,
    )? {
        eprintln!("Warning: {warning}");
    }
    let token_counter = load_token_counter(&repo)?;
    let engine = ContextEngine::new(&store)
        .with_budget_policy(load_budget_policy(&repo)?)
        .with_token_counter(token_counter.counter)
        .with_token_fallback(token_counter.fallback_used, token_counter.fallback_reason);
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
        } else if let Some(rest) = input.strip_prefix("/symbols") {
            render_shell_symbols_output(&store, &repo, &ShellArgs::parse(rest.trim()))?
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
            render_shell_traverse_output(&store, &repo, &ShellArgs::parse(rest.trim()))?
        } else if let Some(rest) = input.strip_prefix("/context") {
            let text = rest.trim();
            if text.is_empty() {
                "Usage: /context <natural-language query>".to_string()
            } else {
                render_shell_context_output(&store, &repo, text)?
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
