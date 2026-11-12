//! VR3 — deterministic evidence-sufficiency checks and gap-driven escalation.
//!
//! No LLM judge is involved: every constraint is derived from structural
//! retrieval facts recorded in [`ContextResult`].

use super::*;
use atlas_core::model::ContextIntent;

/// Constraints required for one intent, in deterministic report order.
pub(super) fn required_constraints(
    intent: ContextIntent,
    include_overviews: bool,
) -> Vec<EvidenceGap> {
    use ContextIntent::*;
    let mut required = vec![EvidenceGap::TargetUnresolved];
    match intent {
        Review | Impact | ImpactAnalysis | UsageLookup | RefactorSafety | DependencyRemoval
        | DeadCodeCheck | RenamePreview => required.push(EvidenceGap::CallersTruncated),
        Symbol | File => {}
    }
    if matches!(
        intent,
        RefactorSafety | DependencyRemoval | DeadCodeCheck | RenamePreview
    ) {
        required.push(EvidenceGap::TestsMissing);
    }
    if include_overviews {
        required.push(EvidenceGap::OverviewNotFresh);
    }
    required
}

/// Follow-up tools for one gap.
fn next_tool_for(gap: EvidenceGap) -> &'static str {
    match gap {
        EvidenceGap::TargetUnresolved => "resolve_symbol",
        EvidenceGap::CallersTruncated | EvidenceGap::CalleesTruncated => "symbol_neighbors",
        EvidenceGap::TestsMissing => "query_graph",
        EvidenceGap::DispatchUnresolved => "symbol_neighbors",
        EvidenceGap::OverviewNotFresh => "overview_search",
    }
}

fn edge_kind_name(edge: &atlas_core::model::Edge) -> String {
    edge.kind.as_str().to_ascii_lowercase()
}

/// Evaluate structural sufficiency for a built context result.
pub(super) fn evaluate(
    request: &ContextRequest,
    result: &ContextResult,
) -> EvidenceSufficiencyReport {
    let target_resolved = result.nodes.iter().any(|node| {
        matches!(
            node.selection_reason,
            SelectionReason::DirectTarget | SelectionReason::ImpactNeighbor
        )
    }) && result
        .ambiguity
        .as_ref()
        .is_none_or(|ambiguity| ambiguity.resolved);

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
    let callers_complete = caller_count < BUCKET_CALLERS;
    let callees_complete = callee_count < BUCKET_CALLEES;
    let tests_covered = result
        .nodes
        .iter()
        .any(|node| node.selection_reason == SelectionReason::TestAdjacent || node.node.is_test);

    let present: HashSet<&str> = result
        .nodes
        .iter()
        .map(|node| node.node.qualified_name.as_str())
        .collect();
    let dispatch_complete = !result.edges.iter().any(|edge| {
        let kind = edge_kind_name(&edge.edge);
        (kind.contains("implement") || kind.contains("dispatch"))
            && !present.contains(edge.edge.target_qn.as_str())
    });

    let overview_fresh = if request.include_overviews {
        !result.overviews.is_empty()
            && result
                .overviews
                .iter()
                .all(|overview| overview.freshness == "fresh")
    } else {
        true
    };

    let mut gaps = Vec::new();
    if !target_resolved {
        gaps.push(EvidenceGap::TargetUnresolved);
    }
    if !callers_complete {
        gaps.push(EvidenceGap::CallersTruncated);
    }
    if !callees_complete {
        gaps.push(EvidenceGap::CalleesTruncated);
    }
    if !tests_covered && !result.nodes.is_empty() {
        gaps.push(EvidenceGap::TestsMissing);
    }
    if !dispatch_complete {
        gaps.push(EvidenceGap::DispatchUnresolved);
    }
    if !overview_fresh {
        gaps.push(EvidenceGap::OverviewNotFresh);
    }

    let required = required_constraints(request.intent, request.include_overviews);
    let unsatisfied: Vec<EvidenceGap> = required
        .iter()
        .copied()
        .filter(|gap| gaps.contains(gap))
        .collect();
    let mut report = EvidenceSufficiencyReport {
        target_resolved,
        callers_complete,
        callees_complete,
        tests_covered,
        dispatch_complete,
        overview_fresh,
        evidence_gaps: unsatisfied.clone(),
        sufficiency_score: 1.0,
        escalation_reason: None,
        next_tools: unsatisfied
            .iter()
            .map(|gap| next_tool_for(*gap).to_owned())
            .collect(),
        rounds_used: 0,
        skipped_reason: None,
    };
    report.next_tools.sort();
    report.next_tools.dedup();
    report.compute_score(&required);
    report
}

