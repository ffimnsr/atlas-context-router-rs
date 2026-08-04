use super::*;

pub(super) fn retained_finding_ids<T>(report: &T) -> HashSet<String>
where
    T: ReportFindingView,
{
    report
        .findings()
        .iter()
        .map(|finding| finding.id.clone())
        .collect()
}

pub(super) trait ReportFindingView {
    fn findings(&self) -> &[InsightFinding];
}

impl ReportFindingView for PatternReport {
    fn findings(&self) -> &[InsightFinding] {
        &self.findings
    }
}

impl ReportFindingView for ArchitectureReport {
    fn findings(&self) -> &[InsightFinding] {
        &self.findings
    }
}

pub(super) fn similar_match_id(item: &SimilarFunctionMatch) -> String {
    format!(
        "similar_function:{}:{}",
        item.source.qualified_name, item.candidate.qualified_name
    )
}

pub(super) fn similar_match_to_finding(item: &SimilarFunctionMatch) -> InsightFinding {
    InsightFinding {
        id: similar_match_id(item),
        title: format!(
            "{} similar to {}",
            item.source.display_name, item.candidate.display_name
        ),
        severity: match item.score_band.as_str() {
            "high" => InsightSeverity::High,
            "medium" => InsightSeverity::Medium,
            _ => InsightSeverity::Low,
        },
        category: "similar_functions".to_owned(),
        message: format!(
            "{} and {} scored {:.2} similarity",
            item.source.qualified_name, item.candidate.qualified_name, item.score
        ),
        evidence: vec![
            symbol_evidence(&item.source),
            symbol_evidence(&item.candidate),
        ],
        ranking_reason: format!(
            "body/signature/name overlap produced {} similarity band",
            item.score_band
        ),
        details: Some(json!({
            "source": item.source,
            "candidate": item.candidate,
            "matched_features": item.matched_features,
            "differing_features": item.differing_features,
            "feature_scores": item.feature_scores,
        })),
        score: item.score * 100.0,
    }
}

pub(super) fn duplicate_group_id(group: &DuplicateGroup) -> String {
    group.group_id.clone()
}

pub(super) fn duplicate_group_to_finding(
    group: &DuplicateGroup,
    thresholds: &SimilarityThresholds,
) -> InsightFinding {
    InsightFinding {
        id: group.group_id.clone(),
        title: format!("{} duplicate group", group.duplicate_kind.replace('_', " ")),
        severity: duplicate_severity(group.confidence, thresholds),
        category: "duplicate_detection".to_owned(),
        message: format!(
            "{} members share {} duplicate pattern with confidence {:.2}",
            group.member_count, group.duplicate_kind, group.confidence
        ),
        evidence: group
            .members
            .iter()
            .map(|member| symbol_evidence(&member.symbol))
            .collect(),
        ranking_reason: format!(
            "confidence {:.2}, duplicated tokens {}, duplicated lines {}",
            group.confidence, group.duplicated_token_count, group.duplicated_line_count
        ),
        details: Some(json!({
            "duplicate_kind": group.duplicate_kind,
            "normalized_pattern_summary": group.normalized_pattern_summary,
            "files": group.files,
            "members": group.members,
            "suggested_extraction_target": group.suggested_extraction_target,
        })),
        score: group.confidence * 100.0,
    }
}

pub(super) fn duplicate_severity(
    confidence: f64,
    thresholds: &SimilarityThresholds,
) -> InsightSeverity {
    if confidence >= thresholds.high {
        InsightSeverity::High
    } else if confidence >= thresholds.medium {
        InsightSeverity::Medium
    } else {
        InsightSeverity::Low
    }
}

pub(super) fn similarity_thresholds(
    config: &atlas_engine::config::InsightsConfig,
) -> SimilarityThresholds {
    SimilarityThresholds {
        high: config.similarity_high_threshold,
        medium: config.similarity_medium_threshold,
        low: config.similarity_low_threshold,
    }
}

pub(super) fn duplicate_thresholds(
    config: &atlas_engine::config::InsightsConfig,
) -> SimilarityThresholds {
    SimilarityThresholds {
        high: config.duplicate_high_threshold,
        medium: config.duplicate_medium_threshold,
        low: config.duplicate_low_threshold,
    }
}

pub(super) fn duplicate_suppressions(
    config: &atlas_engine::config::InsightsConfig,
    request: &DuplicateDetectionRequest,
) -> BTreeSet<String> {
    config
        .duplicate_suppressions
        .iter()
        .chain(request.suppressions.iter())
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

pub(super) fn duplicate_group_suppressed(
    group: &DuplicateGroup,
    suppressions: &BTreeSet<String>,
) -> bool {
    suppressions.iter().any(|pattern| {
        group.group_id.contains(pattern)
            || group.normalized_pattern_summary.contains(pattern)
            || group
                .files
                .iter()
                .any(|file| file == pattern || file.starts_with(pattern))
            || group.members.iter().any(|member| {
                member.symbol.qualified_name.contains(pattern)
                    || member.symbol.file_path == *pattern
                    || member.symbol.file_path.starts_with(pattern)
            })
    })
}

pub(super) fn symbol_evidence(summary: &InsightSymbolSummary) -> InsightEvidence {
    InsightEvidence {
        file_path: Some(summary.file_path.clone()),
        qualified_name: Some(summary.qualified_name.clone()),
        node_kind: Some(summary.node_kind.clone()),
        edge_kind: None,
        line_range: Some(InsightLineRange {
            start_line: summary.line_start,
            end_line: summary.line_end,
        }),
        confidence_tier: None,
    }
}

pub(super) fn similarity_band(score: f64, thresholds: &SimilarityThresholds) -> &'static str {
    if score >= thresholds.high {
        "high"
    } else if score >= thresholds.medium {
        "medium"
    } else {
        "low"
    }
}
