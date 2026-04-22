use serde::{Deserialize, Serialize};

use crate::kinds::{EdgeKind, NodeKind};

/// Opaque primary key for a graph node.
///
/// `NodeId(0)` is the sentinel value used before a database ID is assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub i64);

impl NodeId {
    /// Sentinel used before a real database ID has been assigned.
    pub const UNSET: NodeId = NodeId(0);
}

impl From<i64> for NodeId {
    fn from(v: i64) -> Self {
        NodeId(v)
    }
}

impl From<NodeId> for i64 {
    fn from(id: NodeId) -> Self {
        id.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub language: String,
    pub parent_name: Option<String>,
    pub params: Option<String>,
    pub return_type: Option<String>,
    pub modifiers: Option<String>,
    pub is_test: bool,
    pub file_hash: String,
    pub extra_json: serde_json::Value,
}

impl Node {
    /// Build a concise text representation suitable for semantic embedding.
    ///
    /// Format: `{kind} {name}[({params})][ -> {return_type}]  [{qualified_name}]`
    /// This provides a dense, symbol-level chunk for vector retrieval.
    pub fn chunk_text(&self) -> String {
        let mut out = format!("{} {}", self.kind.as_str(), self.name);
        if let Some(p) = &self.params
            && !p.is_empty()
        {
            out.push('(');
            out.push_str(p);
            out.push(')');
        }
        if let Some(r) = &self.return_type
            && !r.is_empty()
        {
            out.push_str(" -> ");
            out.push_str(r);
        }
        out.push_str("  [");
        out.push_str(&self.qualified_name);
        out.push(']');
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: i64,
    pub kind: EdgeKind,
    pub source_qn: String,
    pub target_qn: String,
    pub file_path: String,
    pub line: Option<u32>,
    pub confidence: f32,
    pub confidence_tier: Option<String>,
    pub extra_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub language: Option<String>,
    pub hash: String,
    pub size: Option<i64>,
    pub indexed_at: String,
    pub owner_id: Option<String>,
    pub owner_kind: Option<String>,
    pub owner_root: Option<String>,
    pub owner_manifest_path: Option<String>,
    pub owner_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOwnerKind {
    Cargo,
    Npm,
    Go,
}

impl PackageOwnerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Go => "go",
        }
    }
}

impl std::fmt::Display for PackageOwnerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageOwner {
    pub owner_id: String,
    pub kind: PackageOwnerKind,
    pub root: String,
    pub manifest_path: String,
    pub package_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub file_count: i64,
    pub node_count: i64,
    pub edge_count: i64,
    pub nodes_by_kind: Vec<(String, i64)>,
    pub languages: Vec<String>,
    pub last_indexed_at: Option<String>,
}

