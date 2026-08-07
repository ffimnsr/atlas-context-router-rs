use crate::cli::{Cli, Command, ReviewContextFormat};
use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_core::BudgetReport;
use atlas_core::GraphToolRequirement;
use atlas_core::model::{
    ChangeType, ContextIntent, ContextRequest, ContextResult, ContextTarget, ImpactResult,
    ReviewContext, ReviewImpactOverview, RiskSummary, SelectionReason,
};
use atlas_impact::analyze as advanced_impact;
use atlas_repo::{
    CanonicalRepoPath, DiffTarget, RepoRegistry, changed_files, find_repo_root,
    phase1_multi_repo_supported, stable_repo_id,
};
use atlas_review::{ContextEngine, build_explain_change_summary, empty_explain_change_summary};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use std::fmt::Write as _;

use super::{
    augment_changes_with_node_counts, change_tag, check_graph_readiness, db_path,
    derive_graph_readiness, derive_graph_readiness_open_failed, detect_changes_target,
    load_budget_policy, load_token_counter, payload_accounting_text, print_json,
    readiness_overrides, resolve_repo,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const MAX_MULTI_REPO_SELECTION: usize = 32;

fn normalize_explicit_files(
    repo_root: &Utf8Path,
    explicit_files: &[String],
) -> Result<Vec<String>> {
    explicit_files
        .iter()
        .map(|path| {
            CanonicalRepoPath::from_cli_argument(repo_root, Utf8Path::new(path))
                .with_context(|| format!("invalid explicit file path '{path}'"))
                .map(|path| path.as_str().to_owned())
        })
        .collect()
}

fn node_repo_id(node: &atlas_core::Node) -> Option<&str> {
    node.extra_json
        .as_object()
        .and_then(|extra| extra.get("repo_id"))
        .and_then(|value| value.as_str())
}

fn selected_impact_repos(
    registry_root: &Utf8Path,
    repo_id: &Option<String>,
    all_repos: bool,
) -> Result<(Vec<atlas_repo::RepoRegistration>, usize)> {
    if !all_repos && repo_id.is_none() {
        return Ok((Vec::new(), 0));
    }
    let registry = RepoRegistry::load(registry_root).with_context(
        || "repo registry missing; run `atlas init` or `atlas repo sync` before multi-repo impact",
    )?;
    let excluded_manual = if all_repos {
        registry
            .registrations
            .iter()
            .filter(|entry| entry.enabled)
            .filter(|entry| entry.relationship.kind == atlas_repo::RepoRelationshipKind::Manual)
            .count()
    } else {
        0
    };
    if all_repos {
        let registrations = registry
            .registrations
            .into_iter()
            .filter(|entry| entry.enabled)
            .filter(|entry| phase1_multi_repo_supported(entry.relationship.kind))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            registrations.len() <= MAX_MULTI_REPO_SELECTION,
            "all_repos scope exceeds max supported repo fan-out ({MAX_MULTI_REPO_SELECTION})"
        );
        return Ok((registrations, excluded_manual));
    }
    let target = repo_id.as_deref().unwrap_or_default();
    let entry = registry
        .registrations
        .into_iter()
        .find(|entry| entry.repo_id == target)
        .with_context(|| format!("repo id '{target}' is not registered"))?;
    anyhow::ensure!(entry.enabled, "repo id '{target}' is disabled");
    Ok((vec![entry], excluded_manual))
}

fn impact_seed_qnames_for_repo(
    store: &Store,
    repo_id: &str,
    files: &[String],
) -> Result<Vec<String>> {
    let mut qnames = Vec::new();
    for file in files {
        for node in store.nodes_by_file(file)? {
            if node_repo_id(&node) == Some(repo_id) {
                qnames.push(node.qualified_name);
            }
        }
    }
    qnames.sort();
    qnames.dedup();
    Ok(qnames)
}

