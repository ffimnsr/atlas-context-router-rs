use serde::{Deserialize, Serialize};

use crate::budget::BudgetReport;

use super::context::{SeedBudgetMeta, TraversalBudgetMeta};
use super::graph::{Edge, Node};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactResult {
    pub changed_nodes: Vec<Node>,
    pub impacted_nodes: Vec<Node>,
    pub impacted_files: Vec<String>,
    pub relevant_edges: Vec<Edge>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub seed_budgets: Vec<SeedBudgetMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traversal_budget: Option<TraversalBudgetMeta>,
    #[serde(flatten)]
    pub budget: BudgetReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewContext {
    pub changed_files: Vec<String>,
    pub changed_symbols: Vec<Node>,
    pub changed_symbol_summaries: Vec<ChangedSymbolSummary>,
    pub impacted_neighbors: Vec<Node>,
    pub critical_edges: Vec<Edge>,
    pub impact_overview: ReviewImpactOverview,
    pub risk_summary: RiskSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedSymbolSummary {
    pub node: Node,
    pub callers: Vec<Node>,
    pub callees: Vec<Node>,
    pub importers: Vec<Node>,
    pub tests: Vec<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewImpactOverview {
    pub max_depth: u32,
    pub max_nodes: usize,
    pub impacted_node_count: usize,
    pub impacted_file_count: usize,
    pub relevant_edge_count: usize,
    pub reached_node_limit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummary {
    pub changed_symbol_count: usize,
    pub public_api_changes: usize,
    pub test_adjacent: bool,
    pub affected_test_count: usize,
    pub uncovered_changed_symbol_count: usize,
    pub large_function_touched: bool,
    pub large_function_count: usize,
    pub cross_module_impact: bool,
    pub cross_package_impact: bool,
}

/// Coarse risk level assigned to a change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
            RiskLevel::Critical => "critical",
        };
        f.write_str(s)
    }
}

/// How a symbol was changed in the current diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Public/visibility modifier changed — externally visible interface break.
    ApiChange,
    /// Parameter list or return type changed.
    SignatureChange,
    /// Body-only change; public interface is stable.
    InternalChange,
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ChangeKind::ApiChange => "api_change",
            ChangeKind::SignatureChange => "signature_change",
            ChangeKind::InternalChange => "internal_change",
        };
        f.write_str(s)
    }
}

/// A node enriched with a weighted impact score and optional change classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredImpactNode {
    pub node: Node,
    /// Higher = more impacted. Based on weighted edge traversal from seed nodes.
    pub impact_score: f64,
    /// Set only for nodes that are direct seed (changed) nodes.
    pub change_kind: Option<ChangeKind>,
}

/// Test-coverage impact: which tests are affected and which changed symbols
/// have no adjacent test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestImpactResult {
    /// Test nodes (`is_test = true` or `NodeKind::Test`) within the impact set.
    pub affected_tests: Vec<Node>,
    /// Changed nodes that have no `Tests`/`TestedBy` edge to any test node.
    pub uncovered_changed_nodes: Vec<Node>,
}

/// A detected cross-boundary impact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryViolation {
    pub kind: BoundaryKind,
    pub description: String,
    /// Representative qualified names crossing the boundary.
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    CrossModule,
    CrossPackage,
}

impl std::fmt::Display for BoundaryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BoundaryKind::CrossModule => "cross_module",
            BoundaryKind::CrossPackage => "cross_package",
        };
        f.write_str(s)
    }
}

/// Full advanced impact analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedImpactResult {
    /// Base impact radius (unchanged nodes + impacted nodes + edges).
    pub base: ImpactResult,
    /// Impacted nodes ranked by weighted traversal score (highest first).
    pub scored_nodes: Vec<ScoredImpactNode>,
    /// Overall risk level for this change set.
    pub risk_level: RiskLevel,
    /// Test-coverage impact.
    pub test_impact: TestImpactResult,
    /// Architecture boundary violations detected.
    pub boundary_violations: Vec<BoundaryViolation>,
}