/// Compact provenance snapshot attached to every MCP tool response (MCP7).
///
/// Intentionally kept minimal: two SQL queries, no breakdown tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceMeta {
    pub indexed_file_count: i64,
    pub last_indexed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub change_type: ChangeType,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactResult {
    pub changed_nodes: Vec<Node>,
    pub impacted_nodes: Vec<Node>,
    pub impacted_files: Vec<String>,
    pub relevant_edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewContext {
    pub changed_files: Vec<String>,
    pub changed_symbols: Vec<Node>,
    pub changed_symbol_summaries: Vec<ChangedSymbolSummary>,
    pub impacted_neighbors: Vec<Node>,
    pub critical_edges: Vec<Edge>,
    pub impact_overview: ReviewImpactOverview,
    pub risk_summary: RiskSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedSymbolSummary {
    pub node: Node,
    pub callers: Vec<Node>,
    pub callees: Vec<Node>,
    pub importers: Vec<Node>,
    pub tests: Vec<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewImpactOverview {
    pub max_depth: u32,
    pub max_nodes: usize,
    pub impacted_node_count: usize,
    pub impacted_file_count: usize,
    pub relevant_edge_count: usize,
    pub reached_node_limit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummary {
    pub changed_symbol_count: usize,
    pub public_api_changes: usize,
    pub test_adjacent: bool,
    pub affected_test_count: usize,
    pub uncovered_changed_symbol_count: usize,
    pub large_function_touched: bool,
    pub large_function_count: usize,
    pub cross_module_impact: bool,
    pub cross_package_impact: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub kind: Option<String>,
    pub language: Option<String>,
    pub file_path: Option<String>,
    /// Filter results whose `file_path` starts with this subpath prefix.
    pub subpath: Option<String>,
    pub is_test: Option<bool>,
    pub limit: usize,
    /// Expand FTS seed results through graph edges.
    pub graph_expand: bool,
    /// Maximum edge hops when `graph_expand` is true (default: 1).
    pub graph_max_hops: u32,
    /// Reference file path for same-directory boost. When set, results in the
    /// same directory as this file receive a ranking bonus.
    pub reference_file: Option<String>,
    /// Reference language for same-language boost. When set, results in the
    /// same language receive a ranking bonus.
    pub reference_language: Option<String>,
    /// Enable fuzzy (edit-distance) name matching boost (+4). Off by default
    /// because it adds O(results) edit-distance work.
    pub fuzzy_match: bool,
    /// Boost nodes whose file was among the most recently indexed (+4). Requires
    /// one extra DB read inside `atlas_search::search`; off by default.
    pub recent_file_boost: bool,
    /// Boost nodes whose file appears in this set of changed file paths (+5).
    /// Caller populates this with the paths from the current git diff.
    /// Empty vec disables the boost.
    pub changed_files: Vec<String>,
    /// Enable hybrid (FTS + vector) retrieval.
    ///
    /// When `true` and `ATLAS_EMBED_URL` is set, the search layer runs both
    /// FTS and vector retrieval and merges results with Reciprocal Rank Fusion.
    /// Falls back to FTS-only when no embedding backend is configured.
    pub hybrid: bool,
    /// FTS candidate pool size before RRF merge (default: 60).
    pub top_k_fts: usize,
    /// Vector candidate pool size before RRF merge (default: 60).
    pub top_k_vector: usize,
    /// Reciprocal Rank Fusion k constant (default: 60).
    pub rrf_k: u32,
    /// Optional regex pattern applied as a SQL-layer UDF filter (`atlas_regexp`) against
    /// `name` and `qualified_name`. When set and `text` is empty, the structural scan path
    /// is used instead of FTS5. When both `text` and `regex_pattern` are set, FTS5 runs
    /// first and the UDF filters the results inside SQLite.
    ///
    /// Patterns must be valid `regex` crate syntax. An invalid pattern is
    /// returned as an error rather than silently skipped.
    pub regex_pattern: Option<String>,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            kind: None,
            language: None,
            file_path: None,
            subpath: None,
            is_test: None,
            limit: 20,
            graph_expand: false,
            graph_max_hops: 1,
            reference_file: None,
            reference_language: None,
            fuzzy_match: false,
            recent_file_boost: false,
            changed_files: vec![],
            hybrid: false,
            top_k_fts: 60,
            top_k_vector: 60,
            rrf_k: 60,
            regex_pattern: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredNode {
    pub node: Node,
    pub score: f64,
}

// ---------------------------------------------------------------------------
// Flows and communities (post-MVP group/partition features)
// ---------------------------------------------------------------------------

/// A named ordered sequence of graph nodes.
///
/// Flows represent user-defined traversal paths or call chains.  Membership
/// is stored with soft references so it survives graph rebuilds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: i64,
    pub name: String,
    pub kind: Option<String>,
    pub description: Option<String>,
    pub extra_json: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// One node's participation in a flow with optional ordering and role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowMembership {
    pub flow_id: i64,
    /// Qualified name of the node.  Soft reference — survives node rebuild.
    pub node_qualified_name: String,
    pub position: Option<i64>,
    pub role: Option<String>,
    pub extra_json: serde_json::Value,
}

/// A named partition of graph nodes produced by a clustering algorithm.
///
/// Communities may nest: `parent_community_id` links child to parent.
/// The actual node members are stored in the `community_nodes` join table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Community {
    pub id: i64,
    pub name: String,
    pub algorithm: Option<String>,
    pub level: Option<i64>,
    pub parent_community_id: Option<i64>,
    pub extra_json: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// One node's membership in a community.  Soft reference on qualified name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityNode {
    pub community_id: i64,
    /// Qualified name of the node.  Soft reference — survives node rebuild.
    pub node_qualified_name: String,
}

/// All data produced by parsing one file, ready to be persisted.
///
/// `nodes` and `edges` carry `id = 0`; the store assigns real database IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFile {
    pub path: String,
    pub language: Option<String>,
    pub hash: String,
    pub size: Option<i64>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

// ---------------------------------------------------------------------------
// Advanced impact types (Slice 16)
// ---------------------------------------------------------------------------

/// Coarse risk level assigned to a change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
            RiskLevel::Critical => "critical",
        };
        f.write_str(s)
    }
}

