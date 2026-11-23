use super::*;

/// A non-code content asset (doc, config, template, SQL, prompt) surfaced as
/// part of a unified graph/content context result.
///
/// Populated when `ContextRequest::include_content_assets` is `true` and a
/// content store is provided to the engine. Only compact metadata and a short
/// preview are included; full content is fetched via `source_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAsset {
    /// Stable identifier for the stored source.
    pub source_id: String,
    /// File path or label for this asset (canonical repo-relative when available).
    pub path: String,
    /// Asset category: `"doc"`, `"config"`, `"template"`, `"sql"`, `"prompt"`, or `"other"`.
    pub content_type: String,
    /// Truncated preview (≤ 512 chars).
    pub preview: String,
    /// Why this asset was included under the unified selection policy.
    pub selection_reason: ContentAssetReason,
    /// Relevance score for ranking within this result (higher = more relevant).
    pub relevance_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_ranking_evidence: Option<ContextRankingEvidence>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_provenance: Option<crate::model::RepoProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_ranking_evidence: Option<ContextRankingEvidence>,
}

/// Requested vs emitted payload accounting for one context build (R7.1).
///
/// Populated only when [`ContextRequest::include_telemetry`] is `true`; the
/// engine always records the same measurements as process metrics so default
/// output stays byte-identical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextTelemetry {
    pub intent: ContextIntent,
    pub requested_nodes: usize,
    pub emitted_nodes: usize,
    pub requested_edges: usize,
    pub emitted_edges: usize,
    pub requested_files: usize,
    pub emitted_files: usize,
    pub requested_saved_sources: usize,
    pub emitted_saved_sources: usize,
    pub requested_content_assets: usize,
    pub emitted_content_assets: usize,
    pub requested_overviews: usize,
    pub emitted_overviews: usize,
    pub requested_bytes: usize,
    pub emitted_bytes: usize,
    pub requested_tokens: usize,
    pub emitted_tokens: usize,
    pub budget_hit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_name: Option<String>,
    pub duration_ms: u64,
    pub outcome: ContextOutcome,
}

/// Deterministic outcome classification for one context build (R7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextOutcome {
    /// Context was built and emitted at least one selected item.
    Ok,
    /// No graph, file, content, saved-context, or overview item was emitted.
    Empty,
    /// Target resolution produced ranked ambiguity candidates.
    Ambiguous,
    /// Payload or selection budgets dropped emitted items.
    BudgetExhausted,
    /// Build failed before a result could be produced.
    Error,
}

impl ContextOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Empty => "empty",
            Self::Ambiguous => "ambiguous",
            Self::BudgetExhausted => "budget_exhausted",
            Self::Error => "error",
        }
    }
}

/// Output of the context engine for a single [`ContextRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResult {
    pub request: ContextRequest,
    pub nodes: Vec<SelectedNode>,
    pub edges: Vec<SelectedEdge>,
    pub files: Vec<SelectedFile>,
    /// Non-code content assets adjacent to the graph targets.
    ///
    /// Populated only when `request.include_content_assets` is `true` and a
    /// content store is provided. Ordered by descending `relevance_score`
    /// under the unified graph/content selection policy (N2).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub content_assets: Vec<ContentAsset>,
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
    /// Fresh (or explicitly allowed stale) overview records relevant to the
    /// selected graph context (ICM-G7). Preview-only; full bodies are fetched
    /// through `source_id`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overviews: Vec<ContextOverview>,
    /// Deterministic evidence-sufficiency report and escalation trace (VR3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sufficiency: Option<EvidenceSufficiencyReport>,
    /// Repo provenance hoisted out of emitted items when every item shares the
    /// same provenance (R7.2). When `None`, provenance stays on each item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_provenance: Option<crate::model::RepoProvenance>,
    /// Requested vs emitted accounting (R7.1). Omitted unless requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<ContextTelemetry>,
    #[serde(flatten)]
    pub budget: BudgetReport,
}

/// One overview record surfaced inside a [`ContextResult`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextOverview {
    pub overview_id: String,
    pub subject_id: String,
    pub subject_kind: String,
    pub title: String,
    pub summary: String,
    /// Bounded preview; never the full overview body.
    pub preview: String,
    pub freshness: String,
    pub confidence: f64,
    /// Why this record matched: `same_symbol`, `same_file`, `containing_module`,
    /// `package_summary`, `project_summary`, or `changed_dependency`.
    pub selection_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
    /// Disclosure level this entry was rendered at (VR1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// Canonical `atlas://` URI for this overview subject (VR1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}
