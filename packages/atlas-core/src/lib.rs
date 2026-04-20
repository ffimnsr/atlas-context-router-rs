pub mod error;
pub mod kinds;
pub mod model;

pub use error::{AtlasError, Result};
pub use kinds::{EdgeKind, NodeKind};
pub use model::{
    AdvancedImpactResult, AmbiguityMeta, BoundaryKind, BoundaryViolation, ChangeKind,
    ChangeRiskResult, ChangeType, ChangedFile, ChangedSymbolSummary, Community, CommunityNode,
    ConfidenceTier, ContextIntent, ContextRequest, ContextResult, ContextTarget, CoverageStrength,
    DeadCodeCandidate, DependencyRemovalResult, Edge, ExtractFunctionCandidate, FileRecord, Flow,
    FlowMembership, GraphStats, ImpactClass, ImpactResult, ImpactedNode, Node, NodeId,
    PackageOwner, PackageOwnerKind, ParsedFile, ReasoningEvidence, ReasoningWarning,
    RefactorDryRunResult, RefactorEdit, RefactorEditKind, RefactorOperation, RefactorPatch,
    RefactorPlan, RefactorSafetyResult, RefactorValidationResult, ReferenceScope,
    RemovalImpactResult, RenamePreviewResult, RenameReference, ReviewContext, ReviewImpactOverview,
    RiskLevel, RiskSummary, SafetyBand, SafetyScore, ScoredImpactNode, ScoredNode, SearchQuery,
    SelectedEdge, SelectedFile, SelectedNode, SelectionReason, SimulatedRefactorImpact,
    TestAdjacencyResult, TestImpactResult, TruncationMeta,
};
