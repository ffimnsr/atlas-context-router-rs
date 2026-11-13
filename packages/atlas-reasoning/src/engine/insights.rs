use std::collections::BTreeMap;

use atlas_core::{
    ArchitectureReport, FreshnessWarning, GraphStats, InsightFinding, InsightSummary,
    LargeFunctionReport, MetricsReport, PatternReport, ProvenanceMeta, RiskReport, format_rfc3339,
    now_utc,
};
use atlas_store_sqlite::Store;

use crate::ranking::{sort_insight_findings, trim_insight_findings};

#[derive(Debug, Clone)]
pub struct InsightsGraphSummary {
    pub graph_stats: GraphStats,
    pub atlas_provenance: ProvenanceMeta,
    pub atlas_freshness: Option<FreshnessWarning>,
}

pub struct InsightsEngine<'s> {
    store: Option<&'s Store>,
    summary: InsightsGraphSummary,
    config: atlas_engine::config::InsightsConfig,
    pub(super) ignore_files_glob: Option<globset::GlobSet>,
    generated_at: String,
}

impl<'s> InsightsEngine<'s> {
    pub fn new(
        store: &'s Store,
        config: atlas_engine::config::InsightsConfig,
    ) -> atlas_core::Result<Self> {
        Ok(Self {
            store: Some(store),
            summary: InsightsGraphSummary {
                graph_stats: store.stats()?,
                atlas_provenance: store.provenance_meta()?,
                atlas_freshness: None,
            },
            ignore_files_glob: build_ignore_files_glob(&config.ignore_files),
            config,
            generated_at: format_rfc3339(now_utc()),
        })
    }

    pub fn from_summary(
        summary: InsightsGraphSummary,
        config: atlas_engine::config::InsightsConfig,
    ) -> Self {
        Self {
            store: None,
            summary,
            ignore_files_glob: build_ignore_files_glob(&config.ignore_files),
            config,
            generated_at: format_rfc3339(now_utc()),
        }
    }

    pub fn with_generated_at(mut self, generated_at: impl Into<String>) -> Self {
        self.generated_at = generated_at.into();
        self
    }

    pub fn graph_stats(&self) -> &GraphStats {
        &self.summary.graph_stats
    }

    pub fn atlas_provenance(&self) -> &ProvenanceMeta {
        &self.summary.atlas_provenance
    }

    pub fn atlas_freshness(&self) -> Option<&FreshnessWarning> {
        self.summary.atlas_freshness.as_ref()
    }

    pub fn store(&self) -> Option<&Store> {
        self.store
    }

    pub fn config(&self) -> &atlas_engine::config::InsightsConfig {
        &self.config
    }

    pub fn architecture_report(&self, findings: Vec<InsightFinding>) -> ArchitectureReport {
        let findings = self.prepare_architecture_findings(findings);
        ArchitectureReport {
            summary: InsightSummary::from_findings(&findings, self.generated_at.clone()),
            findings,
            atlas_provenance: self.summary.atlas_provenance.clone(),
            atlas_freshness: self.summary.atlas_freshness.clone(),
        }
    }

    pub fn metrics_report(&self, findings: Vec<InsightFinding>) -> MetricsReport {
        let findings = self.prepare_findings(findings);
        MetricsReport {
            summary: InsightSummary::from_findings(&findings, self.generated_at.clone()),
            findings,
            atlas_provenance: self.summary.atlas_provenance.clone(),
            atlas_freshness: self.summary.atlas_freshness.clone(),
        }
    }

    pub fn risk_report(&self, findings: Vec<InsightFinding>) -> RiskReport {
        let findings = self.prepare_findings(findings);
        RiskReport {
            summary: InsightSummary::from_findings(&findings, self.generated_at.clone()),
            findings,
            atlas_provenance: self.summary.atlas_provenance.clone(),
            atlas_freshness: self.summary.atlas_freshness.clone(),
        }
    }

    pub fn pattern_report(&self, findings: Vec<InsightFinding>) -> PatternReport {
        self.pattern_report_with_limit(findings, None)
    }

    /// Pattern report whose per-category fair share is derived from the caller's
    /// requested page size, not only the config-wide `max_findings` ceiling.
    ///
    /// Without this, a `limit: 3` request still caps each category at
    /// `max_findings / 4` and the first page is dominated by repeated-chain
    /// findings.
    pub fn pattern_report_with_limit(
        &self,
        findings: Vec<InsightFinding>,
        limit: Option<usize>,
    ) -> PatternReport {
        let findings = self.prepare_pattern_findings(findings, limit);
        PatternReport {
            summary: InsightSummary::from_findings(&findings, self.generated_at.clone()),
            findings,
            atlas_provenance: self.summary.atlas_provenance.clone(),
            atlas_freshness: self.summary.atlas_freshness.clone(),
        }
    }

    pub fn large_function_report(&self, findings: Vec<InsightFinding>) -> LargeFunctionReport {
        let findings = self.prepare_findings_preserving_order(findings);
        LargeFunctionReport {
            summary: InsightSummary::from_findings(&findings, self.generated_at.clone()),
            findings,
            atlas_provenance: self.summary.atlas_provenance.clone(),
            atlas_freshness: self.summary.atlas_freshness.clone(),
        }
    }

