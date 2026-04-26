//! Shared Atlas domain model crate.
//!
//! Defines cross-crate types used by graph build, update, query, review, and
//! transport layers:
//! - graph node and edge kinds
//! - persisted and returned data models
//! - budget, health, and error contracts
//!
//! Most other Atlas crates depend on this crate for stable shared types instead
//! of re-declaring transport or storage-specific copies.

pub mod budget;
pub mod clock;
pub mod error;
pub mod health;
pub mod kinds;
pub mod model;

pub use budget::{
    BudgetHitBehavior, BudgetLimitRule, BudgetManager, BudgetNamespace, BudgetOutcomeClass,
    BudgetPolicy, BudgetReport, BudgetStage, BudgetStageRule, BudgetStatus,
    BuildUpdateBudgetCounters, budget_stage_rules,
};
pub use clock::{Clock, FixedClock, SystemClock, format_rfc3339, now_utc};
pub use error::{AtlasError, Result};
pub use health::{
    GraphHealthInput, graph_health_error_message, graph_health_error_suggestions,
    is_schema_mismatch_error, select_graph_health_error_code, user_facing_error_message,
};
pub use kinds::{EdgeKind, NodeKind};
pub use model::{
    AdvancedImpactResult, AmbiguityMeta, BoundaryKind, BoundaryViolation, ChangeKind,
    ChangeRiskResult, ChangeType, ChangedFile, ChangedSymbolSummary, Community, CommunityNode,
    ConfidenceTier, ContextIntent, ContextRankingEvidence, ContextRequest, ContextResult,
    ContextScoreEvidence, ContextTarget, CoverageStrength, DeadCodeCandidate,
    DependencyRemovalResult, Edge, ExtractFunctionCandidate, FileRecord, Flow, FlowMembership,
    FuzzyCorrectionEvidence, GraphExpansionEvidence, GraphStats, HybridRankContribution,
    HybridRankingSource, HybridRrfEvidence, ImpactClass, ImpactResult, ImpactedNode, Node, NodeId,
    NoiseReductionSummary, PackageOwner, PackageOwnerKind, ParsedFile, PayloadTruncationMeta,
    PostprocessExecutionMode, PostprocessRunState, PostprocessRunSummary, PostprocessStageStatus,
    PostprocessStageSummary, PostprocessStatus, ProvenanceMeta, RankingEvidence, ReasoningEvidence,
    ReasoningWarning, RefactorDryRunResult, RefactorEdit, RefactorEditKind, RefactorOperation,
    RefactorPatch, RefactorPlan, RefactorSafetyResult, RefactorValidationResult, ReferenceScope,
    RemovalImpactResult, RenamePreviewResult, RenameReference, RetrievalMode, ReviewContext,
    ReviewImpactOverview, RiskLevel, RiskSummary, SafetyBand, SafetyScore, SavedContextSource,
    ScoreEvidence, ScoredImpactNode, ScoredNode, SearchMatchedField, SearchQuery, SelectedEdge,
    SelectedFile, SelectedNode, SelectionReason, SimulatedRefactorImpact, TestAdjacencyResult,
    TestImpactResult, TruncationMeta, WorkflowCallChain, WorkflowComponent, WorkflowFocusNode,
    WorkflowSummary, context_ranking_evidence_legend, ranking_evidence_legend,
};
