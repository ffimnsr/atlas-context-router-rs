use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::budget::BudgetReport;

use super::graph::{Edge, Node};

/// What kind of context retrieval is being requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextIntent {
    /// Context centred on a specific symbol (callers, callees, imports, neighbors).
    Symbol,
    /// Context centred on a specific file (symbols defined in it and direct neighbors).
    File,
    /// Review context for a set of changed files (impact + risk).
    Review,
    /// Impact context seeded from files or symbol names.
    Impact,
    /// Breakage / impact analysis — "what breaks if I change X?"
    ImpactAnalysis,
    /// Usage lookup — "who calls / uses X?"
    UsageLookup,
    /// Refactor safety check — "is it safe to refactor X?"
    RefactorSafety,
    /// Dead code check — "is X unused?"
    DeadCodeCheck,
    /// Rename preview — "what does renaming X affect?"
    RenamePreview,
    /// Dependency removal — "what depends on this dep?"
    DependencyRemoval,
}

/// The seed target for a context request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextTarget {
    /// Exact fully-qualified name (`crate::module::fn_name`).
    QualifiedName { qname: String },
    /// Short symbol name — may produce ambiguity.
    SymbolName { name: String },
    /// Repo-relative file path.
    FilePath { path: String },
    /// Set of repo-relative changed file paths (for review/impact intents).
    ChangedFiles { paths: Vec<String> },
    /// Set of changed symbol qualified names (for symbol-seeded impact).
    ChangedSymbols { qnames: Vec<String> },
    /// Seed from a specific edge relationship (source symbol + optional edge kind).
    EdgeQuerySeed {
        source_qname: String,
        edge_kind: Option<String>,
    },
}

/// Structured request to the context engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRequest {
    pub intent: ContextIntent,
    pub target: ContextTarget,
    /// Maximum nodes in the result (hard cap; engine trims before returning).
    pub max_nodes: Option<usize>,
    /// Maximum edges in the result.
    pub max_edges: Option<usize>,
    /// Maximum files in the result.
    pub max_files: Option<usize>,
    /// Graph traversal depth (hops from seed). `None` defaults to 1.
    pub depth: Option<u32>,
    /// Include test nodes in context.
    pub include_tests: bool,
    /// Include import edges/nodes in context.
    pub include_imports: bool,
    /// Include containment-sibling nodes.
    pub include_neighbors: bool,
    /// Populate `SelectedFile.line_ranges` with node spans (Slice 8).
    pub include_code_spans: bool,
    /// Include direct callers in context (defaults to true).
    pub include_callers: bool,
    /// Include direct callees in context (defaults to true).
    pub include_callees: bool,
    // --- CM6: retrieval-backed restoration ---
    /// When `true`, the engine queries the content store for saved artifacts
    /// relevant to this request and populates `ContextResult::saved_context_sources`.
    /// Has no effect when no content store is provided to the engine.
    pub include_saved_context: bool,
    /// Restrict saved-context retrieval to artifacts from this session.
    /// Also applies a same-session relevance boost when scoring.
    pub session_id: Option<String>,
    /// Restrict saved-context retrieval to one agent memory partition.
    /// When omitted, retrieval preserves legacy merged behavior.
    pub agent_id: Option<String>,
    /// When true, allow retrieval across all agent partitions intentionally.
    pub merge_agent_partitions: bool,
    // --- CM13: context budget optimization ---
    /// Optional per-call token budget. When set the engine enforces this cap
    /// during payload trimming instead of (or in addition to) the policy
    /// default. The effective limit is always capped by
    /// `BudgetPolicy::mcp_cli_payload_serialization.context_tokens_estimate.max_limit`
    /// so callers cannot bypass the central policy ceiling.
    pub token_budget: Option<usize>,
}

