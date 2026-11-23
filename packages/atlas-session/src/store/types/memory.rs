use super::default_memory_metadata;
use super::*;

// ── ICM-A — Shared memory model ───────────────────────────────────────────────
// These types form the single memory record shape shared by CLI and MCP so the
// two surfaces cannot drift on defaults, validation, or visibility semantics.

/// Importance of a memory record. Exact values: `critical`, `high`, `normal`, `low`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryImportance {
    Critical,
    High,
    #[default]
    Normal,
    Low,
}

impl MemoryImportance {
    pub const ALL: [Self; 4] = [Self::Critical, Self::High, Self::Normal, Self::Low];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }
}

impl std::fmt::Display for MemoryImportance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MemoryImportance {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "critical" => Ok(Self::Critical),
            "high" => Ok(Self::High),
            "normal" => Ok(Self::Normal),
            "low" => Ok(Self::Low),
            other => Err(AtlasError::Other(format!(
                "unknown memory importance: {other}; expected one of critical, high, normal, low"
            ))),
        }
    }
}

/// Visibility scope of a memory record. Exact values: `project`, `session`,
/// `frontend`, `global`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    #[default]
    Project,
    Session,
    Frontend,
    Global,
}

impl MemoryScope {
    pub const ALL: [Self; 4] = [Self::Project, Self::Session, Self::Frontend, Self::Global];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Session => "session",
            Self::Frontend => "frontend",
            Self::Global => "global",
        }
    }
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "project" => Ok(Self::Project),
            "session" => Ok(Self::Session),
            "frontend" => Ok(Self::Frontend),
            "global" => Ok(Self::Global),
            other => Err(AtlasError::Other(format!(
                "unknown memory scope: {other}; expected one of project, session, frontend, global"
            ))),
        }
    }
}

/// One directed memory-map link stored in memory frontmatter.
///
/// `relation` uses the memoir controlled vocabulary plus `supersedes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryLink {
    /// Target memory id.
    pub id: String,
    /// Relation name (`depends_on`, `refines`, `supersedes`, ...).
    pub relation: String,
}

/// A stored memory record as persisted in the `memories` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub repo_root: String,
    pub session_id: Option<String>,
    pub frontend: Option<String>,
    pub scope: MemoryScope,
    pub topic: String,
    pub title: String,
    pub body: String,
    pub importance: MemoryImportance,
    pub created_at: String,
    pub updated_at: String,
    pub last_accessed_at: String,
    pub decay_score: f64,
    pub source_id: Option<String>,
    /// Free-form JSON metadata (column `metadata_json`).
    pub metadata: Value,
    /// Searchable labels; duplicate tags collapse on write.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Memory-map links to other memory records.
    #[serde(default)]
    pub links: Vec<MemoryLink>,
    /// Id of the consolidated memory that replaced this row (ICM-B3), or
    /// `None` when the row is still active. Derived from `supersedes` links at
    /// read time; never stored as truth.
    pub superseded_by: Option<String>,
}

/// Input shape for a manual memory write shared by CLI and MCP surfaces.
///
/// Defaults: `importance` is `normal`, `scope` is `project`. Call
/// [`NewMemory::validate`] before persisting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMemory {
    pub repo_root: String,
    pub session_id: Option<String>,
    pub frontend: Option<String>,
    #[serde(default)]
    pub scope: MemoryScope,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub importance: MemoryImportance,
    pub source_id: Option<String>,
    /// Searchable labels; duplicate tags collapse on write.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Memory-map links to other memory records.
    #[serde(default)]
    pub links: Vec<MemoryLink>,
    #[serde(default = "default_memory_metadata")]
    pub metadata: Value,
}

impl Default for NewMemory {
    fn default() -> Self {
        Self {
            repo_root: String::new(),
            session_id: None,
            frontend: None,
            scope: MemoryScope::default(),
            topic: String::new(),
            title: String::new(),
            body: String::new(),
            importance: MemoryImportance::default(),
            source_id: None,
            tags: Vec::new(),
            links: Vec::new(),
            metadata: default_memory_metadata(),
        }
    }
}