/// How a symbol was changed in the current diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Public/visibility modifier changed — externally visible interface break.
    ApiChange,
    /// Parameter list or return type changed.
    SignatureChange,
    /// Body-only change; public interface is stable.
    InternalChange,
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ChangeKind::ApiChange => "api_change",
            ChangeKind::SignatureChange => "signature_change",
            ChangeKind::InternalChange => "internal_change",
        };
        f.write_str(s)
    }
}

/// A node enriched with a weighted impact score and optional change classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredImpactNode {
    pub node: Node,
    /// Higher = more impacted. Based on weighted edge traversal from seed nodes.
    pub impact_score: f64,
    /// Set only for nodes that are direct seed (changed) nodes.
    pub change_kind: Option<ChangeKind>,
}

/// Test-coverage impact: which tests are affected and which changed symbols
/// have no adjacent test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestImpactResult {
    /// Test nodes (`is_test = true` or `NodeKind::Test`) within the impact set.
    pub affected_tests: Vec<Node>,
    /// Changed nodes that have no `Tests`/`TestedBy` edge to any test node.
    pub uncovered_changed_nodes: Vec<Node>,
}

/// A detected cross-boundary impact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryViolation {
    pub kind: BoundaryKind,
    pub description: String,
    /// Representative qualified names crossing the boundary.
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    CrossModule,
    CrossPackage,
}

impl std::fmt::Display for BoundaryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BoundaryKind::CrossModule => "cross_module",
            BoundaryKind::CrossPackage => "cross_package",
        };
        f.write_str(s)
    }
}

/// Full advanced impact analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedImpactResult {
    /// Base impact radius (unchanged nodes + impacted nodes + edges).
    pub base: ImpactResult,
    /// Impacted nodes ranked by weighted traversal score (highest first).
    pub scored_nodes: Vec<ScoredImpactNode>,
    /// Overall risk level for this change set.
    pub risk_level: RiskLevel,
    /// Test-coverage impact.
    pub test_impact: TestImpactResult,
    /// Architecture boundary violations detected.
    pub boundary_violations: Vec<BoundaryViolation>,
}

// ---------------------------------------------------------------------------
// Phase 22 — Context Engine types (Slice 1)
// ---------------------------------------------------------------------------

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
        }
    }
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
}

impl TruncationMeta {
    /// No truncation applied.
    pub fn none() -> Self {
        Self {
            nodes_dropped: 0,
            edges_dropped: 0,
            files_dropped: 0,
            truncated: false,
        }
    }
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
/// content store contains artifacts relevant to the current query.  Only
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
    /// Truncated preview of the first chunk (≤ 512 chars); never the raw blob.
    pub preview: String,
    /// Opaque hint an agent can use to retrieve the full artifact,
    /// e.g. `"source_id=<id>"` for a future MCP `search_saved_context` call.
    pub retrieval_hint: String,
    /// Relevance score for ranking within this result (higher = more relevant).
    pub relevance_score: f32,
}

/// Output of the context engine for a single [`ContextRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResult {
    pub request: ContextRequest,
    pub nodes: Vec<SelectedNode>,
    pub edges: Vec<SelectedEdge>,
    pub files: Vec<SelectedFile>,
    pub truncation: TruncationMeta,
    /// Set when the target was ambiguous; contains ranked candidates.
    pub ambiguity: Option<AmbiguityMeta>,
    /// Focused workflow metadata for higher-level developer UX.
    pub workflow: Option<WorkflowSummary>,
    /// Relevant saved artifacts from the content store.
    ///
    /// Populated only when `request.include_saved_context` is `true` and a
    /// content store is provided to the engine.  Ordered by descending
    /// `relevance_score`.
    pub saved_context_sources: Vec<SavedContextSource>,
}

// ---------------------------------------------------------------------------
// Phase 23 — Autonomous Code Reasoning types
// ---------------------------------------------------------------------------

/// Confidence tier for reasoning outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceTier {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for ConfidenceTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConfidenceTier::High => "high",
            ConfidenceTier::Medium => "medium",
            ConfidenceTier::Low => "low",
        };
        f.write_str(s)
    }
}

/// How certain the engine is that a given node is impacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactClass {
    /// Direct call / import / test edge — definitely impacted.
    Definite,
    /// Inferred link or unresolved selector in same file/package.
    Probable,
    /// Textual or unresolved weak edge only.
    Weak,
}

impl std::fmt::Display for ImpactClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ImpactClass::Definite => "definite",
            ImpactClass::Probable => "probable",
            ImpactClass::Weak => "weak",
        };
        f.write_str(s)
    }
}

/// Coarse refactor-safety band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyBand {
    Safe,
    Caution,
    Risky,
}

