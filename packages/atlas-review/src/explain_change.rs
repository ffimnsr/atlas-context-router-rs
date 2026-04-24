use atlas_core::model::{
    ChangeType, ChangedFile, ContextIntent, ContextRequest, ContextTarget, NoiseReductionSummary,
    WorkflowCallChain, WorkflowComponent, WorkflowFocusNode,
};
use atlas_core::{BudgetPolicy, BudgetReport, ImpactResult, Result};
use atlas_impact::analyze as advanced_impact;
use atlas_store_sqlite::Store;
use serde::Serialize;

use crate::ContextEngine;

#[derive(Debug, Clone, Serialize, Default)]
pub struct ExplainChangedByKind {
    pub api_change: usize,
    pub signature_change: usize,
    pub internal_change: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExplainChangedSymbol {
    pub qn: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub change_kind: String,
    pub lang: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExplainBoundaryViolation {
    pub kind: String,
    pub description: String,
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExplainTestImpact {
    pub affected_test_count: usize,
    pub uncovered_symbol_count: usize,
    pub uncovered_symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ExplainDiffCounts {
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub copied: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExplainDiffFile {
    pub path: String,
    pub change_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub changed_symbol_count: usize,
    pub impacted_symbol_count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ExplainDiffSummary {
    pub counts: ExplainDiffCounts,
    pub files: Vec<ExplainDiffFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExplainChangeSummary {
    pub risk_level: String,
    pub changed_file_count: usize,
    pub changed_symbol_count: usize,
    pub changed_by_kind: ExplainChangedByKind,
    pub diff_summary: ExplainDiffSummary,
    pub changed_symbols: Vec<ExplainChangedSymbol>,
    pub impacted_file_count: usize,
    pub impacted_node_count: usize,
    pub high_impact_nodes: Vec<WorkflowFocusNode>,
    pub impacted_components: Vec<WorkflowComponent>,
    pub call_chains: Vec<WorkflowCallChain>,
    pub ripple_effects: Vec<String>,
    pub boundary_violations: Vec<ExplainBoundaryViolation>,
    pub test_impact: ExplainTestImpact,
    pub noise_reduction: NoiseReductionSummary,
    pub summary: String,
    #[serde(flatten)]
    pub budget: BudgetReport,
}

pub fn empty_explain_change_summary() -> ExplainChangeSummary {
    ExplainChangeSummary {
        risk_level: "low".to_string(),
        changed_file_count: 0,
        changed_symbol_count: 0,
        changed_by_kind: ExplainChangedByKind::default(),
        diff_summary: ExplainDiffSummary::default(),
        changed_symbols: vec![],
        impacted_file_count: 0,
        impacted_node_count: 0,
        high_impact_nodes: vec![],
        impacted_components: vec![],
        call_chains: vec![],
        ripple_effects: vec![],
        boundary_violations: vec![],
        test_impact: ExplainTestImpact {
            affected_test_count: 0,
            uncovered_symbol_count: 0,
            uncovered_symbols: vec![],
        },
        noise_reduction: NoiseReductionSummary {
            retained_nodes: 0,
            retained_edges: 0,
            retained_files: 0,
            dropped_nodes: 0,
            dropped_edges: 0,
            dropped_files: 0,
            rules_applied: vec![],
        },
        summary: "No changed files detected.".to_string(),
        budget: BudgetReport::not_applicable(),
    }
}

pub fn build_explain_change_summary(
    store: &Store,
    changes: &[ChangedFile],
    files: &[String],
    max_depth: u32,
    max_nodes: usize,
    policy: &BudgetPolicy,
) -> Result<ExplainChangeSummary> {
    let file_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let base_impact = store.impact_radius(
        &file_refs,
        max_depth,
        max_nodes,
        policy.graph_traversal.edges.default_limit,
    )?;
    let advanced = advanced_impact(base_impact);
    let workflow_request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: files.to_vec(),
        },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        ..ContextRequest::default()
    };
    let workflow_result = ContextEngine::new(store)
        .with_budget_policy(*policy)
        .build(&workflow_request)?;
    let workflow = workflow_result.workflow.clone();

    let mut changed_by_kind = ExplainChangedByKind::default();

    let changed_symbols: Vec<ExplainChangedSymbol> = advanced
        .scored_nodes
        .iter()
        .filter_map(|scored| {
            scored
                .change_kind
                .map(|change_kind| (&scored.node, change_kind))
        })
        .map(|(node, change_kind)| {
            let change_kind = match change_kind {
                atlas_core::ChangeKind::ApiChange => {
                    changed_by_kind.api_change += 1;
                    "api_change"
                }
                atlas_core::ChangeKind::SignatureChange => {
                    changed_by_kind.signature_change += 1;
                    "signature_change"
                }
                atlas_core::ChangeKind::InternalChange => {
                    changed_by_kind.internal_change += 1;
                    "internal_change"
                }
            };

            ExplainChangedSymbol {
                qn: node.qualified_name.clone(),
                kind: node.kind.as_str().to_string(),
                file: node.file_path.clone(),
                line: node.line_start,
                change_kind: change_kind.to_string(),
                lang: node.language.clone(),
                sig: node.params.clone(),
            }
        })
        .collect();

    let boundary_violations: Vec<ExplainBoundaryViolation> = advanced
        .boundary_violations
        .iter()
        .map(|violation| ExplainBoundaryViolation {
            kind: match violation.kind {
                atlas_core::BoundaryKind::CrossModule => "cross_module",
                atlas_core::BoundaryKind::CrossPackage => "cross_package",
            }
            .to_string(),
            description: violation.description.clone(),
            nodes: violation.nodes.clone(),
        })
        .collect();

    let uncovered_symbols: Vec<String> = advanced
        .test_impact
        .uncovered_changed_nodes
        .iter()
        .map(|node| node.qualified_name.clone())
        .collect();

    let risk_level = advanced.risk_level.to_string();
    let impacted_file_count = advanced.base.impacted_files.len();
    let impacted_node_count = advanced.base.impacted_nodes.len();
    let diff_summary = build_diff_summary(changes, &advanced.base);

    let mut summary_parts = vec![format!("Risk: {}.", risk_level)];
    summary_parts.push(format!(
        "{} file change(s): {} modified, {} added, {} deleted, {} renamed.",
        changes.len(),
        diff_summary.counts.modified,
        diff_summary.counts.added,
        diff_summary.counts.deleted,
        diff_summary.counts.renamed,
    ));
    if changed_by_kind.api_change > 0 {
        summary_parts.push(format!("{} api change(s).", changed_by_kind.api_change));
    }
    if changed_by_kind.signature_change > 0 {
        summary_parts.push(format!(
            "{} signature change(s).",
            changed_by_kind.signature_change
        ));
    }
    if changed_by_kind.internal_change > 0 {
        summary_parts.push(format!(
            "{} internal change(s).",
            changed_by_kind.internal_change
        ));
    }
    summary_parts.push(format!(
        "Affects {} file(s), {} node(s).",
        impacted_file_count, impacted_node_count
    ));
    if !boundary_violations.is_empty() {
        summary_parts.push(format!(
            "{} boundary violation(s).",
            boundary_violations.len()
        ));
    }
    if !uncovered_symbols.is_empty() {
        summary_parts.push(format!(
            "{} changed symbol(s) lack test coverage.",
            uncovered_symbols.len()
        ));
    }
    if let Some(workflow) = &workflow {
        if let Some(headline) = &workflow.headline {
            summary_parts.push(headline.clone());
        }
        if let Some(ripple) = workflow.ripple_effects.first() {
            summary_parts.push(ripple.clone());
        }
    }

    Ok(ExplainChangeSummary {
        risk_level,
        changed_file_count: changes.len(),
        changed_symbol_count: changed_symbols.len(),
        changed_by_kind,
        diff_summary,
        changed_symbols,
        impacted_file_count,
        impacted_node_count,
        high_impact_nodes: workflow
            .as_ref()
            .map(|workflow| workflow.high_impact_nodes.clone())
            .unwrap_or_default(),
        impacted_components: workflow
            .as_ref()
            .map(|workflow| workflow.impacted_components.clone())
            .unwrap_or_default(),
        call_chains: workflow
            .as_ref()
            .map(|workflow| workflow.call_chains.clone())
            .unwrap_or_default(),
        ripple_effects: workflow
            .as_ref()
            .map(|workflow| workflow.ripple_effects.clone())
            .unwrap_or_default(),
        boundary_violations,
        test_impact: ExplainTestImpact {
            affected_test_count: advanced.test_impact.affected_tests.len(),
            uncovered_symbol_count: uncovered_symbols.len(),
            uncovered_symbols,
        },
        noise_reduction: workflow.map(|workflow| workflow.noise_reduction).unwrap_or(
            NoiseReductionSummary {
                retained_nodes: 0,
                retained_edges: 0,
                retained_files: 0,
                dropped_nodes: 0,
                dropped_edges: 0,
                dropped_files: 0,
                rules_applied: vec![],
            },
        ),
        summary: summary_parts.join(" "),
        budget: workflow_result.budget.clone(),
    })
}

fn build_diff_summary(changes: &[ChangedFile], impact: &ImpactResult) -> ExplainDiffSummary {
    let changed_by_file =
        impact
            .changed_nodes
            .iter()
            .fold(std::collections::HashMap::new(), |mut acc, node| {
                *acc.entry(node.file_path.as_str()).or_insert(0) += 1;
                acc
            });
    let impacted_by_file =
        impact
            .impacted_nodes
            .iter()
            .fold(std::collections::HashMap::new(), |mut acc, node| {
                *acc.entry(node.file_path.as_str()).or_insert(0) += 1;
                acc
            });

    let mut counts = ExplainDiffCounts::default();
    let files = changes
        .iter()
        .map(|change| {
            match change.change_type {
                ChangeType::Added => counts.added += 1,
                ChangeType::Modified => counts.modified += 1,
                ChangeType::Deleted => counts.deleted += 1,
                ChangeType::Renamed => counts.renamed += 1,
                ChangeType::Copied => counts.copied += 1,
            }

            ExplainDiffFile {
                path: change.path.clone(),
                change_type: match change.change_type {
                    ChangeType::Added => "added",
                    ChangeType::Modified => "modified",
                    ChangeType::Deleted => "deleted",
                    ChangeType::Renamed => "renamed",
                    ChangeType::Copied => "copied",
                }
                .to_string(),
                old_path: change.old_path.clone(),
                changed_symbol_count: changed_by_file
                    .get(change.path.as_str())
                    .copied()
                    .unwrap_or(0),
                impacted_symbol_count: impacted_by_file
                    .get(change.path.as_str())
                    .copied()
                    .unwrap_or(0),
            }
        })
        .collect();

    ExplainDiffSummary { counts, files }
}