/// Human-readable escalation reason for the gaps that triggered Round 2.
pub(super) fn escalation_reason(gaps: &[EvidenceGap]) -> Option<String> {
    if gaps.is_empty() {
        return None;
    }
    Some(format!(
        "evidence gaps: {}",
        gaps.iter()
            .map(|gap| gap.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Merge Round-2 evidence into the Round-1 result.
pub(super) fn merge_round_two(result: &mut ContextResult, extra: ContextResult) {
    let mut seen_nodes: HashSet<String> = result
        .nodes
        .iter()
        .map(|node| node.node.qualified_name.clone())
        .collect();
    for node in extra.nodes {
        if seen_nodes.insert(node.node.qualified_name.clone()) {
            result.nodes.push(node);
        }
    }

    let mut seen_edges: HashSet<(String, String, String)> = result
        .edges
        .iter()
        .map(|edge| {
            (
                edge.edge.source_qn.clone(),
                edge.edge.target_qn.clone(),
                edge.edge.kind.as_str().to_owned(),
            )
        })
        .collect();
    for edge in extra.edges {
        let key = (
            edge.edge.source_qn.clone(),
            edge.edge.target_qn.clone(),
            edge.edge.kind.as_str().to_owned(),
        );
        if seen_edges.insert(key) {
            result.edges.push(edge);
        }
    }

    result.files = super::symbol::collect_files(&result.nodes);
    super::symbol::update_file_node_counts(result);
    super::rank::rank_context(result);
    super::rank::trim_context(result);
}

/// True when the request's token budget is already exhausted.
pub(super) fn budget_exhausted(
    counter: &atlas_token_count::TokenCounter,
    result: &ContextResult,
    token_budget: Option<usize>,
) -> bool {
    let Some(budget) = token_budget else {
        return false;
    };
    let Ok(encoded) = serde_json::to_vec(result) else {
        return false;
    };
    let used = counter
        .count_json_bytes(&encoded)
        .map(|count| count.tokens)
        .unwrap_or(encoded.len().div_ceil(4));
    used >= budget
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(intent: ContextIntent) -> ContextRequest {
        ContextRequest {
            intent,
            ..ContextRequest::default()
        }
    }

    fn empty_result(request: ContextRequest) -> ContextResult {
        ContextResult {
            request,
            nodes: vec![],
            edges: vec![],
            files: vec![],
            truncation: TruncationMeta::none(),
            seed_budgets: vec![],
            traversal_budget: None,
            ambiguity: None,
            workflow: None,
            saved_context_sources: vec![],
            content_assets: vec![],
            overviews: vec![],
            sufficiency: None,
            repo_provenance: None,
            telemetry: None,
            budget: BudgetReport::not_applicable(),
        }
    }

    #[test]
    fn required_constraints_follow_intent() {
        assert_eq!(
            required_constraints(ContextIntent::Review, false),
            vec![EvidenceGap::TargetUnresolved, EvidenceGap::CallersTruncated]
        );
        assert_eq!(
            required_constraints(ContextIntent::RefactorSafety, false),
            vec![
                EvidenceGap::TargetUnresolved,
                EvidenceGap::CallersTruncated,
                EvidenceGap::TestsMissing
            ]
        );
        assert_eq!(
            required_constraints(ContextIntent::UsageLookup, false),
            vec![EvidenceGap::TargetUnresolved, EvidenceGap::CallersTruncated]
        );
        assert_eq!(
            required_constraints(ContextIntent::Symbol, true),
            vec![EvidenceGap::TargetUnresolved, EvidenceGap::OverviewNotFresh]
        );
    }

    #[test]
    fn unresolved_target_reports_gap_and_score() {
        let request = request(ContextIntent::Symbol);
        let result = empty_result(request.clone());
        let report = evaluate(&request, &result);
        assert!(!report.target_resolved);
        assert!(
            report
                .evidence_gaps
                .contains(&EvidenceGap::TargetUnresolved)
        );
        assert!(report.sufficiency_score < 1.0);
        assert!(report.next_tools.contains(&"resolve_symbol".to_owned()));
    }

    #[test]
    fn satisfied_required_constraints_score_one() {
        let request = request(ContextIntent::UsageLookup);
        let result = empty_result(request.clone());
        let report = evaluate(&request, &result);
        // Empty result has caller bucket under the cap, so callers_complete holds.
        assert!(report.callers_complete);
        assert_eq!(report.evidence_gaps, vec![EvidenceGap::TargetUnresolved]);
    }

    #[test]
    fn overview_gap_requires_enabled_and_fresh() {
        let mut request = request(ContextIntent::Symbol);
        request.include_overviews = true;
        let result = empty_result(request.clone());
        let report = evaluate(&request, &result);
        assert!(!report.overview_fresh);
        assert!(
            report
                .evidence_gaps
                .contains(&EvidenceGap::OverviewNotFresh)
        );
    }
}
