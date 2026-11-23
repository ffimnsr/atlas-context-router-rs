use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RiskAssessmentTarget {
    Symbol { symbol: String },
    ResolvedNode { node: Box<Node> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClassification {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for RiskClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskFactorContribution {
    pub factor: String,
    pub raw_value: Value,
    pub normalized: f64,
    pub weight: f64,
    pub contribution: f64,
    pub mitigates_risk: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<InsightEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessmentAnalysis {
    pub target: Node,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_symbol: Option<String>,
    pub score: f64,
    pub classification: RiskClassification,
    pub report: RiskReport,
    pub factor_contributions: Vec<RiskFactorContribution>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedRiskTarget {
    pub(super) node: Node,
    pub(super) requested_symbol: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct RiskRelationshipContext {
    pub(super) inbound: Vec<(Node, Edge)>,
    pub(super) outbound: Vec<(Node, Edge)>,
    pub(super) direct_tests: Vec<(Node, Edge)>,
    pub(super) file_tests: Vec<Node>,
    pub(super) cross_module_neighbors: Vec<Node>,
    pub(super) unresolved_edges: Vec<(Node, Edge)>,
    pub(super) dependency_path: Vec<Node>,
    pub(super) cycle_evidence: Vec<InsightEvidence>,
    pub(super) cycle_member_count: usize,
}

pub(super) struct RiskFactorInput {
    pub(super) factor: &'static str,
    pub(super) raw_value: Value,
    pub(super) normalized: f64,
    pub(super) weight: f64,
    pub(super) mitigates_risk: bool,
    pub(super) reason: String,
    pub(super) evidence: Vec<InsightEvidence>,
}