impl std::fmt::Display for SafetyBand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SafetyBand::Safe => "safe",
            SafetyBand::Caution => "caution",
            SafetyBand::Risky => "risky",
        };
        f.write_str(s)
    }
}

/// Numeric refactor-safety score with a coarse band and human-readable reasons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyScore {
    /// 0.0 = most risky, 1.0 = safest.
    pub score: f64,
    pub band: SafetyBand,
    pub reasons: Vec<String>,
    pub suggested_validations: Vec<String>,
}

/// A single piece of supporting evidence for a reasoning result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningEvidence {
    pub key: String,
    pub value: String,
}

/// A warning attached to a reasoning result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningWarning {
    pub message: String,
    pub confidence: ConfidenceTier,
}

/// A node enriched with depth, impact class, and the edge kind that introduced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactedNode {
    pub node: Node,
    pub depth: u32,
    pub impact_class: ImpactClass,
    pub via_edge_kind: Option<EdgeKind>,
}

/// Full removal-impact result (23.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemovalImpactResult {
    pub seed: Vec<Node>,
    pub impacted_symbols: Vec<ImpactedNode>,
    pub impacted_files: Vec<String>,
    pub impacted_tests: Vec<Node>,
    pub relevant_edges: Vec<Edge>,
    /// Nodes that directly served as evidence for the impact conclusion.
    pub evidence_nodes: Vec<Node>,
    pub warnings: Vec<ReasoningWarning>,
    pub evidence: Vec<ReasoningEvidence>,
    /// Uncertainty flags: qualitative reasons the result may be incomplete.
    pub uncertainty_flags: Vec<String>,
}

/// A dead code candidate with its flagging reasons (23.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeCandidate {
    pub node: Node,
    pub reasons: Vec<String>,
    pub certainty: ConfidenceTier,
    /// Blockers that prevent automatic removal.
    pub blockers: Vec<String>,
}

/// Refactor-safety result for a single symbol (23.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorSafetyResult {
    pub node: Node,
    pub safety: SafetyScore,
    pub fan_in: usize,
    pub fan_out: usize,
    pub linked_test_count: usize,
    pub unresolved_edge_count: usize,
    pub evidence: Vec<ReasoningEvidence>,
}

/// Dependency-removal validation result (23.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRemovalResult {
    pub target_qname: String,
    pub removable: bool,
    pub blocking_references: Vec<Node>,
    /// Edges that directly block removal or serve as evidence.
    pub evidence_edges: Vec<Edge>,
    pub confidence: ConfidenceTier,
    pub suggested_cleanups: Vec<String>,
    pub evidence: Vec<ReasoningEvidence>,
    /// Uncertainty flags: qualitative reasons the result may be incomplete.
    pub uncertainty_flags: Vec<String>,
}

/// Scope of a reference relative to the definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceScope {
    SameFile,
    SameModule,
    CrossModule,
    Test,
}

/// One reference that would be affected by a rename (23.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameReference {
    pub node: Node,
    pub edge: Edge,
    pub scope: ReferenceScope,
}

/// Rename blast-radius preview (23.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePreviewResult {
    pub target: Node,
    pub new_name: String,
    pub affected_references: Vec<RenameReference>,
    pub affected_files: Vec<String>,
    pub risk_level: RiskLevel,
    pub collision_warnings: Vec<String>,
    pub manual_review_flags: Vec<String>,
}

/// Strength of test coverage for a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStrength {
    Direct,
    SameFile,
    SameModule,
    None,
}

/// Test-adjacency result for a single symbol (23.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestAdjacencyResult {
    pub symbol: Node,
    pub linked_tests: Vec<Node>,
    pub coverage_strength: CoverageStrength,
    pub recommendation: Option<String>,
}

/// Change-risk classification result (23.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRiskResult {
    pub risk_level: RiskLevel,
    pub contributing_factors: Vec<String>,
    pub suggested_review_focus: Vec<String>,
}

// ---------------------------------------------------------------------------
// Phase 24 — Smart Refactoring Core types
// ---------------------------------------------------------------------------

/// A category of refactoring operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefactorOperation {
    RenameSymbol {
        old_qname: String,
        new_name: String,
    },
    RemoveDeadCode {
        target_qname: String,
    },
    CleanImports {
        file_path: String,
    },
    ExtractFunctionCandidate {
        file_path: String,
        line_start: u32,
        line_end: u32,
    },
}

/// The kind of text transformation a single edit performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefactorEditKind {
    /// Rename an occurrence of an identifier.
    RenameOccurrence,
    /// Remove a contiguous line span (dead symbol body).
    RemoveSpan,
    /// Remove an import/use statement.
    RemoveImport,
}