impl NewMemory {
    /// Rejects invalid memory writes before they reach storage: `frontend`
    /// scoped memories require a frontend identifier, `session` scoped
    /// memories require a session id, and the body must not be empty.
    pub fn validate(&self) -> atlas_core::Result<()> {
        if self.body.trim().is_empty() {
            return Err(AtlasError::Other(
                "memory body must not be empty".to_owned(),
            ));
        }
        if self.scope == MemoryScope::Frontend
            && self
                .frontend
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        {
            return Err(AtlasError::Other(
                "scope 'frontend' requires a frontend identifier".to_owned(),
            ));
        }
        if self.scope == MemoryScope::Session
            && self
                .session_id
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        {
            return Err(AtlasError::Other(
                "scope 'session' requires a session_id".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A recall hit pairing a memory record with its lexical match tier and
/// cross-session ranking evidence.
///
/// Lower `relevance_score` ranks higher: `0` = exact topic match, `1` =
/// topic/title contains match, `2` = body-only match, `3` = vector-only match
/// added by hybrid recall (ICM-E2). `recall_score` is the higher-is-better
/// composite score described by [`MemoryRecallRanking`]; the lexical tier
/// always dominates so exact keyword evidence is never buried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySearchHit {
    pub memory: MemoryRecord,
    pub relevance_score: i32,
    /// Raw FTS5 BM25 value for primary-path hits; `0.0` for auxiliary hits.
    #[serde(default)]
    pub bm25_score: f64,
    pub recall_score: f64,
    pub ranking: MemoryRecallRanking,
}

/// Deterministic ranking evidence for one recall hit (ICM-E1).
///
/// Ranking is lexical-first: `topic_match_tier` dominates, then importance,
/// broad scope visibility, source-backed evidence, and recency. Lower ranks
/// are stronger for every `*_rank` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecallRanking {
    /// Human-readable match tier: `exact_topic`, `topic_or_title`, or `body`.
    pub topic_match: String,
    /// Numeric match tier (0 = exact topic, 1 = topic/title, 2 = body).
    pub topic_match_tier: u8,
    /// Importance rank (0 = critical, 1 = high, 2 = normal, 3 = low).
    pub importance_rank: u8,
    /// Scope-visibility rank (0 = global, 1 = project, 2 = frontend, 3 = session).
    pub scope_rank: u8,
    /// Whether the memory carries a non-empty `source_id`.
    pub source_backed: bool,
    /// Whether consolidation superseded this memory.
    pub superseded: bool,
    /// Whole days since `updated_at` (0 when unknown or in the future).
    pub age_days: u32,
    /// Normalized origin score combining lexical evidence, importance, scope,
    /// and source-backed evidence (VM3).
    #[serde(default)]
    pub origin_score: f64,
    /// Freshness score: `1.0` inside the freshness window, then
    /// `1 / (1 + age_days / 30)`.
    #[serde(default)]
    pub time_score: f64,
    /// Business-importance score (`critical = 1.0`, `high = 0.75`,
    /// `normal = 0.5`, `low = 0.25`).
    #[serde(default)]
    pub importance_score: f64,
    /// Which recall path produced this hit: `primary`, `auxiliary`, or
    /// `vector` (ICM-E2 hybrid recall only).
    #[serde(default)]
    pub recall_path: String,
    /// Cosine similarity against the configured embedding model when the
    /// hybrid path scored this memory; `None` for lexical-only recall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_score: Option<f64>,
    /// Reciprocal-rank-fusion score for hybrid recall; `None` for lexical-only
    /// recall. Exact lexical tiers still outrank vector-only hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid_score: Option<f64>,
}

/// Filters shared by memory recall and list surfaces.
///
/// Timestamps are normalized RFC 3339 strings (second precision) so string
/// comparison equals chronological comparison.
#[derive(Debug, Clone, Default)]
pub struct MemoryListFilter {
    /// Case-insensitive exact topic match.
    pub topic: Option<String>,
    pub importance: Option<MemoryImportance>,
    pub scope: Option<MemoryScope>,
    /// Only memories updated before this timestamp.
    pub older_than: Option<String>,
    /// Only memories updated after this timestamp.
    pub newer_than: Option<String>,
    /// Include rows already superseded by consolidation (ICM-B3). Default
    /// `false`: superseded rows are hidden from normal listing.
    pub include_superseded: bool,
    /// Every tag in this list must be present (case-insensitive exact match).
    pub tags: Vec<String>,
}

/// Outcome of a memory delete, including dry-run inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDeleteResult {
    pub memory_id: String,
    /// Whether a memory with the exact id exists in this repo.
    pub found: bool,
    /// Whether a row was actually removed (false for dry-run).
    pub deleted: bool,
    pub dry_run: bool,
}

/// Identifies who is viewing memories so recall can enforce visibility rules.
///
/// Visibility (ICM-A3): `global` visible everywhere, `project` visible to all
/// frontends in the repo, `session` visible only to the same session, and
/// `frontend` visible only to the same repo plus the same frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryViewer {
    /// Canonical frontend identity (`claude`, `codex`, `copilot`, `cli`, `mcp`).
    pub frontend: String,
    /// Viewer session id; only `session`-scoped memories with the same id are visible.
    pub session_id: String,
}

/// Recall scoring and quota policy mirrored from `[memory.recall]` (VM3).
///
/// Lexical recall stays the default; the auxiliary memoir path only widens the
/// candidate set and never pools its raw scores with the primary path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryRecallPolicy {
    /// Memories updated within this many days keep `time_score = 1.0`.
    pub freshness_window_days: u32,
    /// Composite weight for the freshness signal (`0..=1`).
    pub weight_time: f64,
    /// Composite weight for the business-importance signal (`0..=1`).
    pub weight_business: f64,
    /// Fraction of `limit` reserved for the primary lexical path (`0..=1`).
    pub primary_quota_ratio: f64,
    /// Maximum memoir relation hops for the auxiliary path (`1..=2`).
    pub aux_max_hops: u32,
}

