use super::is_false;
use super::*;

/// Structured explanation for why a node, edge, or saved artifact ranked where
/// it did inside bounded context assembly.
///
/// This contract is intentionally separate from search ranking evidence.
/// Search ranking evidence explains why a symbol or file matched retrieval.
/// Context ranking evidence explains why an item was included and ranked within
/// a context/review result after graph expansion, impact analysis, and
/// saved-context scoring.
///
/// Patch N3 extends this struct with normalized ranking signals applicable
/// across all retrieval surfaces (graph, content, saved context) so that
/// mixed-result sets are explainable rather than opaque.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextRankingEvidence {
    // --- core scores ---
    /// Initial relevance score before context-specific additive boosts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_score: Option<f32>,
    /// Final relevance score after all context-specific contributions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_score: Option<f32>,

    // --- N3: mixed-result surface discriminant ---
    /// Which retrieval surface produced this item.
    /// Allows callers to interpret other evidence fields correctly across
    /// heterogeneous (graph + content + saved-context) result sets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<MixedResultKind>,

    // --- graph selection reason flags ---
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

    // --- N3: normalized ranking signals (apply across graph and content) ---
    /// Item matched an exact symbol name derived from the request target.
    /// Set for content assets whose preview text contains an exact symbol name
    /// and for graph nodes that are the direct resolution of a symbol query.
    #[serde(default, skip_serializing_if = "is_false")]
    pub exact_symbol_match: bool,
    /// Raw BM25 / FTS content-match score for this item.
    /// Populated for content assets and saved-context sources retrieved via
    /// lexical search; absent for graph nodes selected by graph traversal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_score: Option<f32>,
    /// Additive boost applied because this item is in or adjacent to a
    /// changed file.  Set for content assets (adjacency) and graph nodes
    /// (direct-target or changed-symbol selection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_file_boost: Option<f32>,
    /// Additive boost applied for same-package or same-directory proximity
    /// relative to the request target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_package_dir_boost: Option<f32>,
    /// Score contribution from trigram or fuzzy-match correction applied to
    /// bring a near-miss symbol or path hit into the result set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigram_correction: Option<f32>,
    /// Additive boost from path-title proximity reranking — applied when an
    /// asset's path is similar to the target file or changed-file paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_proximity_boost: Option<f32>,
    /// Score penalty from graph hop distance from the seed node.
    /// Populated for graph nodes; absent for content / saved-context items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_distance_penalty: Option<f32>,
    /// Additive boost from session recency or relevance.
    /// Unifies `recent_source_boost` and `same_session_boost` under one
    /// normalized signal for mixed-result ranking; individual fields retain
    /// their original values for backward compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_recency_boost: Option<f32>,

    // --- legacy saved-context-specific signals (kept for compat) ---
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
    /// Provenance for a node selected through an experience edge (VR4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experience_shortcut: Option<ExperienceShortcutEvidence>,
}

/// Provenance attached to an experience-edge shortcut.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperienceShortcutEvidence {
    pub edge_id: String,
    pub confidence: f64,
    pub trace_summary: String,
    pub commit_sha: String,
}

pub type ContextScoreEvidence = ContextRankingEvidence;

impl ContextRankingEvidence {
    pub fn from_selection_reason(reason: SelectionReason) -> Self {
        let mut evidence = Self {
            source_kind: Some(MixedResultKind::GraphNode),
            ..Self::default()
        };
        // Graph nodes are the default surface kind for selection-reason
        // construction; callers building edge evidence should override with
        // `evidence.source_kind = Some(MixedResultKind::GraphEdge)`.
        match reason {
            SelectionReason::DirectTarget => evidence.direct_target = true,
            SelectionReason::Caller => evidence.caller_neighbor = true,
            SelectionReason::Callee => evidence.callee_neighbor = true,
            SelectionReason::TestAdjacent => evidence.test_adjacent = true,
            SelectionReason::Importer
            | SelectionReason::Importee
            | SelectionReason::ContainmentSibling
            | SelectionReason::ImpactNeighbor
            | SelectionReason::OverviewLevel
            | SelectionReason::ExperienceShortcut => {}
        }
        evidence
    }

    /// Override the source kind on an already-constructed evidence value.
    /// Useful when `from_selection_reason` is called for edges but the caller
    /// needs to tag the item as [`MixedResultKind::GraphEdge`].
    pub fn with_source_kind(mut self, kind: MixedResultKind) -> Self {
        self.source_kind = Some(kind);
        self
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
        "source_kind": "Which retrieval surface produced this item: graph_node, graph_edge, file_asset, content_match, template, text_asset, or saved_context. Allows interpretation of other evidence fields across heterogeneous mixed results.",
        "direct_target": "Item is the primary target resolved from the request.",
        "changed_symbol": "Item is a changed symbol selected from changed-file or impact review seeds.",
        "caller_neighbor": "Item was included as a caller of the target or neighbor.",
        "callee_neighbor": "Item was included as a callee of the target or neighbor.",
        "test_adjacent": "Item was included because of test adjacency.",
        "exact_symbol_match": "Item matched an exact symbol name from the request target (set for graph nodes that are direct resolutions and for content assets whose text contains an exact symbol name).",
        "bm25_score": "Raw BM25/FTS content-match score for content assets and saved-context sources retrieved via lexical search. Absent for graph nodes selected by graph traversal.",
        "changed_file_boost": "Quantified additive boost applied because item is in or adjacent to a changed file.",
        "same_package_dir_boost": "Additive boost applied for same-package or same-directory proximity relative to the request target.",
        "trigram_correction": "Score contribution from trigram or fuzzy-match correction bringing a near-miss symbol or path hit into the result.",
        "path_proximity_boost": "Additive boost from path-title proximity reranking when an asset path is similar to the target or changed-file paths.",
        "graph_distance_penalty": "Score penalty from graph hop distance from the seed node (graph nodes only).",
        "session_recency_boost": "Unified normalized signal for session recency/relevance across all surfaces. Mirrors recent_source_boost and same_session_boost for saved context; also set for content assets when session context is available.",
        "impact_score_contribution": "Additive contribution from impact analysis weighting in review or impact context.",
        "saved_context_rank_score": "Base saved-context ranking contribution derived from retrieval rank.",
        "recent_source_boost": "Additive boost for recently created saved-context artifacts (kept for backward compatibility; see also session_recency_boost).",
        "same_session_boost": "Additive boost for saved-context artifacts from the active session (kept for backward compatibility; see also session_recency_boost)."
    })
}