impl Default for ContextRequest {
    fn default() -> Self {
        Self {
            intent: ContextIntent::Symbol,
            target: ContextTarget::SymbolName {
                name: String::new(),
            },
            max_nodes: None,
            max_edges: None,
            max_files: None,
            depth: None,
            include_tests: false,
            include_imports: true,
            include_neighbors: false,
            include_code_spans: false,
            include_callers: true,
            include_callees: true,
            include_saved_context: false,
            session_id: None,
            agent_id: None,
            merge_agent_partitions: false,
            token_budget: None,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Why a node, edge, or file was included in a [`ContextResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionReason {
    /// The primary seed target.
    DirectTarget,
    /// Calls the target (caller of seed).
    Caller,
    /// Called by the target (callee of seed).
    Callee,
    /// Imports the target.
    Importer,
    /// Imported by the target.
    Importee,
    /// Sibling contained in the same parent scope.
    ContainmentSibling,
    /// Adjacent to a test node.
    TestAdjacent,
    /// Reached via impact traversal.
    ImpactNeighbor,
}

impl SelectionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectTarget => "direct_target",
            Self::Caller => "caller",
            Self::Callee => "callee",
            Self::Importer => "importer",
            Self::Importee => "importee",
            Self::ContainmentSibling => "containment_sibling",
            Self::TestAdjacent => "test_adjacent",
            Self::ImpactNeighbor => "impact_neighbor",
        }
    }
}

/// Structured explanation for why a node, edge, or saved artifact ranked where
/// it did inside bounded context assembly.
///
/// This contract is intentionally separate from search ranking evidence.
/// Search ranking evidence explains why a symbol or file matched retrieval.
/// Context ranking evidence explains why an item was included and ranked within
/// a context/review result after graph expansion, impact analysis, and
/// saved-context scoring.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextRankingEvidence {
    /// Initial relevance score before context-specific additive boosts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_score: Option<f32>,
    /// Final relevance score after all context-specific contributions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_score: Option<f32>,
    /// Item is the direct target of the context request.
    #[serde(default, skip_serializing_if = "is_false")]
    pub direct_target: bool,
    /// Item is a changed symbol in review / impact context.
    #[serde(default, skip_serializing_if = "is_false")]
    pub changed_symbol: bool,
    /// Item was included as a caller neighbor.
    #[serde(default, skip_serializing_if = "is_false")]
    pub caller_neighbor: bool,
    /// Item was included as a callee neighbor.
    #[serde(default, skip_serializing_if = "is_false")]
    pub callee_neighbor: bool,
    /// Item was included through test adjacency.
    #[serde(default, skip_serializing_if = "is_false")]
    pub test_adjacent: bool,
    /// Additive score contributed by impact analysis weighting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact_score_contribution: Option<f32>,
    /// Base lexical retrieval rank contribution for saved-context sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_context_rank_score: Option<f32>,
    /// Additive boost for recently created saved-context sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_source_boost: Option<f32>,
    /// Additive boost for saved-context sources from the active session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_session_boost: Option<f32>,
}

pub type ContextScoreEvidence = ContextRankingEvidence;

impl ContextRankingEvidence {
    pub fn from_selection_reason(reason: SelectionReason) -> Self {
        let mut evidence = Self::default();
        match reason {
            SelectionReason::DirectTarget => evidence.direct_target = true,
            SelectionReason::Caller => evidence.caller_neighbor = true,
            SelectionReason::Callee => evidence.callee_neighbor = true,
            SelectionReason::TestAdjacent => evidence.test_adjacent = true,
            SelectionReason::Importer
            | SelectionReason::Importee
            | SelectionReason::ContainmentSibling
            | SelectionReason::ImpactNeighbor => {}
        }
        evidence
    }

    pub fn sync_score(&mut self, score: f32) {
        self.base_score.get_or_insert(score);
        self.final_score = Some(score);
    }
}

pub fn context_ranking_evidence_legend() -> serde_json::Value {
    json!({
        "contract_scope": "Separate from retrieval ranking evidence: retrieval evidence explains why a result matched search, while context ranking evidence explains why a node, edge, or saved artifact was included and ranked inside a bounded context or review result.",
        "base_score": "Initial context relevance score before impact or saved-context additive boosts.",
        "final_score": "Final context relevance score after all recorded contributions.",
        "direct_target": "Item is the primary target resolved from the request.",
        "changed_symbol": "Item is a changed symbol selected from changed-file or impact review seeds.",
        "caller_neighbor": "Item was included as a caller of the target or neighbor.",
        "callee_neighbor": "Item was included as a callee of the target or neighbor.",
        "test_adjacent": "Item was included because of test adjacency.",
        "impact_score_contribution": "Additive contribution from impact analysis weighting in review or impact context.",
        "saved_context_rank_score": "Base saved-context ranking contribution derived from retrieval rank.",
        "recent_source_boost": "Additive boost for recently created saved-context artifacts.",
        "same_session_boost": "Additive boost for saved-context artifacts from the active session."
    })
}

/// A graph node selected for inclusion in a [`ContextResult`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedNode {
    pub node: Node,
    pub selection_reason: SelectionReason,
    /// Graph-hop distance from the seed node (0 = seed itself).
    pub distance: u32,
    /// Relevance score assigned by `rank_context` (higher = more relevant).
    /// Zero before ranking is applied.
    pub relevance_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_ranking_evidence: Option<ContextRankingEvidence>,
}

/// A graph edge selected for inclusion in a [`ContextResult`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedEdge {
    pub edge: Edge,
    pub selection_reason: SelectionReason,
    /// Hop depth at which this edge was encountered (None when not tracked).
    pub depth: Option<u32>,
    /// Relevance score assigned by `rank_context` (higher = more relevant).
    /// Zero before ranking is applied.
    pub relevance_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_ranking_evidence: Option<ContextRankingEvidence>,
}

/// A file selected for inclusion in a [`ContextResult`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedFile {
    pub path: String,
    pub selection_reason: SelectionReason,
    /// Relevant line ranges within the file (start, end inclusive).
    /// Empty means no span narrowing has been applied yet.
    pub line_ranges: Vec<(u32, u32)>,
    /// Primary language of the file (derived from the first node in the file).
    pub language: Option<String>,
    /// Number of nodes included from this file after trimming.
    pub node_count_included: usize,
}

/// Metadata about truncation applied to a [`ContextResult`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruncationMeta {
    /// Nodes dropped to stay within the cap.
    pub nodes_dropped: usize,
    /// Edges dropped to stay within the cap.
    pub edges_dropped: usize,
    /// Files dropped to stay within the cap.
    pub files_dropped: usize,
    /// `true` when any item was dropped.
    pub truncated: bool,
    /// Payload-level trimming metadata applied after graph selection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<PayloadTruncationMeta>,
}

/// Metadata about byte/token trimming applied to a serialized context payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayloadTruncationMeta {
    pub bytes_requested: usize,
    pub bytes_emitted: usize,
    pub tokens_estimated: usize,
    /// Effective token budget that was enforced (from request or policy default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_budget_applied: Option<usize>,
    pub omitted_node_count: usize,
    pub omitted_file_count: usize,
    pub omitted_source_count: usize,
    pub omitted_byte_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_hint: Option<String>,
    /// Per-source-type token usage after trimming.
    /// Shows how many tokens/bytes each source kind contributed so callers
    /// can understand how the budget was allocated.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub source_mix: Vec<ContextSourceMix>,
}

/// Token/byte usage for a single context source kind inside a [`ContextResult`].
///
/// Populated in [`PayloadTruncationMeta::source_mix`] whenever payload
/// trimming runs so callers can see how the token budget was distributed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSourceMix {
    /// Source kind: `"graph_context"`, `"saved_artifacts"`, or `"resume_snapshot"`.
    pub source_kind: String,
    /// Items included in the emitted result.
    pub items_included: usize,
    /// Items dropped to stay within budget.
    pub items_dropped: usize,
    /// Estimated tokens used by this source in the emitted result.
    pub tokens_used: usize,
}

impl TruncationMeta {
    /// No truncation applied.
    pub fn none() -> Self {
        Self {
            nodes_dropped: 0,
            edges_dropped: 0,
            files_dropped: 0,
            truncated: false,
            payload: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedBudgetMeta {
    pub seed_kind: String,
    pub requested_seed_count: usize,
    pub accepted_seed_count: usize,
    pub omitted_seed_count: usize,
    pub budget_hit: bool,
    pub partial: bool,
    pub safe_to_answer: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_narrower_query: Option<String>,
}

impl SeedBudgetMeta {
    pub fn new(
        seed_kind: impl Into<String>,
        requested_seed_count: usize,
        accepted_seed_count: usize,
        safe_to_answer: bool,
        suggested_narrower_query: Option<String>,
    ) -> Self {
        let omitted_seed_count = requested_seed_count.saturating_sub(accepted_seed_count);
        Self {
            seed_kind: seed_kind.into(),
            requested_seed_count,
            accepted_seed_count,
            omitted_seed_count,
            budget_hit: omitted_seed_count > 0 || !safe_to_answer,
            partial: omitted_seed_count > 0 && accepted_seed_count > 0 && safe_to_answer,
            safe_to_answer,
            suggested_narrower_query,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraversalBudgetMeta {
    pub requested_depth: u32,
    pub accepted_depth: u32,
    pub requested_node_budget: usize,
    pub accepted_node_budget: usize,
    pub requested_edge_budget: usize,
    pub accepted_edge_budget: usize,
    pub emitted_node_count: usize,
    pub emitted_edge_count: usize,
    pub omitted_edge_count: usize,
    pub budget_hit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_narrower_query: Option<String>,
}

/// Metadata about target resolution when the seed was ambiguous.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbiguityMeta {
    /// The ambiguous query string that was submitted.
    pub query: String,
    /// Ranked candidate qualified names. Empty when `resolved` is `true`.
    pub candidates: Vec<String>,
    /// `true` when the engine selected a single candidate automatically.
    pub resolved: bool,
}

/// High-signal node surfaced for developer workflow output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowFocusNode {
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    pub relevance_score: f32,
    pub selection_reason: String,
}

/// Grouped impact summary for one component / package / directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowComponent {
    pub label: String,
    pub kind: String,
    pub changed_node_count: usize,
    pub impacted_node_count: usize,
    pub file_count: usize,
    pub summary: String,
}

/// One concise call chain or dependency chain relevant to a workflow result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCallChain {
    pub summary: String,
    pub steps: Vec<String>,
    pub edge_kinds: Vec<String>,
}

/// Summary of workflow filters and trimming used to keep output focused.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoiseReductionSummary {
    pub retained_nodes: usize,
    pub retained_edges: usize,
    pub retained_files: usize,
    pub dropped_nodes: usize,
    pub dropped_edges: usize,
    pub dropped_files: usize,
    pub rules_applied: Vec<String>,
}

/// Focused metadata for review, explain-change, and interactive workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSummary {
    pub headline: Option<String>,
    pub high_impact_nodes: Vec<WorkflowFocusNode>,
    pub impacted_components: Vec<WorkflowComponent>,
    pub call_chains: Vec<WorkflowCallChain>,
    pub ripple_effects: Vec<String>,
    pub noise_reduction: NoiseReductionSummary,
}

/// A saved artifact from the content store surfaced inside a [`ContextResult`].
///
/// Returned when `ContextRequest::include_saved_context` is `true` and the
/// content store contains artifacts relevant to the current query. Only
/// compact metadata and a short preview are included here; the full content
/// is retrieved separately using `source_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedContextSource {
    /// Stable identifier for the stored artifact; pass to content-store
    /// retrieval APIs to fetch the full blob.
    pub source_id: String,
    /// Human-readable label assigned when the artifact was indexed.
    pub label: String,
    /// Category: `"review_context"`, `"impact_result"`, `"command_output"`, etc.
    pub source_type: String,
    /// Session that produced this artifact, if recorded.
    pub session_id: Option<String>,
    /// Agent partition that produced this artifact, if recorded.
    pub agent_id: Option<String>,
    /// Truncated preview of the first chunk (≤ 512 chars); never the raw blob.
    pub preview: String,
    /// Opaque hint an agent can use to retrieve the full artifact,
    /// e.g. `"source_id=<id>"` for a future MCP `search_saved_context` call.
    pub retrieval_hint: String,
    /// Relevance score for ranking within this result (higher = more relevant).
    pub relevance_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_ranking_evidence: Option<ContextRankingEvidence>,
}

/// Output of the context engine for a single [`ContextRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResult {
    pub request: ContextRequest,
    pub nodes: Vec<SelectedNode>,
    pub edges: Vec<SelectedEdge>,
    pub files: Vec<SelectedFile>,
    pub truncation: TruncationMeta,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub seed_budgets: Vec<SeedBudgetMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traversal_budget: Option<TraversalBudgetMeta>,
    /// Set when the target was ambiguous; contains ranked candidates.
    pub ambiguity: Option<AmbiguityMeta>,
    /// Focused workflow metadata for higher-level developer UX.
    pub workflow: Option<WorkflowSummary>,
    /// Relevant saved artifacts from the content store.
    ///
    /// Populated only when `request.include_saved_context` is `true` and a
    /// content store is provided to the engine. Ordered by descending
    /// `relevance_score`.
    pub saved_context_sources: Vec<SavedContextSource>,
    #[serde(flatten)]
    pub budget: BudgetReport,
}