fn print_review_context_text(ctx: &ContextResult, changed_files: &[String]) {
    println!("Changed files ({}):", changed_files.len());
    for path in changed_files {
        println!("  {path}");
    }

    println!("\nContext summary:");
    println!("  Selected nodes   : {}", ctx.nodes.len());
    println!("  Selected edges   : {}", ctx.edges.len());
    println!("  Selected files   : {}", ctx.files.len());
    println!(
        "  Max depth        : {}",
        ctx.request.depth.unwrap_or_default()
    );
    println!(
        "  Max nodes        : {}",
        ctx.request.max_nodes.unwrap_or(ctx.nodes.len())
    );

    let changed_symbols: Vec<_> = ctx
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .collect();
    println!("\nChanged symbols: {}", changed_symbols.len());
    for selected in changed_symbols.iter().take(10) {
        println!(
            "  {} {} ({}:{})",
            selected.node.kind.as_str(),
            selected.node.qualified_name,
            selected.node.file_path,
            selected.node.line_start
        );
    }

    if let Some(workflow) = &ctx.workflow {
        let cross_package_impact = workflow
            .impacted_components
            .iter()
            .filter(|component| component.kind == "package")
            .count()
            > 1;
        println!("\nRisk summary:");
        println!("  Cross-package impact: {cross_package_impact}");
        if let Some(headline) = &workflow.headline {
            println!("\nFocus: {headline}");
        }
        if !workflow.high_impact_nodes.is_empty() {
            println!("\nHigh-impact nodes:");
            for node in workflow.high_impact_nodes.iter().take(5) {
                println!(
                    "  [{:.1}] {} {} ({})",
                    node.relevance_score, node.kind, node.qualified_name, node.file_path
                );
            }
        }
        if !workflow.impacted_components.is_empty() {
            println!("\nImpacted components:");
            for component in workflow.impacted_components.iter().take(5) {
                println!(
                    "  [{}] {} | changed {} | impacted {} | files {}",
                    component.kind,
                    component.label,
                    component.changed_node_count,
                    component.impacted_node_count,
                    component.file_count
                );
            }
        }
        if !workflow.call_chains.is_empty() {
            println!("\nCall chains:");
            for chain in workflow.call_chains.iter().take(5) {
                println!("  {}", chain.summary);
            }
        }
        if !workflow.ripple_effects.is_empty() {
            println!("\nRipple effects:");
            for ripple in &workflow.ripple_effects {
                println!("  {ripple}");
            }
        }
        println!("\nNoise reduction:");
        println!(
            "  Retained nodes   : {}",
            workflow.noise_reduction.retained_nodes
        );
        println!(
            "  Retained edges   : {}",
            workflow.noise_reduction.retained_edges
        );
        println!(
            "  Retained files   : {}",
            workflow.noise_reduction.retained_files
        );
        println!(
            "  Dropped nodes    : {}",
            workflow.noise_reduction.dropped_nodes
        );
        println!(
            "  Dropped edges    : {}",
            workflow.noise_reduction.dropped_edges
        );
        println!(
            "  Dropped files    : {}",
            workflow.noise_reduction.dropped_files
        );
    }

    if ctx.truncation.truncated {
        println!("\nTruncation:");
        println!("  Nodes dropped    : {}", ctx.truncation.nodes_dropped);
        println!("  Edges dropped    : {}", ctx.truncation.edges_dropped);
        println!("  Files dropped    : {}", ctx.truncation.files_dropped);
    }
    if let Some(payload) = &ctx.truncation.payload {
        println!("  Payload          : {}", payload_accounting_text(payload));
    }
}

