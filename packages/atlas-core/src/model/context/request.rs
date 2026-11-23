use super::is_false;
use super::*;

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
    /// Allow caller/callee graph expansion to cross repo boundaries.
    /// Defaults to `false` so single-repo symbol context remains zero-config.
    pub allow_cross_repo_edges: bool,
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
    // --- Graph/Content Companion Patch N2: unified bounded selection ---
    /// When `true`, the engine queries the content store for non-code assets
    /// (docs, config, templates, SQL, prompts) adjacent to the request target
    /// or changed files and populates `ContextResult::content_assets`.
    /// Has no effect when no content store is provided to the engine.
    pub include_content_assets: bool,
    /// Maximum content assets to include. `None` uses the policy default.
    pub max_content_assets: Option<usize>,
    // --- ICM-G7: overview memory context integration ---
    /// When `true`, the engine includes matching overview records from the
    /// continuity store. Overview bodies stay preview-only; full bodies are
    /// retrieved through `source_id`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_overviews: bool,
    /// Maximum overview records to include. `None` uses the policy default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview_limit: Option<usize>,
    /// Allowed overview freshness states (e.g. `fresh`, `possibly_stale`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview_freshness: Option<Vec<String>>,
    /// Allowed overview subject kinds (e.g. `file`, `function`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview_subject_kinds: Option<Vec<String>>,
    /// Byte cap for overview previews. `None` uses the policy default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview_max_bytes: Option<usize>,
    // --- VR2: directory-semantic query scope ---
    /// Recursive canonical path prefix restricting target resolution and
    /// selected context to one subtree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    /// Non-recursive directory scope: only files directly inside this
    /// canonical directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_dir: Option<String>,
    /// Canonical subpath prefixes excluded from resolution and selection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_subpaths: Vec<String>,
    // --- R7.1: build telemetry ---
    /// When `true`, populate [`ContextResult::telemetry`] with requested vs
    /// emitted item counts, payload sizes, budget outcome, and duration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_telemetry: bool,
    // --- R7.2: payload metadata slimming ---
    /// When `true`, keep per-item `context_ranking_evidence` in the result.
    /// Defaults to `false` so emitted payloads carry only selection reasons and
    /// relevance scores unless callers opt in to ranking diagnostics.
    #[serde(default, skip_serializing_if = "is_false")]
    pub include_ranking_evidence: bool,
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
            allow_cross_repo_edges: false,
            include_saved_context: false,
            session_id: None,
            agent_id: None,
            merge_agent_partitions: false,
            token_budget: None,
            include_content_assets: false,
            max_content_assets: None,
            include_overviews: false,
            overview_limit: None,
            overview_freshness: None,
            overview_subject_kinds: None,
            overview_max_bytes: None,
            subpath: None,
            exact_dir: None,
            exclude_subpaths: Vec::new(),
            include_telemetry: false,
            include_ranking_evidence: false,
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
    /// Surfaced through an overview record at a specific disclosure level (VR1).
    OverviewLevel,
    /// Activated through a query-conditioned experience edge (VR4).
    ExperienceShortcut,
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
            Self::OverviewLevel => "overview_level",
            Self::ExperienceShortcut => "experience_shortcut",
        }
    }
}
