use super::*;

/// Assign an overall risk level for the change set.
pub(crate) fn compute_risk_level(
    base: &ImpactResult,
    scored: &[ScoredImpactNode],
    test_impact: &TestImpactResult,
    violations: &[BoundaryViolation],
) -> RiskLevel {
    let has_api_change = scored
        .iter()
        .any(|s| s.change_kind == Some(ChangeKind::ApiChange));
    let has_sig_change = scored
        .iter()
        .any(|s| s.change_kind == Some(ChangeKind::SignatureChange));
    let has_cross_module = violations
        .iter()
        .any(|v| v.kind == BoundaryKind::CrossModule);
    let has_cross_package = violations
        .iter()
        .any(|v| v.kind == BoundaryKind::CrossPackage);
    let has_cross_repo = violations.iter().any(|v| v.kind == BoundaryKind::CrossRepo);
    let impacted_count = base.impacted_nodes.len();
    let uncovered = !test_impact.uncovered_changed_nodes.is_empty();

    if has_api_change && (has_cross_repo || has_cross_package || has_cross_module) {
        RiskLevel::Critical
    } else if has_cross_repo
        || has_api_change
        || (has_cross_package && (uncovered || impacted_count > 5))
    {
        RiskLevel::High
    } else if has_sig_change || has_cross_module || (impacted_count > 20) || uncovered {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
