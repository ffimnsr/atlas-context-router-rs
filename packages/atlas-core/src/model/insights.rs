use serde::{Deserialize, Serialize};

use super::graph::ProvenanceMeta;
use super::reasoning::ConfidenceTier;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightSeverity {
    Info,
    Low,
    Medium,
    High,
}

impl InsightSeverity {
    pub const fn priority(self) -> u8 {
        match self {
            Self::Info => 0,
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
        }
    }
}

impl std::fmt::Display for InsightSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightLineRange {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_range: Option<InsightLineRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_tier: Option<ConfidenceTier>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightFinding {
    pub id: String,
    pub title: String,
    pub severity: InsightSeverity,
    pub category: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<InsightEvidence>,
    pub ranking_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsightSummary {
    pub total_findings: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highest_severity: Option<InsightSeverity>,
    pub generated_at: String,
}

impl InsightSummary {
    pub fn from_findings(findings: &[InsightFinding], generated_at: impl Into<String>) -> Self {
        let highest_severity = findings.iter().map(|finding| finding.severity).max();
        Self {
            total_findings: findings.len(),
            highest_severity,
            generated_at: generated_at.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessWarning {
    pub stale: bool,
    pub changed_files: Vec<String>,
    pub stale_result_files: Vec<String>,
    pub warning: String,
    pub suggested_recovery: Vec<String>,
}

macro_rules! define_insight_report {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        pub struct $name {
            pub summary: InsightSummary,
            pub findings: Vec<InsightFinding>,
            pub atlas_provenance: ProvenanceMeta,
            #[serde(skip_serializing_if = "Option::is_none")]
            pub atlas_freshness: Option<FreshnessWarning>,
        }
    };
}

define_insight_report!(ArchitectureReport);
define_insight_report!(MetricsReport);
define_insight_report!(RiskReport);
define_insight_report!(PatternReport);
define_insight_report!(LargeFunctionReport);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_finding() -> InsightFinding {
        InsightFinding {
            id: "risk:src/lib.rs::fn::compute".to_owned(),
            title: "High fan-in symbol".to_owned(),
            severity: InsightSeverity::High,
            category: "risk".to_owned(),
            message: "Symbol exceeds configured fan-in threshold.".to_owned(),
            evidence: vec![InsightEvidence {
                file_path: Some("src/lib.rs".to_owned()),
                qualified_name: Some("src/lib.rs::fn::compute".to_owned()),
                node_kind: Some("function".to_owned()),
                edge_kind: Some("calls".to_owned()),
                line_range: Some(InsightLineRange {
                    start_line: 10,
                    end_line: 28,
                }),
                confidence_tier: Some(ConfidenceTier::High),
            }],
            ranking_reason: "high severity, high score, direct evidence".to_owned(),
            details: None,
            score: 88.0,
        }
    }

    #[test]
    fn insight_summary_uses_highest_severity() {
        let findings = vec![
            InsightFinding {
                severity: InsightSeverity::Low,
                ..sample_finding()
            },
            sample_finding(),
        ];
        let summary = InsightSummary::from_findings(&findings, "2026-05-11T00:00:00Z");
        assert_eq!(summary.total_findings, 2);
        assert_eq!(summary.highest_severity, Some(InsightSeverity::High));
        assert_eq!(summary.generated_at, "2026-05-11T00:00:00Z");
    }

    #[test]
    fn insight_severity_order_is_stable() {
        assert!(InsightSeverity::High > InsightSeverity::Medium);
        assert!(InsightSeverity::Medium > InsightSeverity::Low);
        assert!(InsightSeverity::Low > InsightSeverity::Info);
        assert_eq!(InsightSeverity::Info.priority(), 0);
        assert_eq!(InsightSeverity::High.priority(), 3);
    }

    #[test]
    fn risk_report_json_shape_stable() {
        let report = RiskReport {
            summary: InsightSummary::from_findings(&[sample_finding()], "2026-05-11T00:00:00Z"),
            findings: vec![sample_finding()],
            atlas_provenance: ProvenanceMeta {
                indexed_file_count: 12,
                last_indexed_at: Some("2026-05-11T00:00:00Z".to_owned()),
            },
            atlas_freshness: Some(FreshnessWarning {
                stale: true,
                changed_files: vec!["src/lib.rs".to_owned()],
                stale_result_files: vec!["src/lib.rs".to_owned()],
                warning: "Graph-backed answer may be stale.".to_owned(),
                suggested_recovery: vec!["run build_or_update_graph".to_owned()],
            }),
        };

        let value = serde_json::to_value(&report).expect("serialize report");
        assert_eq!(
            value,
            json!({
                "summary": {
                    "total_findings": 1,
                    "highest_severity": "high",
                    "generated_at": "2026-05-11T00:00:00Z"
                },
                "findings": [{
                    "id": "risk:src/lib.rs::fn::compute",
                    "title": "High fan-in symbol",
                    "severity": "high",
                    "category": "risk",
                    "message": "Symbol exceeds configured fan-in threshold.",
                    "evidence": [{
                        "file_path": "src/lib.rs",
                        "qualified_name": "src/lib.rs::fn::compute",
                        "node_kind": "function",
                        "edge_kind": "calls",
                        "line_range": {
                            "start_line": 10,
                            "end_line": 28
                        },
                        "confidence_tier": "high"
                    }],
                    "ranking_reason": "high severity, high score, direct evidence",
                    "score": 88.0
                }],
                "atlas_provenance": {
                    "indexed_file_count": 12,
                    "last_indexed_at": "2026-05-11T00:00:00Z"
                },
                "atlas_freshness": {
                    "stale": true,
                    "changed_files": ["src/lib.rs"],
                    "stale_result_files": ["src/lib.rs"],
                    "warning": "Graph-backed answer may be stale.",
                    "suggested_recovery": ["run build_or_update_graph"]
                }
            })
        );
    }
}
