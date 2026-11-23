use super::*;

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

// ---------------------------------------------------------------------------
// Graph/Content Companion Patch N2 — content asset types
// Graph/Content Companion Patch N3 — mixed-result ranking envelope
// ---------------------------------------------------------------------------

/// Discriminant identifying which retrieval surface produced an item in a
/// mixed graph/content context result.
///
/// Carried in [`ContextRankingEvidence::source_kind`] so callers can
/// understand which surface each result came from and interpret the other
/// evidence fields correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MixedResultKind {
    /// Symbol or type node from the code graph.
    GraphNode,
    /// Relationship edge from the code graph.
    GraphEdge,
    /// Source file entry tracked in the graph file index.
    FileAsset,
    /// Non-code asset matched via general content search (docs, readmes).
    ContentMatch,
    /// Template file (HTML, Jinja2, Mustache, Tera, Handlebars, etc.).
    Template,
    /// Text asset: SQL, config, env, or prompt file.
    TextAsset,
    /// Saved context artifact from a prior Atlas session.
    SavedContext,
}

impl MixedResultKind {
    /// Map a content asset `content_type` string to the appropriate surface
    /// kind.  Falls back to [`ContentMatch`][Self::ContentMatch] for unknown
    /// categories.
    pub fn from_content_type(content_type: &str) -> Self {
        match content_type {
            "template" => Self::Template,
            "sql" | "config" | "prompt" => Self::TextAsset,
            _ => Self::ContentMatch,
        }
    }
}

/// Why a content asset was included in a mixed graph/content context result.
///
/// Variants follow the unified selection ordering policy:
/// adjacent_to_changed_file > related_to_changed_symbol > content_match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentAssetReason {
    /// Asset is in the same directory as a changed file, or its path
    /// prefix matches a changed file path.
    AdjacentToChangedFile,
    /// Asset was retrieved because its content mentions a changed symbol name.
    RelatedToChangedSymbol,
    /// General content match from a query derived from the request context.
    ContentMatch,
}

impl ContentAssetReason {
    /// Numeric priority used for deterministic tie-breaking.
    /// Higher value = higher priority.
    pub fn priority(self) -> u8 {
        match self {
            Self::AdjacentToChangedFile => 3,
            Self::RelatedToChangedSymbol => 2,
            Self::ContentMatch => 1,
        }
    }
}
