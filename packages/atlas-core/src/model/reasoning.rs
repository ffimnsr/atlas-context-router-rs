use serde::{Deserialize, Serialize};

use crate::budget::BudgetReport;
use crate::kinds::EdgeKind;

use super::graph::{Edge, Node};
use super::impact::RiskLevel;

/// Confidence tier for reasoning outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceTier {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for ConfidenceTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConfidenceTier::High => "high",
            ConfidenceTier::Medium => "medium",
            ConfidenceTier::Low => "low",
        };
        f.write_str(s)
    }
}

/// How certain the engine is that a given node is impacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactClass {
    /// Direct call / import / test edge — definitely impacted.
    Definite,
    /// Inferred link or unresolved selector in same file/package.
    Probable,
    /// Textual or unresolved weak edge only.
    Weak,
}

impl std::fmt::Display for ImpactClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ImpactClass::Definite => "definite",
            ImpactClass::Probable => "probable",
            ImpactClass::Weak => "weak",
        };
        f.write_str(s)
    }
}

/// Coarse refactor-safety band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyBand {
    Safe,
    Caution,
    Risky,
}

impl std::fmt::Display for SafetyBand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SafetyBand::Safe => "safe",
            SafetyBand::Caution => "caution",
            SafetyBand::Risky => "risky",
        };
        f.write_str(s)
    }
}

/// Numeric refactor-safety score with a coarse band and human-readable reasons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyScore {
    /// 0.0 = most risky, 1.0 = safest.
    pub score: f64,
    pub band: SafetyBand,
    pub reasons: Vec<String>,
    pub suggested_validations: Vec<String>,
}

/// A single piece of supporting evidence for a reasoning result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningEvidence {
    pub key: String,
    pub value: String,
}

/// A warning attached to a reasoning result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningWarning {
    pub message: String,
    pub confidence: ConfidenceTier,
    /// Machine-readable error code (e.g. `"seed_not_found"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Suggested remediation steps for this warning.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub suggestions: Vec<String>,
}

/// A node enriched with depth, impact class, and the edge kind that introduced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactedNode {
    pub node: Node,
    pub depth: u32,
    pub impact_class: ImpactClass,
    pub via_edge_kind: Option<EdgeKind>,
}

/// Full removal-impact result (23.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovalImpactResult {
    pub seed: Vec<Node>,
    pub impacted_symbols: Vec<ImpactedNode>,
    pub impacted_files: Vec<String>,
    pub impacted_tests: Vec<Node>,
    pub relevant_edges: Vec<Edge>,
    /// Nodes that directly served as evidence for the impact conclusion.
    pub evidence_nodes: Vec<Node>,
    pub warnings: Vec<ReasoningWarning>,
    pub evidence: Vec<ReasoningEvidence>,
    /// Uncertainty flags: qualitative reasons the result may be incomplete.
    pub uncertainty_flags: Vec<String>,
    #[serde(flatten)]
    pub budget: BudgetReport,
}

/// A dead code candidate with its flagging reasons (23.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeCandidate {
    pub node: Node,
    pub reasons: Vec<String>,
    pub certainty: ConfidenceTier,
    /// Blockers that prevent automatic removal.
    pub blockers: Vec<String>,
}

/// Strength of test coverage for a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStrength {
    Direct,
    /// A caller of this symbol has direct test coverage.
    IndirectThroughCallers,
    SameFile,
    SameModule,
    None,
}

/// Refactor-safety result for a single symbol (23.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorSafetyResult {
    pub node: Node,
    pub safety: SafetyScore,
    pub fan_in: usize,
    pub fan_out: usize,
    pub linked_test_count: usize,
    pub unresolved_edge_count: usize,
    /// Classified test coverage strength for this symbol.
    pub coverage_strength: CoverageStrength,
    pub evidence: Vec<ReasoningEvidence>,
    #[serde(flatten)]
    pub budget: BudgetReport,
}

/// Dependency-removal validation result (23.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRemovalResult {
    pub target_qname: String,
    pub removable: bool,
    pub blocking_references: Vec<Node>,
    /// Edges that directly block removal or serve as evidence.
    pub evidence_edges: Vec<Edge>,
    pub confidence: ConfidenceTier,
    pub suggested_cleanups: Vec<String>,
    pub evidence: Vec<ReasoningEvidence>,
    /// Uncertainty flags: qualitative reasons the result may be incomplete.
    pub uncertainty_flags: Vec<String>,
    #[serde(flatten)]
    pub budget: BudgetReport,
}

/// Scope of a reference relative to the definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceScope {
    SameFile,
    SameModule,
    CrossModule,
    Test,
}

/// One reference that would be affected by a rename (23.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameReference {
    pub node: Node,
    pub edge: Edge,
    pub scope: ReferenceScope,
}

/// Rename blast-radius preview (23.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePreviewResult {
    pub target: Node,
    pub new_name: String,
    pub affected_references: Vec<RenameReference>,
    pub affected_files: Vec<String>,
    pub risk_level: RiskLevel,
    pub collision_warnings: Vec<String>,
    pub manual_review_flags: Vec<String>,
}

/// Test-adjacency result for a single symbol (23.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestAdjacencyResult {
    pub symbol: Node,
    pub linked_tests: Vec<Node>,
    pub coverage_strength: CoverageStrength,
    pub recommendation: Option<String>,
}

/// Change-risk classification result (23.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRiskResult {
    pub risk_level: RiskLevel,
    pub contributing_factors: Vec<String>,
    pub suggested_review_focus: Vec<String>,
}