fn markdown_label(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '`' => escaped.push_str("\\`"),
            '*' | '_' | '[' | ']' | '<' | '>' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            '\n' | '\r' => escaped.push(' '),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn push_markdown_list(out: &mut String, items: &[String], limit: usize) {
    for item in items.iter().take(limit) {
        let _ = writeln!(out, "- {}", markdown_label(item));
    }
    if items.len() > limit {
        let _ = writeln!(out, "- ... and {} more", items.len() - limit);
    }
}

fn render_review_context_markdown(ctx: &ContextResult, changed_files: &[String]) -> String {
    let mut out = String::new();
    let changed_symbols: Vec<_> = ctx
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .collect();
    let cross_package_impact = ctx
        .workflow
        .as_ref()
        .map(|workflow| {
            workflow
                .impacted_components
                .iter()
                .filter(|component| component.kind == "package")
                .count()
                > 1
        })
        .unwrap_or(false);

    out.push_str("## Atlas Review Context\n\n");

    if let Some(workflow) = &ctx.workflow
        && let Some(headline) = &workflow.headline
    {
        let _ = writeln!(out, "> {}\n", markdown_label(headline));
    }

    out.push_str("### Summary\n\n");
    let _ = writeln!(out, "- Changed files: {}", changed_files.len());
    let _ = writeln!(out, "- Changed symbols: {}", changed_symbols.len());
    let _ = writeln!(out, "- Selected nodes: {}", ctx.nodes.len());
    let _ = writeln!(out, "- Selected edges: {}", ctx.edges.len());
    let _ = writeln!(out, "- Selected files: {}", ctx.files.len());
    let _ = writeln!(
        out,
        "- Max depth: {}",
        ctx.request.depth.unwrap_or_default()
    );
    let _ = writeln!(
        out,
        "- Max nodes: {}",
        ctx.request.max_nodes.unwrap_or(ctx.nodes.len())
    );
    let _ = writeln!(
        out,
        "- Cross-package impact: {}",
        yes_no(cross_package_impact)
    );
    if ctx.truncation.truncated {
        let _ = writeln!(
            out,
            "- Truncated: yes (dropped nodes {}, edges {}, files {})",
            ctx.truncation.nodes_dropped,
            ctx.truncation.edges_dropped,
            ctx.truncation.files_dropped
        );
    } else {
        out.push_str("- Truncated: no\n");
    }
    out.push('\n');

    out.push_str("<details>\n<summary>Changed files</summary>\n\n");
    push_markdown_list(&mut out, changed_files, 20);
    out.push_str("\n</details>\n\n");

    if !changed_symbols.is_empty() {
        out.push_str("### Changed Symbols\n\n");
        for selected in changed_symbols.iter().take(12) {
            let _ = writeln!(
                out,
                "- `{}` `{}` in `{}`:{}",
                selected.node.kind.as_str(),
                selected.node.qualified_name,
                selected.node.file_path,
                selected.node.line_start
            );
        }
        if changed_symbols.len() > 12 {
            let _ = writeln!(out, "- ... and {} more", changed_symbols.len() - 12);
        }
        out.push('\n');
    }

    if let Some(workflow) = &ctx.workflow {
        if !workflow.high_impact_nodes.is_empty() {
            out.push_str("### Reviewer Focus\n\n");
            for node in workflow.high_impact_nodes.iter().take(8) {
                let _ = writeln!(
                    out,
                    "- {:.1} `{}` `{}` in `{}` ({})",
                    node.relevance_score,
                    node.kind,
                    node.qualified_name,
                    node.file_path,
                    markdown_label(&node.selection_reason)
                );
            }
            if workflow.high_impact_nodes.len() > 8 {
                let _ = writeln!(
                    out,
                    "- ... and {} more",
                    workflow.high_impact_nodes.len() - 8
                );
            }
            out.push('\n');
        }

        if !workflow.impacted_components.is_empty() {
            out.push_str("### Impacted Components\n\n");
            for component in workflow.impacted_components.iter().take(8) {
                let _ = writeln!(
                    out,
                    "- `{}` {}: changed {}, impacted {}, files {}",
                    component.kind,
                    markdown_label(&component.label),
                    component.changed_node_count,
                    component.impacted_node_count,
                    component.file_count
                );
            }
            if workflow.impacted_components.len() > 8 {
                let _ = writeln!(
                    out,
                    "- ... and {} more",
                    workflow.impacted_components.len() - 8
                );
            }
            out.push('\n');
        }

        if !workflow.call_chains.is_empty() {
            out.push_str("### Critical Paths\n\n```text\n");
            for chain in workflow.call_chains.iter().take(6) {
                let _ = writeln!(out, "{}", chain.summary);
            }
            if workflow.call_chains.len() > 6 {
                let _ = writeln!(out, "... and {} more", workflow.call_chains.len() - 6);
            }
            out.push_str("```\n\n");
        }

        if !workflow.ripple_effects.is_empty() {
            out.push_str("### Ripple Effects\n\n");
            for ripple in workflow.ripple_effects.iter().take(8) {
                let _ = writeln!(out, "- {}", markdown_label(ripple));
            }
            if workflow.ripple_effects.len() > 8 {
                let _ = writeln!(out, "- ... and {} more", workflow.ripple_effects.len() - 8);
            }
            out.push('\n');
        }

        out.push_str("<details>\n<summary>Noise reduction</summary>\n\n");
        let _ = writeln!(
            out,
            "- Retained nodes: {}",
            workflow.noise_reduction.retained_nodes
        );
        let _ = writeln!(
            out,
            "- Retained edges: {}",
            workflow.noise_reduction.retained_edges
        );
        let _ = writeln!(
            out,
            "- Retained files: {}",
            workflow.noise_reduction.retained_files
        );
        let _ = writeln!(
            out,
            "- Dropped nodes: {}",
            workflow.noise_reduction.dropped_nodes
        );
        let _ = writeln!(
            out,
            "- Dropped edges: {}",
            workflow.noise_reduction.dropped_edges
        );
        let _ = writeln!(
            out,
            "- Dropped files: {}",
            workflow.noise_reduction.dropped_files
        );
        if !workflow.noise_reduction.rules_applied.is_empty() {
            out.push_str("\nApplied rules:\n");
            push_markdown_list(&mut out, &workflow.noise_reduction.rules_applied, 12);
        }
        out.push_str("\n</details>\n");
    }

    out
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub fn run_detect_changes(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let (base, staged, selected_repo_id, all_repos) = match &cli.command {
        Command::DetectChanges {
            base,
            staged,
            repo_id,
            all_repos,
        } => (base.clone(), *staged, repo_id.clone(), *all_repos),
        _ => unreachable!(),
    };
    let diff_target = detect_changes_target(&base, staged);

    if all_repos || selected_repo_id.is_some() {
        let (registrations, excluded_manual_repo_count) =
            selected_impact_repos(Utf8Path::new(&repo), &selected_repo_id, all_repos)?;
        let mut repo_changes = Vec::new();
        let mut warnings = Vec::new();
        let mut processed_repo_count = 0usize;
        let mut failed_repo_count = 0usize;
        let mut skipped_repo_count = 0usize;
        let mut changed_file_count = 0usize;
        for registration in registrations {
            if !registration.root.exists() {
                failed_repo_count += 1;
                skipped_repo_count += 1;
                warnings.push(format!(
                    "skipped repo {}: repo root missing",
                    registration.display_alias
                ));
                repo_changes.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "root": registration.root,
                    "status": "skipped",
                    "error": "repo root missing",
                    "changes": [],
                }));
                continue;
            }
            match changed_files(registration.root.as_path(), &diff_target) {
                Ok(changes) => {
                    processed_repo_count += 1;
                    changed_file_count += changes.len();
                    repo_changes.push(serde_json::json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "root": registration.root,
                        "status": "ok",
                        "changed_file_count": changes.len(),
                        "changes": augment_changes_with_node_counts(&changes, None),
                    }));
                }
                Err(error) => {
                    failed_repo_count += 1;
                    skipped_repo_count += 1;
                    warnings.push(format!(
                        "skipped repo {}: {}",
                        registration.display_alias, error
                    ));
                    repo_changes.push(serde_json::json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "root": registration.root,
                        "status": "skipped",
                        "error": error.to_string(),
                        "changes": [],
                    }));
                }
            }
        }
        if cli.json {
            return print_json(
                "detect_changes",
                serde_json::json!({
                    "diff_target": {
                        "base": base,
                        "staged": staged,
                        "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
                    },
                    "repo_scope": {
                        "selected_repo_count": repo_changes.len(),
                        "processed_repo_count": processed_repo_count,
                        "failed_repo_count": failed_repo_count,
                        "skipped_repo_count": skipped_repo_count,
                        "excluded_manual_repo_count": excluded_manual_repo_count,
                    },
                    "summary": {
                        "changed_file_count": changed_file_count,
                        "warning_count": warnings.len(),
                    },
                    "warnings": warnings,
                    "repos": repo_changes,
                }),
            );
        }
        println!(
            "Detected changes across {} repo(s); processed={} failures={} skipped={} changed_files={}",
            repo_changes.len(),
            processed_repo_count,
            failed_repo_count,
            skipped_repo_count,
            changed_file_count,
        );
        if excluded_manual_repo_count > 0 {
            println!(
                "Phase-1 rollout: excluded {} manual repo(s) from --all-repos. Use --repo-id to target them explicitly.",
                excluded_manual_repo_count
            );
        }
        for warning in &warnings {
            println!("warning\t{warning}");
        }
        for repo_change in &repo_changes {
            println!("{}", repo_change);
        }
        return Ok(());
    }

    let changes = changed_files(repo_root, &diff_target).context("cannot detect changed files")?;

    // Try to open the DB for graph summary — tolerate failure (DB may not exist yet).
    let store_result = Store::open(&db_path);

    if cli.json {
        print_json(
            "detect_changes",
            serde_json::json!({
                "diff_target": {
                    "base": base,
                    "staged": staged,
                    "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
                },
                "changes": augment_changes_with_node_counts(&changes, store_result.as_ref().ok()),
            }),
        )?;
    } else if changes.is_empty() {
        println!("No changed files detected.");
    } else {
        for cf in &changes {
            let node_info = store_result
                .as_ref()
                .ok()
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| format!(" [{} nodes]", ns.len()))
                .unwrap_or_default();
            if let Some(old) = &cf.old_path {
                println!(
                    "{}  {old} -> {}{node_info}",
                    change_tag(cf.change_type),
                    cf.path
                );
            } else {
                println!("{}  {}{node_info}", change_tag(cf.change_type), cf.path);
            }
        }
        println!("\n{} file(s) changed.", changes.len());

        // Graph-level impact summary when DB is available.
        if let Ok(store) = &store_result {
            let policy = load_budget_policy(&repo)?;
            let non_deleted: Vec<&str> = changes
                .iter()
                .filter(|cf| cf.change_type != ChangeType::Deleted)
                .map(|cf| cf.path.as_str())
                .collect();
            if !non_deleted.is_empty()
                && let Ok(impact) = store.impact_radius(
                    &non_deleted,
                    5,
                    200,
                    policy.graph_traversal.edges.default_limit,
                )
            {
                println!("\nGraph impact summary:");
                println!("  Changed symbols : {}", impact.changed_nodes.len());
                println!("  Impacted nodes  : {}", impact.impacted_nodes.len());
                println!("  Impacted files  : {}", impact.impacted_files.len());
            }
        }
    }

    Ok(())
}

