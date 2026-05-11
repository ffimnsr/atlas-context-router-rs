//! Atlas reasoning engine — Phase 23.
//!
//! Answers structural questions from graph + parser + store facts only.
//! No unsupported claims. All results carry structured `ReasoningEvidence`
//! and `ConfidenceTier` so callers can explain decisions.
//!
//! # Entry point
//! ```ignore
//! let engine = ReasoningEngine::new(store);
//! let result = engine.analyze_removal(&["my_crate::foo::bar"], 3, 200)?;
//! ```

mod engine;
mod ranking;

pub use engine::ReasoningEngine;
pub use engine::{
    ArchitectureAnalysis, ArchitectureEdgeEvidence, ArchitectureModuleEdge, ArchitectureModuleNode,
    CodeHealthMetrics, FileMetric, InsightsEngine, InsightsGraphSummary, LargeFunctionAnalysis,
    LargeFunctionCandidate, LargeFunctionMode, LargeFunctionRequest, MetricDistribution,
    MetricOutlier, MetricValue, MetricsAnalysis, ModuleMetric, NodeMetric, RiskAssessmentAnalysis,
    RiskAssessmentTarget, RiskClassification, RiskFactorContribution,
};
pub use ranking::{
    AnalysisRankingPrimitives, AnalysisTrimmingPrimitives, sort_dead_code_candidates,
    sort_dependency_result, sort_insight_findings, sort_refactor_safety_result,
    sort_removal_result, trim_insight_findings,
};
