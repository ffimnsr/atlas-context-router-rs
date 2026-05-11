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
        let findings = self.prepare_findings(findings);
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
        let findings = self.prepare_findings(findings);
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
            evidence
                .file_path
                .as_deref()
                .is_some_and(|path| path_matches_any(path, &self.config.ignore_files))
                || evidence
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