pub fn run_explain_change(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("explain-change");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        let db_path = db_path(cli, &repo);

        let (base, staged, explicit_files, max_depth, max_nodes, allow_stale, allow_partial) =
            match &cli.command {
                Command::ExplainChange {
                    base,
                    staged,
                    files,
                    max_depth,
                    max_nodes,
                    allow_stale,
                    allow_partial,
                } => (
                    base.clone(),
                    *staged,
                    files.clone(),
                    *max_depth,
                    *max_nodes as usize,
                    *allow_stale,
                    *allow_partial,
                ),
                _ => unreachable!(),
            };

        let changes = if !explicit_files.is_empty() {
            normalize_explicit_files(repo_root, &explicit_files)?
                .into_iter()
                .map(|path| atlas_core::model::ChangedFile {
                    path,
                    change_type: ChangeType::Modified,
                    old_path: None,
                })
                .collect()
        } else {
            changed_files(repo_root, &detect_changes_target(&base, staged))
                .context("cannot detect changed files")?
        };

        let target_files: Vec<String> = changes
            .iter()
            .filter(|change| change.change_type != ChangeType::Deleted)
            .map(|change| change.path.clone())
            .collect();

        if target_files.is_empty() {
            let empty = empty_explain_change_summary();
            if cli.json {
                print_json("explain_change", serde_json::to_value(&empty)?)?;
            } else {
                println!("No changed files detected.");
            }
            return Ok(());
        }

        let store = match Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &e.to_string());
                check_graph_readiness(
                    &readiness,
                    GraphToolRequirement::Analysis,
                    readiness_overrides(allow_stale, allow_partial),
                    "explain_change",
                    cli,
                )?;
                return Err(e).with_context(|| format!("cannot open database at {db_path}"));
            }
        };

        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(allow_stale, allow_partial),
            "explain_change",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }

        let policy = load_budget_policy(&repo)?;
        let summary = build_explain_change_summary(
            &store,
            &changes,
            &target_files,
            max_depth,
            max_nodes,
            &policy,
        )?;

        if cli.json {
            print_json("explain_change", serde_json::to_value(&summary)?)?;
        } else {
            println!("Risk level      : {}", summary.risk_level);
            println!("Changed files   : {}", summary.changed_file_count);
            println!("Changed symbols : {}", summary.changed_symbol_count);
            println!(
                "Diff summary    : +{} ~{} -{} r{}",
                summary.diff_summary.counts.added,
                summary.diff_summary.counts.modified,
                summary.diff_summary.counts.deleted,
                summary.diff_summary.counts.renamed
            );
            println!(
                "Change kinds    : api {} | signature {} | internal {}",
                summary.changed_by_kind.api_change,
                summary.changed_by_kind.signature_change,
                summary.changed_by_kind.internal_change
            );
            println!("Impacted files  : {}", summary.impacted_file_count);
            println!("Impacted nodes  : {}", summary.impacted_node_count);

            if !summary.changed_symbols.is_empty() {
                println!("\nChanged symbols:");
                for symbol in summary.changed_symbols.iter().take(20) {
                    println!(
                        "  [{}] {} {} ({}:{})",
                        symbol.change_kind, symbol.kind, symbol.qn, symbol.file, symbol.line
                    );
                }
            }

            if !summary.boundary_violations.is_empty() {
                println!("\nBoundary violations:");
                for violation in &summary.boundary_violations {
                    println!("  [{}] {}", violation.kind, violation.description);
                }
            }

            if !summary.impacted_components.is_empty() {
                println!("\nImpacted components:");
                for component in summary.impacted_components.iter().take(8) {
                    println!(
                        "  [{}] {} | changed {} | impacted {} | files {}",
                        component.kind,
                        component.label,
                        component.changed_node_count,
                        component.impacted_node_count,
                        component.file_count
                    );
                }
            }

            if !summary.call_chains.is_empty() {
                println!("\nCall chains:");
                for chain in summary.call_chains.iter().take(5) {
                    println!("  {}", chain.summary);
                }
            }

            if !summary.ripple_effects.is_empty() {
                println!("\nRipple effects:");
                for ripple in &summary.ripple_effects {
                    println!("  {ripple}");
                }
            }

            if summary.test_impact.affected_test_count > 0 {
                println!(
                    "\nAffected tests  : {}",
                    summary.test_impact.affected_test_count
                );
            }
            if summary.test_impact.uncovered_symbol_count > 0 {
                println!("Changed symbols without test coverage:");
                for symbol in &summary.test_impact.uncovered_symbols {
                    println!("  {symbol}");
                }
            }

            println!("\nSummary: {}", summary.summary);
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("explain-change", result.is_ok());
    }
    result
}