impl Default for MemoryRecallPolicy {
    fn default() -> Self {
        Self {
            freshness_window_days: DEFAULT_RECALL_FRESHNESS_WINDOW_DAYS,
            weight_time: DEFAULT_RECALL_WEIGHT_TIME,
            weight_business: DEFAULT_RECALL_WEIGHT_BUSINESS,
            primary_quota_ratio: DEFAULT_RECALL_PRIMARY_QUOTA_RATIO,
            aux_max_hops: DEFAULT_RECALL_AUX_MAX_HOPS,
        }
    }
}

impl MemoryRecallPolicy {
    /// Validates weights, quota, and hop bounds; used by `Config::load`.
    pub fn validate(&self) -> atlas_core::Result<()> {
        if !self.weight_time.is_finite() || !(0.0..=1.0).contains(&self.weight_time) {
            return Err(AtlasError::Other(
                "memory.recall.weight_time must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if !self.weight_business.is_finite() || !(0.0..=1.0).contains(&self.weight_business) {
            return Err(AtlasError::Other(
                "memory.recall.weight_business must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if self.weight_time + self.weight_business >= 1.0 {
            return Err(AtlasError::Other(
                "memory.recall.weight_time + memory.recall.weight_business must be less than 1.0"
                    .to_owned(),
            ));
        }
        if !self.primary_quota_ratio.is_finite() || !(0.0..=1.0).contains(&self.primary_quota_ratio)
        {
            return Err(AtlasError::Other(
                "memory.recall.primary_quota_ratio must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if !(1..=2).contains(&self.aux_max_hops) {
            return Err(AtlasError::Other(
                "memory.recall.aux_max_hops must be 1 or 2".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Default freshness tolerance window in days (`[memory.recall]`).
pub const DEFAULT_RECALL_FRESHNESS_WINDOW_DAYS: u32 = 1;
/// Default composite weight for freshness.
pub const DEFAULT_RECALL_WEIGHT_TIME: f64 = 0.25;
/// Default composite weight for business importance.
pub const DEFAULT_RECALL_WEIGHT_BUSINESS: f64 = 0.15;
/// Default quota reserved for the primary lexical path.
pub const DEFAULT_RECALL_PRIMARY_QUOTA_RATIO: f64 = 0.75;
/// Default maximum memoir hops for the auxiliary path.
pub const DEFAULT_RECALL_AUX_MAX_HOPS: u32 = 2;
/// Default reciprocal-rank-fusion weight for the lexical path.
pub const DEFAULT_HYBRID_WEIGHT_FTS: f64 = 0.7;
/// Default reciprocal-rank-fusion weight for the vector path.
pub const DEFAULT_HYBRID_WEIGHT_VECTOR: f64 = 0.3;
/// RRF k constant shared with hybrid graph retrieval.
pub const MEMORY_HYBRID_RRF_K: u32 = 60;
/// Minimum cosine similarity for a vector-only recall candidate.
pub const DEFAULT_MEMORY_VECTOR_MIN_SIMILARITY: f64 = 0.30;
/// Upper bound on memories scanned by the vector path per recall call.
pub const MEMORY_VECTOR_SCAN_LIMIT: usize = 4096;

/// Hybrid recall fusion policy mirrored from `[memory.search]` (ICM-E2).
///
/// The lexical path stays authoritative: fusion only reranks lexical hits and
/// appends vector-only matches after them. Raw lexical and vector scores are
/// never pooled into one number.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEmbeddingPolicy {
    /// RRF weight for the lexical ranking (`0..=1`).
    pub weight_fts: f64,
    /// RRF weight for the vector ranking (`0..=1`).
    pub weight_vector: f64,
    /// RRF k constant; higher values flatten rank-position sensitivity.
    pub rrf_k: u32,
    /// Minimum cosine similarity before a vector-only hit is emitted.
    pub min_similarity: f64,
}

impl Default for MemoryEmbeddingPolicy {
    fn default() -> Self {
        Self {
            weight_fts: DEFAULT_HYBRID_WEIGHT_FTS,
            weight_vector: DEFAULT_HYBRID_WEIGHT_VECTOR,
            rrf_k: MEMORY_HYBRID_RRF_K,
            min_similarity: DEFAULT_MEMORY_VECTOR_MIN_SIMILARITY,
        }
    }
}

impl MemoryEmbeddingPolicy {
    /// Validates weights and the RRF constant.
    pub fn validate(&self) -> atlas_core::Result<()> {
        if !self.weight_fts.is_finite() || !(0.0..=1.0).contains(&self.weight_fts) {
            return Err(AtlasError::Other(
                "memory.search.hybrid_weight_fts must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if !self.weight_vector.is_finite() || !(0.0..=1.0).contains(&self.weight_vector) {
            return Err(AtlasError::Other(
                "memory.search.hybrid_weight_vector must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if self.weight_fts + self.weight_vector <= 0.0 {
            return Err(AtlasError::Other(
                "memory.search.hybrid_weight_fts + memory.search.hybrid_weight_vector must be greater than 0.0".to_owned(),
            ));
        }
        if self.rrf_k == 0 {
            return Err(AtlasError::Other(
                "memory.search.rrf_k must be greater than 0".to_owned(),
            ));
        }
        if !self.min_similarity.is_finite() || !(0.0..=1.0).contains(&self.min_similarity) {
            return Err(AtlasError::Other(
                "memory.search.min_vector_similarity must be between 0.0 and 1.0".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Embedding coverage for one repo and model (ICM-E2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEmbeddingCoverage {
    /// Embedding model the coverage was measured for.
    pub embedding_model: String,
    /// Vector dimension stored for that model.
    pub dimension: usize,
    /// Number of recallable memories with a compatible stored vector.
    pub embedded: usize,
    /// Number of recallable memories without a compatible stored vector.
    pub missing: usize,
}

// ── ICM-B — memory curation (decay, stale, prune, health, consolidation) ──────

/// Default retention (days) per importance when `memory.decay` config is absent.
pub const DEFAULT_DECAY_LOW_DAYS: u32 = 30;
pub const DEFAULT_DECAY_NORMAL_DAYS: u32 = 90;
pub const DEFAULT_DECAY_HIGH_DAYS: u32 = 365;

/// Topic entry count above which `atlas memory health` reports the topic as noisy.
pub const NOISY_TOPIC_ENTRIES: usize = 10;

/// Body length (characters) above which `atlas memory health` reports the
/// memory as oversized.
pub const OVERSIZED_BODY_CHARS: usize = 2000;

/// Decay policy mirrored from `[memory.decay]` in `.atlas/config.toml`.
///
/// `decay_score` grows from `0.0` (fresh) toward `1.0` (pruneable) as a
/// memory ages past its retention window. `critical` memories are protected:
/// they never decay and are never auto-prune candidates unless
/// `critical_never_prune` is explicitly disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryDecayPolicy {
    pub enabled: bool,
    pub low_days: u32,
    pub normal_days: u32,
    pub high_days: u32,
    pub critical_never_prune: bool,
}

impl Default for MemoryDecayPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            low_days: DEFAULT_DECAY_LOW_DAYS,
            normal_days: DEFAULT_DECAY_NORMAL_DAYS,
            high_days: DEFAULT_DECAY_HIGH_DAYS,
            critical_never_prune: true,
        }
    }
}

impl MemoryDecayPolicy {
    /// Retention window in days for an importance level; `None` means the row
    /// never decays (critical + never-prune).
    pub fn retention_days(&self, importance: MemoryImportance) -> Option<u32> {
        match importance {
            MemoryImportance::Critical if self.critical_never_prune => None,
            MemoryImportance::Critical | MemoryImportance::High => Some(self.high_days),
            MemoryImportance::Normal => Some(self.normal_days),
            MemoryImportance::Low => Some(self.low_days),
        }
    }

    /// Whether the row is protected from auto-pruning by policy.
    pub fn is_protected(&self, importance: MemoryImportance) -> bool {
        importance == MemoryImportance::Critical && self.critical_never_prune
    }

    /// Updated decay score for a memory of `importance` that is `age_days` old.
    ///
    /// Protected critical memories always score `0.0`; everything else decays
    /// linearly to `1.0` at the end of its retention window.
    pub fn score(&self, importance: MemoryImportance, age_days: f64) -> f64 {
        let Some(retention) = self.retention_days(importance) else {
            return 0.0;
        };
        (age_days / f64::from(retention)).clamp(0.0, 1.0)
    }
}

/// Per-memory decay computation from `atlas memory decay`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryDecayReport {
    pub memory: MemoryRecord,
    /// Age of the memory in days, derived from `updated_at`.
    pub age_days: f64,
    /// Retention window in days for this memory's importance; `None` when
    /// policy protects the row from ever decaying (critical + never-prune).
    pub retention_days: Option<u32>,
    /// Score that would be (or was) written back when not dry-running.
    pub updated_decay_score: f64,
    /// Whether policy protects this row from auto-pruning.
    pub protected: bool,
    /// Whether the row is past its retention window and not protected.
    pub stale: bool,
}

/// Outcome of `atlas memory prune` (dry-run or applied).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryPruneResult {
    pub dry_run: bool,
    /// Rows past their retention window and selected by filters.
    pub candidate_count: usize,
    /// Rows actually deleted (0 for dry-run).
    pub deleted_count: usize,
    /// Critical rows skipped because `critical_never_prune` is on and no
    /// explicit override was given.
    pub protected_count: usize,
    pub candidates: Vec<MemoryRecord>,
}

/// Deterministic health categories from `atlas memory health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryHealthCategory {
    Healthy,
    Stale,
    Noisy,
    Duplicated,
    Orphaned,
    Oversized,
}

impl MemoryHealthCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Stale => "stale",
            Self::Noisy => "noisy",
            Self::Duplicated => "duplicated",
            Self::Orphaned => "orphaned",
            Self::Oversized => "oversized",
        }
    }
}

impl std::fmt::Display for MemoryHealthCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One deterministic health finding with an actionable follow-up command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryHealthFinding {
    pub category: MemoryHealthCategory,
    /// Stable sub-kind: `stale_memory`, `duplicated_memory`, `orphaned_source`,
    /// `oversized_memory`, `noisy_topic`, `topic_without_critical`.
    pub kind: String,
    /// Memory id the finding applies to; `None` for topic-level findings.
    pub memory_id: Option<String>,
    /// Topic the finding applies to.
    pub topic: Option<String>,
    pub detail: String,
    pub suggestion: String,
    /// Exact follow-up command for the human output.
    pub command: String,
}

/// Deterministic health report from `atlas memory health`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryHealthReport {
    pub total_memories: usize,
    pub findings: Vec<MemoryHealthFinding>,
    /// Count of findings per category (healthy rows are never findings).
    pub by_category: std::collections::BTreeMap<String, usize>,
}

/// One deterministic consolidation group from `atlas memory consolidate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryConsolidationGroup {
    pub topic: String,
    /// Deterministic grouping reason: `same_title`, `same_body`, `same_source`,
    /// or `same_category`.
    pub group_key: String,
    /// Representative memory kept as the consolidated row.
    pub kept_memory_id: String,
    /// Other rows merged into the consolidated row.
    pub merged_memory_ids: Vec<String>,
    /// Every distinct non-null `source_id` in the group (preserved in the
    /// consolidated record metadata).
    pub source_ids: Vec<String>,
    /// Deterministic synthesized body containing every distinct fact from all
    /// members in chronological order (truncated on overflow).
    pub body: String,
    /// Full merged metadata JSON written to the consolidated row.
    pub metadata_json: String,
    /// Every distinct member `source_id` preserved by consolidation (same
    /// value set as [`Self::source_ids`], kept for JSON consumers).
    pub merged_source_ids: Vec<String>,
    /// Set when the synthesized body exceeded `OVERSIZED_BODY_CHARS` and the
    /// full text was routed to `context.db`.
    pub overflow_source_id: Option<String>,
    /// Id of the consolidated memory; `Some` only after apply mode ran.
    pub consolidated_id: Option<String>,
}

/// Consolidation plan from `atlas memory consolidate` (dry-run or applied).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryConsolidationPlan {
    pub dry_run: bool,
    pub groups: Vec<MemoryConsolidationGroup>,
    pub kept_ids: Vec<String>,
    pub merged_ids: Vec<String>,
}

/// One persisted supersession link (table `memory_supersessions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemorySupersessionLink {
    pub old_memory_id: String,
    pub new_memory_id: String,
    pub reason: String,
    pub created_at: String,
}
