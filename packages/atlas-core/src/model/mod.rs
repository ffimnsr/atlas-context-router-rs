mod context;
mod graph;
mod grouping;
mod impact;
mod reasoning;
mod refactor;
mod search;

pub use self::context::{
    AmbiguityMeta, ContextIntent, ContextRequest, ContextResult, ContextSourceMix, ContextTarget,
    NoiseReductionSummary, PayloadTruncationMeta, SavedContextSource, SeedBudgetMeta, SelectedEdge,
    SelectedFile, SelectedNode, SelectionReason, TraversalBudgetMeta, TruncationMeta,
    WorkflowCallChain, WorkflowComponent, WorkflowFocusNode, WorkflowSummary,
};
pub use self::graph::{
    ChangeType, ChangedFile, Edge, FileRecord, GraphStats, Node, NodeId, PackageOwner,
    PackageOwnerKind, ParsedFile, ProvenanceMeta,
};
pub use self::grouping::{Community, CommunityNode, Flow, FlowMembership};
pub use self::impact::{
    AdvancedImpactResult, BoundaryKind, BoundaryViolation, ChangeKind, ChangedSymbolSummary,
    ImpactResult, ReviewContext, ReviewImpactOverview, RiskLevel, RiskSummary, ScoredImpactNode,
    TestImpactResult,
};
pub use self::reasoning::{
    ChangeRiskResult, ConfidenceTier, CoverageStrength, DeadCodeCandidate, DependencyRemovalResult,
    ImpactClass, ImpactedNode, ReasoningEvidence, ReasoningWarning, RefactorSafetyResult,
    ReferenceScope, RemovalImpactResult, RenamePreviewResult, RenameReference, SafetyBand,
    SafetyScore, TestAdjacencyResult,
};
pub use self::refactor::{
    ExtractFunctionCandidate, RefactorDryRunResult, RefactorEdit, RefactorEditKind,
    RefactorOperation, RefactorPatch, RefactorPlan, RefactorValidationResult,
    SimulatedRefactorImpact,
};
pub use self::search::{ScoredNode, SearchQuery};

#[cfg(test)]
mod tests;
