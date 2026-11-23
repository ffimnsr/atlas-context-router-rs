use super::*;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_provenance: Option<crate::model::RepoProvenance>,
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
    /// Content assets dropped to stay within the cap.
    /// Tracks non-code assets (docs, config, templates, SQL) omitted by the
    /// graph/content companion unified budget policy.
    #[serde(default)]
    pub content_assets_dropped: usize,
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
    /// Token count for the emitted payload. Compatibility name for the
    /// counted tokens produced by whichever method `token_accounting` reports.
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
    /// How `tokens_estimated` was produced: provider/model and whether a
    /// heuristic fallback counted the payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_accounting: Option<TokenAccountingMeta>,
}

/// How tokens were counted for a truncated payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenAccountingMeta {
    /// Counting provider: `"tiktoken"` or `"heuristic"`.
    pub provider: String,
    /// `tiktoken` encoding name when the provider is `"tiktoken"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// Configured model identifier when a tokenizer was used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// True when the tokenizer could not be used and the byte heuristic
    /// counted the payload instead.
    pub fallback_used: bool,
    /// Why the heuristic was used; present when `fallback_used`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
    /// Bytes per token for heuristic counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_per_token: Option<usize>,
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
            content_assets_dropped: 0,
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