pub fn run_impact(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("impact");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        let db_path = db_path(cli, &repo);

        let store = match Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &e.to_string());
                check_graph_readiness(
                    &readiness,
                    GraphToolRequirement::Analysis,
                    readiness_overrides(false, false),
                    "impact",
                    cli,
                )?;
                return Err(e).with_context(|| format!("cannot open database at {db_path}"));
            }
        };
        let policy = load_budget_policy(&repo)?;

        let (
            base,
            explicit_files,
            max_depth,
            max_nodes,
            allow_stale,
            allow_partial,
            repo_id,
            all_repos,
        ) = match &cli.command {
            Command::Impact {
                base,
                files,
                max_depth,
                max_nodes,
                allow_stale,
                allow_partial,
                repo_id,
                all_repos,
            } => (
                base.clone(),
                files.clone(),
                *max_depth,
                *max_nodes as usize,
                *allow_stale,
                *allow_partial,
                repo_id.clone(),
                *all_repos,
            ),
            _ => unreachable!(),
        };

        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(allow_stale, allow_partial),
            "impact",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }

        let (selected_repos, excluded_manual_repo_count) =
            selected_impact_repos(repo_root, &repo_id, all_repos)?;
        let mut repo_warnings = Vec::new();
        let mut processed_repo_count = 0usize;
        let mut failed_repo_count = 0usize;
        let mut skipped_repo_count = 0usize;
        let target_files: Vec<(String, String)> = if selected_repos.is_empty() {
            let files = if !explicit_files.is_empty() {
                normalize_explicit_files(repo_root, &explicit_files)?
            } else {
                let diff_target = if let Some(base_ref) = &base {
                    DiffTarget::BaseRef(base_ref.clone())
                } else {
                    DiffTarget::WorkingTree
                };
                changed_files(repo_root, &diff_target)
                    .context("cannot detect changed files")?
                    .into_iter()
                    .filter(|cf| cf.change_type != ChangeType::Deleted)
                    .map(|cf| cf.path)
                    .collect()
            };
            let root_repo_id = stable_repo_id(repo_root);
            files
                .into_iter()
                .map(|path| (root_repo_id.clone(), path))
                .collect()
        } else {
            let diff_target = if let Some(base_ref) = &base {
                DiffTarget::BaseRef(base_ref.clone())
            } else {
                DiffTarget::WorkingTree
            };
            let mut combined = Vec::new();
            for registration in &selected_repos {
                if !registration.root.exists() {
                    failed_repo_count += 1;
                    skipped_repo_count += 1;
                    repo_warnings.push(format!(
                        "skipped repo {}: repo root missing",
                        registration.display_alias
                    ));
                    continue;
                }
                let repo_files = if !explicit_files.is_empty() {
                    match normalize_explicit_files(registration.root.as_path(), &explicit_files) {
                        Ok(files) => files,
                        Err(error) => {
                            failed_repo_count += 1;
                            skipped_repo_count += 1;
                            repo_warnings.push(format!(
                                "skipped repo {}: {}",
                                registration.display_alias, error
                            ));
                            continue;
                        }
                    }
                } else {
                    match changed_files(registration.root.as_path(), &diff_target) {
                        Ok(changes) => changes
                            .into_iter()
                            .filter(|cf| cf.change_type != ChangeType::Deleted)
                            .map(|cf| cf.path)
                            .collect(),
                        Err(error) => {
                            failed_repo_count += 1;
                            skipped_repo_count += 1;
                            repo_warnings.push(format!(
                                "skipped repo {}: {}",
                                registration.display_alias, error
                            ));
                            continue;
                        }
                    }
                };
                if repo_files.is_empty() {
                    skipped_repo_count += 1;
                    continue;
                }
                processed_repo_count += 1;
                combined.extend(
                    repo_files
                        .into_iter()
                        .map(|path| (registration.repo_id.clone(), path)),
                );
            }
            combined
        };

        if target_files.is_empty() {
            if cli.json {
                print_json(
                    "impact",
                    serde_json::json!({
                        "files": target_files,
                        "repo_scope": {
                            "selected_repo_count": selected_repos.len(),
                            "processed_repo_count": processed_repo_count,
                            "failed_repo_count": failed_repo_count,
                            "skipped_repo_count": skipped_repo_count,
                            "excluded_manual_repo_count": excluded_manual_repo_count,
                        },
                        "warnings": repo_warnings,
                        "analysis": ImpactResult {
                            changed_nodes: vec![],
                            impacted_nodes: vec![],
                            impacted_files: vec![],
                            relevant_edges: vec![],
                            seed_budgets: vec![],
                            traversal_budget: None,
                            budget: BudgetReport::not_applicable(),
                        }
                    }),
                )?;
            } else {
                println!("No changed files detected.");
                if excluded_manual_repo_count > 0 {
                    println!(
                        "Phase-1 rollout: excluded {} manual repo(s) from --all-repos. Use --repo-id to target them explicitly.",
                        excluded_manual_repo_count
                    );
                }
                for warning in &repo_warnings {
                    println!("warning\t{warning}");
                }
            }
            return Ok(());
        }

        let mut seed_qnames = Vec::new();
        for (seed_repo_id, file_path) in &target_files {
            seed_qnames.extend(impact_seed_qnames_for_repo(
                &store,
                seed_repo_id,
                std::slice::from_ref(file_path),
            )?);
        }
        seed_qnames.sort();
        seed_qnames.dedup();

        let t0 = std::time::Instant::now();
        let result = store
            .traverse_from_qnames(
                &seed_qnames.iter().map(String::as_str).collect::<Vec<_>>(),
                max_depth,
                max_nodes,
                policy.graph_traversal.edges.default_limit,
            )
            .context("impact radius query failed")?;
        let latency_ms = t0.elapsed().as_millis();

        let advanced = advanced_impact(result);
        let repo_aliases: std::collections::BTreeMap<String, String> = selected_repos
            .iter()
            .map(|entry| (entry.repo_id.clone(), entry.display_alias.clone()))
            .collect();

        if cli.json {
            print_json(
                "impact",
                serde_json::json!({
                    "files": target_files.iter().map(|(repo_id, path)| serde_json::json!({
                        "repo_id": repo_id,
                        "repo_alias": repo_aliases.get(repo_id).cloned().unwrap_or_else(|| repo_id.clone()),
                        "path": path,
                    })).collect::<Vec<_>>(),
                    "repo_scope": {
                        "selected_repo_count": selected_repos.len(),
                        "processed_repo_count": processed_repo_count,
                        "failed_repo_count": failed_repo_count,
                        "skipped_repo_count": skipped_repo_count,
                        "excluded_manual_repo_count": excluded_manual_repo_count,
                    },
                    "warnings": repo_warnings,
                    "analysis": advanced,
                }),
            )?;
        } else {
            if !selected_repos.is_empty() {
                println!(
                    "Repo scope    : selected={} processed={} failures={} skipped={}",
                    selected_repos.len(),
                    processed_repo_count,
                    failed_repo_count,
                    skipped_repo_count,
                );
            }
            println!("Changed files : {}", target_files.len());
            println!("Changed nodes : {}", advanced.base.changed_nodes.len());
            println!("Impacted nodes: {}", advanced.base.impacted_nodes.len());
            println!("Impacted files: {}", advanced.base.impacted_files.len());
            println!("Relevant edges: {}", advanced.base.relevant_edges.len());
            println!("Risk level    : {}", advanced.risk_level);
            println!("Latency       : {latency_ms}ms");
            if !advanced.base.impacted_files.is_empty() {
                println!("\nImpacted files:");
                for f in &advanced.base.impacted_files {
                    println!("  {f}");
                }
            }
            if !advanced.scored_nodes.is_empty() {
                println!("\nTop impacted nodes (by score):");
                for sn in advanced.scored_nodes.iter().take(20) {
                    let ck = sn
                        .change_kind
                        .map(|c| format!(" [{c}]"))
                        .unwrap_or_default();
                    let repo_suffix = node_repo_id(&sn.node)
                        .map(|repo_id| {
                            let label = repo_aliases
                                .get(repo_id)
                                .cloned()
                                .unwrap_or_else(|| repo_id.to_owned());
                            format!(" [repo {label}]")
                        })
                        .unwrap_or_default();
                    println!(
                        "  {:>6.2}  {} {}{}{}",
                        sn.impact_score,
                        sn.node.kind.as_str(),
                        sn.node.qualified_name,
                        ck,
                        repo_suffix
                    );
                }
            }
            if !advanced.test_impact.affected_tests.is_empty() {
                println!(
                    "\nAffected tests: {}",
                    advanced.test_impact.affected_tests.len()
                );
            }
            if !advanced.test_impact.uncovered_changed_nodes.is_empty() {
                println!("\nChanged nodes with no test coverage:");
                for n in &advanced.test_impact.uncovered_changed_nodes {
                    println!("  {} {}", n.kind.as_str(), n.qualified_name);
                }
            }
            if !advanced.boundary_violations.is_empty() {
                println!("\nBoundary violations:");
                for v in &advanced.boundary_violations {
                    println!("  [{}] {}", v.kind, v.description);
                }
            }
            if excluded_manual_repo_count > 0 {
                println!(
                    "\nPhase-1 rollout: excluded {} manual repo(s) from --all-repos. Use --repo-id to target them explicitly.",
                    excluded_manual_repo_count
                );
            }
            for warning in &repo_warnings {
                println!("warning\t{warning}");
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("impact", result.is_ok());
    }
    result
}

