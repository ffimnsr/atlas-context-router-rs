//! `ReasoningEngine` — Phase 23 implementation.
//!
//! All methods operate on graph data from the Store; no external inference.
//! Results are deterministic given the same graph state.

mod architecture;
mod dead_code;
mod helpers;
mod insights;
mod large_functions;
mod metrics;
mod patterns;
mod removal;
mod rename;
mod risk;

#[cfg(test)]
mod tests;

use atlas_store_sqlite::Store;

pub use architecture::{
    ArchitectureAnalysis, ArchitectureEdgeEvidence, ArchitectureModuleEdge, ArchitectureModuleNode,
};
pub use insights::{InsightsEngine, InsightsGraphSummary};
pub use large_functions::{
    LargeFunctionAnalysis, LargeFunctionCandidate, LargeFunctionMode, LargeFunctionRequest,
};
pub use metrics::{
    CodeHealthMetrics, FileMetric, MetricDistribution, MetricOutlier, MetricValue, MetricsAnalysis,
    ModuleMetric, NodeMetric,
};
pub use risk::{
    RiskAssessmentAnalysis, RiskAssessmentTarget, RiskClassification, RiskFactorContribution,
};

/// Provides autonomous code-reasoning queries backed by the Atlas graph store.
///
/// Wrap a `Store` reference; all methods borrow it immutably.
pub struct ReasoningEngine<'s> {
    store: &'s Store,
}

impl<'s> ReasoningEngine<'s> {
    /// Create a new engine from a shared store reference.
    pub fn new(store: &'s Store) -> Self {
        Self { store }
    }
}
