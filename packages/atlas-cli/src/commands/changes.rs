use crate::cli::{Cli, Command, ReviewContextFormat};
use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_core::BudgetReport;
use atlas_core::GraphToolRequirement;
use atlas_core::model::{
    ChangeType, ContextIntent, ContextRequest, ContextResult, ContextTarget, ImpactResult,
    SelectionReason,
};
use atlas_impact::analyze as advanced_impact;
use atlas_repo::{
    DiffTarget, RepoRegistry, changed_files, find_repo_root, phase1_multi_repo_supported,
    stable_repo_id,
};
use atlas_review::{
    ContextEngine, build_explain_change_summary, empty_explain_change_summary,
    explain_change_wire_payload,
};
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
            atlas_repo::normalize_repo_change_path(repo_root, path)
                .map(|resolved| resolved.canonical)
                .with_context(|| format!("invalid explicit file path '{path}'"))
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

mod detect;
mod explain;
mod impact;
mod review;

pub use self::detect::run_detect_changes;
pub use self::explain::run_explain_change;
pub use self::impact::run_impact;
pub use self::review::run_review_context;