pub fn run_review_context(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("review-context");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        let db_path = db_path(cli, &repo);

        let store = match Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &e.to_string());
                check_graph_readiness(
                    &readiness,
                    GraphToolRequirement::Analysis,
                    readiness_overrides(false, false),
                    "review_context",
                    cli,
                )?;
                return Err(e).with_context(|| format!("cannot open database at {db_path}"));
            }
        };

        let (base, explicit_files, max_depth, max_nodes, format, allow_stale, allow_partial) =
            match &cli.command {
                Command::ReviewContext {
                    base,
                    files,
                    max_depth,
                    max_nodes,
                    format,
                    allow_stale,
                    allow_partial,
                } => (
                    base.clone(),
                    files.clone(),
                    *max_depth,
                    *max_nodes as usize,
                    *format,
                    *allow_stale,
                    *allow_partial,
                ),
                _ => unreachable!(),
            };

        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(allow_stale, allow_partial),
            "review_context",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }

        if cli.json && format == ReviewContextFormat::Markdown {
            anyhow::bail!("--format markdown cannot be combined with --json");
        }

        let target_files: Vec<String> = if !explicit_files.is_empty() {
            normalize_explicit_files(repo_root, &explicit_files)?
        } else {
            let diff_target = if let Some(base_ref) = &base {
                DiffTarget::BaseRef(base_ref.clone())
            } else {
                DiffTarget::WorkingTree
            };
            changed_files(repo_root, &diff_target)
                .context("cannot detect changed files")?
                .into_iter()
                .filter(|cf| cf.change_type != ChangeType::Deleted)
                .map(|cf| cf.path)
                .collect()
        };

        if target_files.is_empty() {
            if cli.json {
                let empty = ReviewContext {
                    changed_files: vec![],
                    changed_symbols: vec![],
                    changed_symbol_summaries: vec![],
                    impacted_neighbors: vec![],
                    critical_edges: vec![],
                    impact_overview: ReviewImpactOverview {
                        max_depth,
                        max_nodes,
                        impacted_node_count: 0,
                        impacted_file_count: 0,
                        relevant_edge_count: 0,
                        reached_node_limit: false,
                    },
                    risk_summary: RiskSummary {
                        changed_symbol_count: 0,
                        public_api_changes: 0,
                        test_adjacent: false,
                        affected_test_count: 0,
                        uncovered_changed_symbol_count: 0,
                        large_function_touched: false,
                        large_function_count: 0,
                        cross_module_impact: false,
                        cross_package_impact: false,
                        cross_repo_impact: false,
                    },
                };
                print_json(
                    "review_context",
                    serde_json::json!({
                        "files": target_files,
                        "review_context": empty,
                    }),
                )?;
            } else if format == ReviewContextFormat::Markdown {
                println!("## Atlas Review Context\n\nNo changed files detected.");
            } else {
                println!("No changed files detected.");
            }
            return Ok(());
        }

        let workflow_request = ContextRequest {
            intent: ContextIntent::Review,
            target: ContextTarget::ChangedFiles {
                paths: target_files.clone(),
            },
            max_nodes: Some(max_nodes),
            depth: Some(max_depth),
            ..ContextRequest::default()
        };
        let token_counter = load_token_counter(repo_root.as_str())?;
        let workflow_result = ContextEngine::new(&store)
            .with_budget_policy(load_budget_policy(repo_root.as_str())?)
            .with_token_counter(token_counter.counter)
            .with_token_fallback(token_counter.fallback_used, token_counter.fallback_reason)
            .build(&workflow_request)
            .context("context engine failed")?;

        if cli.json {
            let mut value = serde_json::to_value(&workflow_result)?;
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "context_ranking_evidence_legend".to_owned(),
                    atlas_core::context_ranking_evidence_legend(),
                );
            }
            print_json("review_context", value)?;
            return Ok(());
        }

        match format {
            ReviewContextFormat::Text => print_review_context_text(&workflow_result, &target_files),
            ReviewContextFormat::Markdown => {
                print!(
                    "{}",
                    render_review_context_markdown(&workflow_result, &target_files)
                );
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("review-context", result.is_ok());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_repo::{
        RepoRegistration, RepoRelationship, RepoRelationshipKind, TrustState, VcsMetadata,
        stable_repo_id,
    };
    use camino::{Utf8Path, Utf8PathBuf};

    fn registration(root: &Utf8Path, alias: &str, kind: RepoRelationshipKind) -> RepoRegistration {
        RepoRegistration {
            repo_id: stable_repo_id(root),
            root: root.to_path_buf(),
            display_alias: alias.to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind,
                parent_repo_id: None,
                parent_path: None,
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn selected_impact_repos_all_repos_excludes_manual_registrations() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let sub = root.join("submodule");
        let manual = root.join("../manual-sibling");
        let mut registry = RepoRegistry::new(stable_repo_id(root));
        registry.registrations = vec![
            registration(root, ".", RepoRelationshipKind::Root),
            registration(sub.as_path(), "submodule", RepoRelationshipKind::Submodule),
            registration(manual.as_path(), "manual", RepoRelationshipKind::Manual),
        ];
        registry.save(root).unwrap();

        let (selected, excluded_manual) = selected_impact_repos(root, &None, true).unwrap();

        assert_eq!(selected.len(), 2);
        assert_eq!(excluded_manual, 1);
        assert!(
            selected
                .iter()
                .all(|entry| entry.relationship.kind != RepoRelationshipKind::Manual)
        );
    }

    #[test]
    fn selected_impact_repos_rejects_excessive_all_repo_fanout() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let mut registry = RepoRegistry::new(stable_repo_id(root));
        registry.registrations = (0..(MAX_MULTI_REPO_SELECTION + 1))
            .map(|index| {
                let repo_root = Utf8PathBuf::from(format!("{}/repo-{index}", root.as_str()));
                registration(
                    repo_root.as_path(),
                    &format!("repo-{index}"),
                    RepoRelationshipKind::Submodule,
                )
            })
            .collect();
        registry.save(root).unwrap();

        let error = selected_impact_repos(root, &None, true).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("all_repos scope exceeds max supported repo fan-out")
        );
    }
}