/// A single deterministic text replacement applied to one file.
///
/// Line numbers are 1-based and inclusive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorEdit {
    pub file_path: String,
    /// 1-based start line (inclusive).
    pub line_start: u32,
    /// 1-based end line (inclusive).
    pub line_end: u32,
    pub old_text: String,
    pub new_text: String,
    pub edit_kind: RefactorEditKind,
}

/// Unified-diff patch for one file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorPatch {
    pub file_path: String,
    pub unified_diff: String,
}

/// All planned edits and metadata describing the full refactoring step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorPlan {
    pub operation: RefactorOperation,
    pub edits: Vec<RefactorEdit>,
    pub affected_files: Vec<String>,
    /// References that require human review (low-confidence, dynamic, cross-module).
    pub manual_review: Vec<String>,
    pub estimated_safety: SafetyBand,
}

/// Structured outcome of a post-apply (or simulated) validation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub manual_review: Vec<String>,
}

/// Full result of a dry-run or applied refactoring execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorDryRunResult {
    pub plan: RefactorPlan,
    pub patches: Vec<RefactorPatch>,
    pub validation: RefactorValidationResult,
    /// Number of files changed (or would be changed in dry-run).
    pub files_changed: usize,
    /// Total edits applied.
    pub edit_count: usize,
    /// `true` when no files were actually written.
    pub dry_run: bool,
}

/// A candidate block for extract-function analysis (detection only; no auto-apply).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractFunctionCandidate {
    pub file_path: String,
    /// 1-based start line of the block.
    pub line_start: u32,
    /// 1-based end line of the block.
    pub line_end: u32,
    pub proposed_inputs: Vec<String>,
    pub proposed_outputs: Vec<String>,
    /// Higher = better extraction candidate.
    pub difficulty_score: f64,
    pub score_reasons: Vec<String>,
}