    fn prepare_findings(&self, findings: Vec<InsightFinding>) -> Vec<InsightFinding> {
        let mut findings = findings
            .into_iter()
            .filter(|finding| !self.is_ignored_finding(finding))
            .collect::<Vec<_>>();
        sort_insight_findings(&mut findings);
        trim_insight_findings(&mut findings, self.config.max_findings);
        findings
    }

    /// Pattern findings need category diversity: repeated-call-chain rows carry
    /// scores proportional to occurrence count and otherwise crowd hub,
    /// bottleneck, isolated-structure, orphan, and deep-chain findings out of
    /// every score-ranked trim. Cap each category to a fair share, then fill any
    /// remaining slots from the already-sorted overflow.
    fn prepare_pattern_findings(
        &self,
        findings: Vec<InsightFinding>,
        requested_limit: Option<usize>,
    ) -> Vec<InsightFinding> {
        let mut findings = findings
            .into_iter()
            .filter(|finding| !self.is_ignored_finding(finding))
            .collect::<Vec<_>>();
        sort_insight_findings(&mut findings);

        // Fair-share against the page the caller will actually see, capped by
        // the configured ceiling. `max_findings / 4` alone makes small pages
        // score-dominated by one category.
        let effective_limit = requested_limit
            .unwrap_or(self.config.max_findings)
            .clamp(1, self.config.max_findings.max(1));
        let per_category = (effective_limit / 4).max(1);
        let mut counts = BTreeMap::<String, usize>::new();
        let mut kept = Vec::with_capacity(findings.len().min(self.config.max_findings));
        let mut deferred = Vec::new();
        for finding in findings {
            let count = counts.entry(finding.category.clone()).or_insert(0);
            if *count < per_category {
                *count += 1;
                kept.push(finding);
            } else {
                deferred.push(finding);
            }
        }
        let remaining = self.config.max_findings.saturating_sub(kept.len());
        kept.extend(deferred.into_iter().take(remaining));
        trim_insight_findings(&mut kept, self.config.max_findings);
        kept
    }

    /// Architecture findings rank by signal class before generic
    /// severity/score. File hotspots can carry a much larger raw score than a
    /// high-coupling module, so a purely score-ranked trim can evict every
    /// structural finding. The stable re-sort keeps severity order inside each
    /// class.
    fn prepare_architecture_findings(&self, findings: Vec<InsightFinding>) -> Vec<InsightFinding> {
        let mut findings = findings
            .into_iter()
            .filter(|finding| !self.is_ignored_finding(finding))
            .collect::<Vec<_>>();
        sort_insight_findings(&mut findings);
        findings.sort_by_key(architecture_signal_rank);
        trim_insight_findings(&mut findings, self.config.max_findings);
        findings
    }

    fn prepare_findings_preserving_order(
        &self,
        findings: Vec<InsightFinding>,
    ) -> Vec<InsightFinding> {
        let mut findings = findings
            .into_iter()
            .filter(|finding| !self.is_ignored_finding(finding))
            .collect::<Vec<_>>();
        trim_insight_findings(&mut findings, self.config.max_findings);
        findings
    }

    fn is_ignored_finding(&self, finding: &InsightFinding) -> bool {
        finding.evidence.iter().any(|evidence| {
            evidence.file_path.as_deref().is_some_and(|path| {
                path_matches_ignore_list(
                    path,
                    &self.config.ignore_files,
                    self.ignore_files_glob.as_ref(),
                )
            }) || evidence
                .qualified_name
                .as_deref()
                .is_some_and(|qname| module_matches_any(qname, &self.config.ignore_modules))
                || evidence.node_kind.as_deref().is_some_and(|kind| {
                    self.config
                        .ignore_node_kinds
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(kind))
                })
        })
    }
}

/// Ranking band for architecture findings: rarer structural signals first.
fn architecture_signal_rank(finding: &InsightFinding) -> u8 {
    match finding.category.as_str() {
        "architecture_cycle" => 0,
        "layer_violation" => 1,
        "architecture_module_health" => 2,
        "architecture_cluster" => 3,
        "architecture_file_health" => 4,
        _ => 5,
    }
}

fn has_glob_meta(pattern: &str) -> bool {
    pattern
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'['))
}

/// Compile glob-style `ignore_files` entries (`*.md`, `**/fixtures/*.json`)
/// into a matcher. Exact/prefix entries keep their legacy semantics; invalid
/// globs are skipped here (config validation reports them at load time).
pub(super) fn build_ignore_files_glob(patterns: &[String]) -> Option<globset::GlobSet> {
    let mut builder = globset::GlobSetBuilder::new();
    let mut any = false;
    for pattern in patterns {
        if has_glob_meta(pattern)
            && let Ok(glob) = globset::Glob::new(pattern)
        {
            builder.add(glob);
            any = true;
        }
    }
    if any { builder.build().ok() } else { None }
}

/// Match a repo-relative file path against `ignore_files`: compiled globs
/// first, then legacy exact/prefix entries.
pub(super) fn path_matches_ignore_list(
    path: &str,
    patterns: &[String],
    globs: Option<&globset::GlobSet>,
) -> bool {
    globs.is_some_and(|set| set.is_match(path)) || path_matches_any(path, patterns)
}

pub(super) fn path_matches_any(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        path == pattern
            || path
                .strip_prefix(pattern.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

pub(super) fn module_matches_any(qualified_name: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        qualified_name == pattern
            || qualified_name
                .strip_prefix(pattern.as_str())
                .is_some_and(|suffix| suffix.starts_with("::"))
    })
}