/// High-level simulated impact of a planned refactoring before any files are written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedRefactorImpact {
    /// Qualified names of graph nodes within the blast radius.
    pub affected_symbols: Vec<String>,
    /// Files touched by the simulated edits or graph impact.
    pub affected_files: Vec<String>,
    /// 0.0 (most risky) to 1.0 (safest).
    pub safety_score: f64,
    /// Test nodes that may need re-running.
    pub nearby_tests: Vec<String>,
    /// Unresolved concerns that block a high-confidence apply.
    pub unresolved_risks: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::{EdgeKind, NodeKind};

    // -------------------------------------------------------------------------
    // NodeId
    // -------------------------------------------------------------------------

    #[test]
    fn node_id_serde_round_trip() {
        let id = NodeId(42);
        let json = serde_json::to_string(&id).unwrap();
        // transparent: serialises as a plain number
        assert_eq!(json, "42");
        let back: NodeId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn node_id_unset_sentinel() {
        assert_eq!(NodeId::UNSET.0, 0);
    }

    #[test]
    fn node_id_from_i64() {
        assert_eq!(NodeId::from(7_i64), NodeId(7));
        let raw: i64 = NodeId(99).into();
        assert_eq!(raw, 99);
    }

    // -------------------------------------------------------------------------
    // Node serialization
    // -------------------------------------------------------------------------

    fn sample_node() -> Node {
        Node {
            id: NodeId(1),
            kind: NodeKind::Function,
            name: "my_func".to_string(),
            qualified_name: "src/lib.rs::fn::my_func".to_string(),
            file_path: "src/lib.rs".to_string(),
            line_start: 10,
            line_end: 20,
            language: "rust".to_string(),
            parent_name: None,
            params: Some("(x: i32)".to_string()),
            return_type: Some("i32".to_string()),
            modifiers: None,
            is_test: false,
            file_hash: "abc123".to_string(),
            extra_json: serde_json::Value::Null,
        }
    }

    #[test]
    fn node_serde_round_trip() {
        let n = sample_node();
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, n.id);
        assert_eq!(back.kind, n.kind);
        assert_eq!(back.name, n.name);
        assert_eq!(back.qualified_name, n.qualified_name);
        assert_eq!(back.file_path, n.file_path);
        assert_eq!(back.line_start, n.line_start);
        assert_eq!(back.line_end, n.line_end);
        assert_eq!(back.language, n.language);
        assert_eq!(back.params, n.params);
        assert_eq!(back.return_type, n.return_type);
        assert_eq!(back.is_test, n.is_test);
        assert_eq!(back.file_hash, n.file_hash);
    }

    #[test]
    fn node_optional_fields_null_round_trip() {
        let mut n = sample_node();
        n.parent_name = None;
        n.params = None;
        n.return_type = None;
        n.modifiers = None;
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert!(back.parent_name.is_none());
        assert!(back.params.is_none());
        assert!(back.return_type.is_none());
        assert!(back.modifiers.is_none());
    }

    #[test]
    fn node_is_test_flag_preserved() {
        let mut n = sample_node();
        n.is_test = true;
        n.kind = NodeKind::Test;
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert!(back.is_test);
        assert_eq!(back.kind, NodeKind::Test);
    }

    // -------------------------------------------------------------------------
    // Edge serialization
    // -------------------------------------------------------------------------

    fn sample_edge() -> Edge {
        Edge {
            id: 0,
            kind: EdgeKind::Calls,
            source_qn: "src/a.rs::fn::caller".to_string(),
            target_qn: "src/b.rs::fn::callee".to_string(),
            file_path: "src/a.rs".to_string(),
            line: Some(15),
            confidence: 1.0,
            confidence_tier: Some("high".to_string()),
            extra_json: serde_json::Value::Null,
        }
    }

    #[test]
    fn edge_serde_round_trip() {
        let e = sample_edge();
        let json = serde_json::to_string(&e).unwrap();
        let back: Edge = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, e.id);
        assert_eq!(back.kind, e.kind);
        assert_eq!(back.source_qn, e.source_qn);
        assert_eq!(back.target_qn, e.target_qn);
        assert_eq!(back.file_path, e.file_path);
        assert_eq!(back.line, e.line);
        assert_eq!(back.confidence, e.confidence);
        assert_eq!(back.confidence_tier, e.confidence_tier);
    }

    #[test]
    fn edge_optional_line_none_round_trip() {
        let mut e = sample_edge();
        e.line = None;
        let json = serde_json::to_string(&e).unwrap();
        let back: Edge = serde_json::from_str(&json).unwrap();
        assert!(back.line.is_none());
    }

    #[test]
    fn edge_optional_confidence_tier_none_round_trip() {
        let mut e = sample_edge();
        e.confidence_tier = None;
        let json = serde_json::to_string(&e).unwrap();
        let back: Edge = serde_json::from_str(&json).unwrap();
        assert!(back.confidence_tier.is_none());
    }

    // -------------------------------------------------------------------------
    // Phase 22 Slice 1 — ContextRequest / ContextResult serde round-trips
    // -------------------------------------------------------------------------

    fn sample_context_request_symbol() -> ContextRequest {
        ContextRequest {
            intent: ContextIntent::Symbol,
            target: ContextTarget::QualifiedName {
                qname: "crate::module::my_fn".to_string(),
            },
            max_nodes: Some(50),
            max_edges: Some(100),
            max_files: Some(10),
            depth: Some(1),
            include_tests: true,
            include_imports: true,
            include_neighbors: false,
            include_code_spans: false,
            include_callers: true,
            include_callees: true,
            include_saved_context: false,
            session_id: None,
        }
    }

    #[test]
    fn context_intent_serde_variants() {
        for (intent, expected) in [
            (ContextIntent::Symbol, "\"symbol\""),
            (ContextIntent::File, "\"file\""),
            (ContextIntent::Review, "\"review\""),
            (ContextIntent::Impact, "\"impact\""),
        ] {
            let json = serde_json::to_string(&intent).unwrap();
            assert_eq!(json, expected);
            let back: ContextIntent = serde_json::from_str(&json).unwrap();
            assert_eq!(back, intent);
        }
    }

    #[test]
    fn context_target_qualified_name_round_trip() {
        let t = ContextTarget::QualifiedName {
            qname: "crate::foo::bar".to_string(),
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: ContextTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        // tag must be present
        assert!(json.contains("\"kind\":\"qualified_name\""));
    }

    #[test]
    fn context_target_symbol_name_round_trip() {
        let t = ContextTarget::SymbolName {
            name: "my_func".to_string(),
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: ContextTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        assert!(json.contains("\"kind\":\"symbol_name\""));
    }

    #[test]
    fn context_target_file_path_round_trip() {
        let t = ContextTarget::FilePath {
            path: "src/lib.rs".to_string(),
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: ContextTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        assert!(json.contains("\"kind\":\"file_path\""));
    }

    #[test]
    fn context_target_changed_files_round_trip() {
        let t = ContextTarget::ChangedFiles {
            paths: vec!["src/a.rs".to_string(), "src/b.rs".to_string()],
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: ContextTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        assert!(json.contains("\"kind\":\"changed_files\""));
    }

    #[test]
    fn context_request_round_trip() {
        let req = sample_context_request_symbol();
        let json = serde_json::to_string(&req).unwrap();
        let back: ContextRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.intent, req.intent);
        assert_eq!(back.target, req.target);
        assert_eq!(back.max_nodes, req.max_nodes);
        assert_eq!(back.max_edges, req.max_edges);
        assert_eq!(back.max_files, req.max_files);
        assert_eq!(back.depth, req.depth);
        assert_eq!(back.include_tests, req.include_tests);
        assert_eq!(back.include_imports, req.include_imports);
        assert_eq!(back.include_neighbors, req.include_neighbors);
    }

    #[test]
    fn context_request_default_round_trip() {
        let req = ContextRequest::default();
        let json = serde_json::to_string(&req).unwrap();
        let back: ContextRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.intent, ContextIntent::Symbol);
        assert!(back.max_nodes.is_none());
        assert!(back.depth.is_none());
        assert!(!back.include_tests);
        assert!(back.include_imports);
        assert!(!back.include_neighbors);
    }

    #[test]
    fn selection_reason_serde_variants() {
        let reasons = [
            (SelectionReason::DirectTarget, "\"direct_target\""),
            (SelectionReason::Caller, "\"caller\""),
            (SelectionReason::Callee, "\"callee\""),
            (SelectionReason::Importer, "\"importer\""),
            (SelectionReason::Importee, "\"importee\""),
            (
                SelectionReason::ContainmentSibling,
                "\"containment_sibling\"",
            ),
            (SelectionReason::TestAdjacent, "\"test_adjacent\""),
            (SelectionReason::ImpactNeighbor, "\"impact_neighbor\""),
        ];
        for (reason, expected) in reasons {
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, expected);
            let back: SelectionReason = serde_json::from_str(&json).unwrap();
            assert_eq!(back, reason);
        }
    }

    #[test]
    fn selected_node_round_trip() {
        let sn = SelectedNode {
            node: sample_node(),
            selection_reason: SelectionReason::Caller,
            distance: 1,
            relevance_score: 0.0,
        };
        let json = serde_json::to_string(&sn).unwrap();
        let back: SelectedNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.selection_reason, sn.selection_reason);
        assert_eq!(back.distance, sn.distance);
        assert_eq!(back.node.qualified_name, sn.node.qualified_name);
    }

    #[test]
    fn selected_edge_round_trip() {
        let se = SelectedEdge {
            edge: sample_edge(),
            selection_reason: SelectionReason::Callee,
            depth: None,
            relevance_score: 0.0,
        };
        let json = serde_json::to_string(&se).unwrap();
        let back: SelectedEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(back.selection_reason, se.selection_reason);
        assert_eq!(back.edge.source_qn, se.edge.source_qn);
    }

    #[test]
    fn selected_file_round_trip() {
        let sf = SelectedFile {
            path: "src/main.rs".to_string(),
            selection_reason: SelectionReason::DirectTarget,
            line_ranges: vec![(10, 20), (35, 50)],
            language: Some("rust".to_string()),
            node_count_included: 2,
        };
        let json = serde_json::to_string(&sf).unwrap();
        let back: SelectedFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, sf.path);
        assert_eq!(back.selection_reason, sf.selection_reason);
        assert_eq!(back.line_ranges, sf.line_ranges);
    }

    #[test]
    fn selected_file_empty_ranges_round_trip() {
        let sf = SelectedFile {
            path: "src/lib.rs".to_string(),
            selection_reason: SelectionReason::ImpactNeighbor,
            line_ranges: vec![],
            language: None,
            node_count_included: 0,
        };
        let json = serde_json::to_string(&sf).unwrap();
        let back: SelectedFile = serde_json::from_str(&json).unwrap();
        assert!(back.line_ranges.is_empty());
    }

    #[test]
    fn truncation_meta_none_round_trip() {
        let tm = TruncationMeta::none();
        let json = serde_json::to_string(&tm).unwrap();
        let back: TruncationMeta = serde_json::from_str(&json).unwrap();
        assert!(!back.truncated);
        assert_eq!(back.nodes_dropped, 0);
        assert_eq!(back.edges_dropped, 0);
        assert_eq!(back.files_dropped, 0);
    }

    #[test]
    fn truncation_meta_with_drops_round_trip() {
        let tm = TruncationMeta {
            nodes_dropped: 5,
            edges_dropped: 3,
            files_dropped: 1,
            truncated: true,
        };
        let json = serde_json::to_string(&tm).unwrap();
        let back: TruncationMeta = serde_json::from_str(&json).unwrap();
        assert!(back.truncated);
        assert_eq!(back.nodes_dropped, 5);
        assert_eq!(back.edges_dropped, 3);
        assert_eq!(back.files_dropped, 1);
    }

    #[test]
    fn ambiguity_meta_round_trip() {
        let am = AmbiguityMeta {
            query: "my_fn".to_string(),
            candidates: vec!["crate::a::my_fn".to_string(), "crate::b::my_fn".to_string()],
            resolved: false,
        };
        let json = serde_json::to_string(&am).unwrap();
        let back: AmbiguityMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.query, am.query);
        assert_eq!(back.candidates, am.candidates);
        assert!(!back.resolved);
    }

    #[test]
    fn ambiguity_meta_resolved_round_trip() {
        let am = AmbiguityMeta {
            query: "my_fn".to_string(),
            candidates: vec![],
            resolved: true,
        };
        let json = serde_json::to_string(&am).unwrap();
        let back: AmbiguityMeta = serde_json::from_str(&json).unwrap();
        assert!(back.resolved);
        assert!(back.candidates.is_empty());
    }

    #[test]
    fn context_result_round_trip() {
        let result = ContextResult {
            request: sample_context_request_symbol(),
            nodes: vec![SelectedNode {
                node: sample_node(),
                selection_reason: SelectionReason::DirectTarget,
                distance: 0,
                relevance_score: 0.0,
            }],
            edges: vec![SelectedEdge {
                edge: sample_edge(),
                selection_reason: SelectionReason::Caller,
                depth: None,
                relevance_score: 0.0,
            }],
            files: vec![SelectedFile {
                path: "src/lib.rs".to_string(),
                selection_reason: SelectionReason::DirectTarget,
                line_ranges: vec![(10, 20)],
                language: Some("rust".to_string()),
                node_count_included: 1,
            }],
            truncation: TruncationMeta::none(),
            ambiguity: None,
            workflow: Some(WorkflowSummary {
                headline: Some("Focus on helper callers".to_string()),
                high_impact_nodes: vec![WorkflowFocusNode {
                    qualified_name: "src/lib.rs::fn::helper".to_string(),
                    kind: "function".to_string(),
                    file_path: "src/lib.rs".to_string(),
                    relevance_score: 42.0,
                    selection_reason: "direct_target".to_string(),
                }],
                impacted_components: vec![WorkflowComponent {
                    label: "src".to_string(),
                    kind: "directory".to_string(),
                    changed_node_count: 1,
                    impacted_node_count: 1,
                    file_count: 1,
                    summary: "1 changed, 1 impacted".to_string(),
                }],
                call_chains: vec![WorkflowCallChain {
                    summary: "caller -> helper".to_string(),
                    steps: vec![
                        "src/main.rs::fn::caller".to_string(),
                        "src/lib.rs::fn::helper".to_string(),
                    ],
                    edge_kinds: vec!["calls".to_string()],
                }],
                ripple_effects: vec!["Change reaches one dependent component.".to_string()],
                noise_reduction: NoiseReductionSummary {
                    retained_nodes: 1,
                    retained_edges: 1,
                    retained_files: 1,
                    dropped_nodes: 0,
                    dropped_edges: 0,
                    dropped_files: 0,
                    rules_applied: vec!["omitted containment siblings".to_string()],
                },
            }),
            saved_context_sources: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: ContextResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.nodes.len(), 1);
        assert_eq!(back.edges.len(), 1);
        assert_eq!(back.files.len(), 1);
        assert!(back.ambiguity.is_none());
        assert!(!back.truncation.truncated);
        assert!(back.workflow.is_some());
    }

    #[test]
    fn context_result_with_ambiguity_round_trip() {
        let result = ContextResult {
            request: ContextRequest {
                intent: ContextIntent::Symbol,
                target: ContextTarget::SymbolName {
                    name: "parse".to_string(),
                },
                ..ContextRequest::default()
            },
            nodes: vec![],
            edges: vec![],
            files: vec![],
            truncation: TruncationMeta::none(),
            ambiguity: Some(AmbiguityMeta {
                query: "parse".to_string(),
                candidates: vec!["crate::a::parse".to_string(), "crate::b::parse".to_string()],
                resolved: false,
            }),
            workflow: None,
            saved_context_sources: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: ContextResult = serde_json::from_str(&json).unwrap();
        let amb = back.ambiguity.unwrap();
        assert_eq!(amb.candidates.len(), 2);
        assert!(!amb.resolved);
    }
}
