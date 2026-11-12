//! Atlas configuration: `.atlas/config.toml` schema, defaults, validation,
//! template rendering, and budget-policy mapping.
//!
//! Module root: `Config` struct, shared validation helpers, and per-family
//! submodules (`build`, `search`, `mcp`, `analysis`, `insights`,
//! `sanitization`, `context`, `load`, `template`, `accessors`, `budget`).

mod accessors;
mod analysis;
mod auto;
mod budget;
mod build;
mod context;
mod insights;
mod load;
mod mcp;
mod parsers;
mod reachability;
mod sanitization;
mod search;
mod template;

#[cfg(test)]
mod tests;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use analysis::{AnalysisConfig, FeedbackAdjustmentConfig};
pub use auto::{
    RepoEstimate, SystemSnapshot, estimate_build_wall_seconds, probe_repo, probe_system,
    render_auto_template, tuned_config,
};
pub use build::{BuildConfig, BuildRunBudget, DEFAULT_PARSE_BATCH_SIZE};
pub use context::{
    ContextConfig, ContextEscalationConfig, ContextExperienceConfig, ContextTokenizerConfig,
    TokenizerFallbackMode, TokenizerProvider,
};
pub use insights::{InsightsConfig, InsightsLayerRule};
pub use load::TokenCounterLoadResult;
pub use mcp::{
    DEFAULT_MCP_TOOL_TIMEOUT_MS, DEFAULT_MCP_WORKER_THREADS, McpConfig, McpHttpAuthConfig,
    ValidatedMcpHttpAuthConfig,
};
pub use parsers::ParsersConfig;
pub use reachability::{
    DEFAULT_REACHABILITY_BUDGET_MS, DEFAULT_REACHABILITY_MAX_OWNERS, ReachabilityConfig,
};
pub use sanitization::SanitizationConfig;
pub use search::{
    DEFAULT_EMBED_MAX_RETRIES, DEFAULT_EMBED_MODEL, DEFAULT_EMBED_RETRY_BACKOFF_MS,
    DEFAULT_EMBED_TIMEOUT_SECS, DEFAULT_TRIEHI_MIN_CHUNKS, RetrievalConfig, SearchConfig,
    SearchEmbeddingConfig,
};
pub use template::ConfigTemplateProfile;

/// Top-level atlas configuration loaded from `.atlas/config.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub retrieval: RetrievalConfig,
    #[serde(default)]
    pub analysis: AnalysisConfig,
    #[serde(default)]
    pub insights: InsightsConfig,
    #[serde(default)]
    pub sanitization: SanitizationConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub parsers: ParsersConfig,
    #[serde(default)]
    pub reachability: ReachabilityConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub overview: OverviewConfig,
}

/// `[overview]` external-analysis handoff configuration (ICM-G9).
///
/// Atlas never infers a model provider from environment variables. The
/// external analyzer command is either absent or an explicit command string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverviewConfig {
    #[serde(default = "default_overview_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub auto_export_on_commit: bool,
    #[serde(default = "default_overview_auto_mark_stale")]
    pub auto_mark_stale_on_commit: bool,
    #[serde(default)]
    pub external_analyzer_command: Option<String>,
    #[serde(default)]
    pub ingest_after_external_command: bool,
    #[serde(default = "default_overview_max_subjects")]
    pub max_subjects_per_run: usize,
    #[serde(default = "default_overview_max_packet_bytes")]
    pub max_packet_bytes: usize,
}

fn default_overview_enabled() -> bool {
    true
}

fn default_overview_auto_mark_stale() -> bool {
    true
}

fn default_overview_max_subjects() -> usize {
    50
}

fn default_overview_max_packet_bytes() -> usize {
    1_048_576
}

impl Default for OverviewConfig {
    fn default() -> Self {
        Self {
            enabled: default_overview_enabled(),
            auto_export_on_commit: false,
            auto_mark_stale_on_commit: default_overview_auto_mark_stale(),
            external_analyzer_command: None,
            ingest_after_external_command: false,
            max_subjects_per_run: default_overview_max_subjects(),
            max_packet_bytes: default_overview_max_packet_bytes(),
        }
    }
}

impl OverviewConfig {
    pub fn validate(&self) -> Result<()> {
        if self.max_subjects_per_run == 0 || self.max_subjects_per_run > 1_000 {
            anyhow::bail!(
                "invalid config: overview.max_subjects_per_run must be between 1 and 1000"
            );
        }
        if self.max_packet_bytes < 1_024 || self.max_packet_bytes > 64 * 1_024 * 1_024 {
            anyhow::bail!(
                "invalid config: overview.max_packet_bytes must be between 1024 and 67108864"
            );
        }
        if let Some(command) = self.external_analyzer_command.as_deref() {
            let trimmed = command.trim();
            if trimmed.is_empty() {
                anyhow::bail!(
                    "invalid config: overview.external_analyzer_command must not be empty"
                );
            }
            if trimmed.starts_with('$') {
                anyhow::bail!(
                    "invalid config: overview.external_analyzer_command must be an explicit command, not an environment variable reference"
                );
            }
        }
        Ok(())
    }
}

/// Memory surface configuration (ICM-A + ICM-B + ICM-D).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Allow arbitrary frontend identities beyond the known set
    /// (`claude`, `codex`, `copilot`, `cli`, `mcp`) for memory writes and
    /// visibility. Defaults to false: unknown frontends are rejected.
    #[serde(default)]
    pub allow_custom_frontends: bool,
    /// Memory decay and retention policy (ICM-B1). Safe defaults apply when
    /// the section is absent.
    #[serde(default)]
    pub decay: MemoryDecayConfig,
    /// Wake-up pack size budget (ICM-D1). Safe defaults apply when the
    /// section is absent.
    #[serde(default)]
    pub wake_up: WakeUpConfig,
    /// Two-path recall scoring and quota policy (VM3). Safe defaults apply
    /// when the section is absent.
    #[serde(default)]
    pub recall: MemoryRecallConfig,
    /// Optional embedding provider for semantic recall (ICM-E2). Disabled by
    /// default; lexical recall never requires it.
    #[serde(default)]
    pub embedding: MemoryEmbeddingConfig,
    /// Hybrid recall fusion weights (ICM-E2).
    #[serde(default)]
    pub search: MemorySearchConfig,
}

/// `[memory.decay]` retention policy for `atlas memory decay|stale|prune`.
///
/// `decay_score` grows toward `1.0` as a memory ages past its importance
/// retention window. `critical` memories never decay and are never
/// auto-prune candidates while `critical_never_prune` is true.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDecayConfig {
    /// Master switch; `false` disables decay scoring and pruning entirely.
    #[serde(default = "default_decay_enabled")]
    pub enabled: bool,
    /// Retention days for `low`-importance memories.
    #[serde(default = "default_decay_low_days")]
    pub low_days: u32,
    /// Retention days for `normal`-importance memories.
    #[serde(default = "default_decay_normal_days")]
    pub normal_days: u32,
    /// Retention days for `high`-importance (and unprotected `critical`) memories.
    #[serde(default = "default_decay_high_days")]
    pub high_days: u32,
    /// Never auto-prune `critical` memories; also keeps their decay score at 0.
    #[serde(default = "default_decay_critical_never_prune")]
    pub critical_never_prune: bool,
}

impl Default for MemoryDecayConfig {
    fn default() -> Self {
        Self {
            enabled: default_decay_enabled(),
            low_days: default_decay_low_days(),
            normal_days: default_decay_normal_days(),
            high_days: default_decay_high_days(),
            critical_never_prune: default_decay_critical_never_prune(),
        }
    }
}

fn default_decay_enabled() -> bool {
    true
}

fn default_decay_low_days() -> u32 {
    30
}

fn default_decay_normal_days() -> u32 {
    90
}

fn default_decay_high_days() -> u32 {
    365
}

fn default_decay_critical_never_prune() -> bool {
    true
}

impl MemoryDecayConfig {
    /// Validates retention days as positive integers; fails `atlas doctor`
    /// clearly through `Config::load` on invalid config.
    pub fn validate(&self) -> Result<()> {
        if self.enabled {
            validate_positive_u32("memory.decay.low_days", self.low_days)?;
            validate_positive_u32("memory.decay.normal_days", self.normal_days)?;
            validate_positive_u32("memory.decay.high_days", self.high_days)?;
        }
        Ok(())
    }
}

/// Hard ceiling shared with the wake-up budget policy in `atlas-agent-events`
/// (`WakeBudget::HARD_MAX_ITEMS`). Engine cannot depend on that crate, so the
/// ceiling is mirrored here; both must stay in sync.
pub const WAKE_UP_MAX_ITEMS_HARD_CAP: usize = 25;
/// Hard ceiling for feedback entries per wake-up pack.
pub const WAKE_UP_MAX_FEEDBACK_ITEMS_HARD_CAP: usize = 10;
/// Hard ceiling for pending-change listings per wake-up pack.
pub const WAKE_UP_MAX_PENDING_CHANGES_HARD_CAP: usize = 100;

/// `[memory.wake_up]` size budget for `atlas wake-up` and the MCP `wake_up`
/// tool (ICM-D1). Every list in the pack is bounded by these values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeUpConfig {
    /// Items per pack list (decisions, memories, concepts, changes, hints).
    #[serde(default = "default_wake_up_max_items")]
    pub max_items: usize,
    /// Feedback records surfaced per wake-up pack (smallest list by design).
    #[serde(default = "default_wake_up_max_feedback_items")]
    pub max_feedback_items: usize,
    /// Pending graph-relevant changes listed in the readiness block.
    #[serde(default = "default_wake_up_max_pending_changes")]
    pub max_pending_changes: usize,
}

impl Default for WakeUpConfig {
    fn default() -> Self {
        Self {
            max_items: default_wake_up_max_items(),
            max_feedback_items: default_wake_up_max_feedback_items(),
            max_pending_changes: default_wake_up_max_pending_changes(),
        }
    }
}

fn default_wake_up_max_items() -> usize {
    10
}

fn default_wake_up_max_feedback_items() -> usize {
    3
}

fn default_wake_up_max_pending_changes() -> usize {
    20
}

impl WakeUpConfig {
    /// Validates the wake-up size budget against hard caps; fails
    /// `atlas doctor` clearly through `Config::load` on invalid config.
    pub fn validate(&self) -> Result<()> {
        validate_usize_limit(
            "memory.wake_up.max_items",
            self.max_items,
            WAKE_UP_MAX_ITEMS_HARD_CAP,
        )?;
        validate_usize_limit(
            "memory.wake_up.max_feedback_items",
            self.max_feedback_items,
            WAKE_UP_MAX_FEEDBACK_ITEMS_HARD_CAP,
        )?;
        validate_usize_limit(
            "memory.wake_up.max_pending_changes",
            self.max_pending_changes,
            WAKE_UP_MAX_PENDING_CHANGES_HARD_CAP,
        )?;
        Ok(())
    }
}

impl MemoryConfig {
    /// Validates the memory surface config; used by `Config::load`.
    pub fn validate(&self) -> Result<()> {
        self.decay.validate()?;
        self.wake_up.validate()?;
        self.recall.validate()?;
        self.embedding.validate()?;
        self.search.validate()
    }
}

/// `[memory.embedding]` optional semantic recall configuration (ICM-E2).
///
/// Embeddings are opt-in: `enabled = false` (the default) keeps recall fully
/// lexical. When enabled, `provider` must name a supported backend and
/// `dimension` must match the vectors that backend returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEmbeddingConfig {
    /// Master switch; `false` keeps recall lexical-only.
    #[serde(default)]
    pub enabled: bool,
    /// Backend identifier. Supported value: `search` (reuse `[search.embedding]`).
    #[serde(default)]
    pub provider: Option<String>,
    /// Embedding model name recorded next to stored vectors.
    #[serde(default = "default_memory_embedding_model")]
    pub model: String,
    /// Expected vector dimension; mismatches are rejected on write and read.
    #[serde(default = "default_memory_embedding_dimension")]
    pub dimension: usize,
}

impl Default for MemoryEmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            model: default_memory_embedding_model(),
            dimension: default_memory_embedding_dimension(),
        }
    }
}

fn default_memory_embedding_model() -> String {
    DEFAULT_EMBED_MODEL.to_owned()
}

fn default_memory_embedding_dimension() -> usize {
    // Matches `all-minilm` (all-MiniLM-L6-v2), the default CPU-friendly model.
    384
}

/// Supported `memory.embedding.provider` values.
pub const MEMORY_EMBEDDING_PROVIDERS: &[&str] = &["search"];
/// Upper bound accepted for `memory.embedding.dimension`.
pub const MEMORY_EMBEDDING_MAX_DIMENSION: usize = 8192;

impl MemoryEmbeddingConfig {
    /// Validates the opt-in shape; fails `atlas doctor` clearly through
    /// `Config::load` on invalid config.
    pub fn validate(&self) -> Result<()> {
        if self.dimension == 0 || self.dimension > MEMORY_EMBEDDING_MAX_DIMENSION {
            anyhow::bail!(
                "invalid config: memory.embedding.dimension must be between 1 and {MEMORY_EMBEDDING_MAX_DIMENSION}"
            );
        }
        if let Some(provider) = self.provider.as_deref() {
            let provider = provider.trim();
            if provider.is_empty() {
                anyhow::bail!(
                    "invalid config: memory.embedding.provider must not be empty when set"
                );
            }
            if !MEMORY_EMBEDDING_PROVIDERS.contains(&provider) {
                anyhow::bail!(
                    "invalid config: unsupported memory.embedding.provider {provider:?}; supported providers: {}",
                    MEMORY_EMBEDDING_PROVIDERS.join(", ")
                );
            }
        }
        if self.enabled {
            if self.provider.is_none() {
                anyhow::bail!(
                    "invalid config: memory.embedding.provider is required when memory.embedding.enabled = true"
                );
            }
            if self.model.trim().is_empty() {
                anyhow::bail!(
                    "invalid config: memory.embedding.model must not be empty when memory.embedding.enabled = true"
                );
            }
        } else if self.model.trim().is_empty() {
            anyhow::bail!("invalid config: memory.embedding.model must not be empty");
        }
        Ok(())
    }
}

/// `[memory.search]` hybrid fusion weights (ICM-E2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySearchConfig {
    /// RRF weight for the lexical ranking (`0..=1`).
    #[serde(default = "default_hybrid_weight_fts")]
    pub hybrid_weight_fts: f64,
    /// RRF weight for the vector ranking (`0..=1`).
    #[serde(default = "default_hybrid_weight_vector")]
    pub hybrid_weight_vector: f64,
    /// Minimum cosine similarity for a vector-only hit.
    #[serde(default = "default_min_vector_similarity")]
    pub min_vector_similarity: f64,
}

impl Default for MemorySearchConfig {
    fn default() -> Self {
        Self {
            hybrid_weight_fts: default_hybrid_weight_fts(),
            hybrid_weight_vector: default_hybrid_weight_vector(),
            min_vector_similarity: default_min_vector_similarity(),
        }
    }
}

fn default_hybrid_weight_fts() -> f64 {
    0.7
}

fn default_hybrid_weight_vector() -> f64 {
    0.3
}

fn default_min_vector_similarity() -> f64 {
    0.30
}

impl MemorySearchConfig {
    /// Validates fusion weights; fails `atlas doctor` clearly on invalid config.
    pub fn validate(&self) -> Result<()> {
        if !self.hybrid_weight_fts.is_finite() || !(0.0..=1.0).contains(&self.hybrid_weight_fts) {
            anyhow::bail!(
                "invalid config: memory.search.hybrid_weight_fts must be between 0.0 and 1.0"
            );
        }
        if !self.hybrid_weight_vector.is_finite()
            || !(0.0..=1.0).contains(&self.hybrid_weight_vector)
        {
            anyhow::bail!(
                "invalid config: memory.search.hybrid_weight_vector must be between 0.0 and 1.0"
            );
        }
        if self.hybrid_weight_fts + self.hybrid_weight_vector <= 0.0 {
            anyhow::bail!(
                "invalid config: memory.search.hybrid_weight_fts + memory.search.hybrid_weight_vector must be greater than 0.0"
            );
        }
        if !self.min_vector_similarity.is_finite()
            || !(0.0..=1.0).contains(&self.min_vector_similarity)
        {
            anyhow::bail!(
                "invalid config: memory.search.min_vector_similarity must be between 0.0 and 1.0"
            );
        }
        Ok(())
    }
}

/// `[memory.recall]` two-path recall scoring and quota policy (VM3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallConfig {
    /// Memories updated within this many days keep `time_score = 1.0`.
    #[serde(default = "default_recall_freshness_window_days")]
    pub freshness_window_days: u32,
    /// Composite weight for the freshness signal (`0..=1`).
    #[serde(default = "default_recall_weight_time")]
    pub weight_time: f64,
    /// Composite weight for the business-importance signal (`0..=1`).
    #[serde(default = "default_recall_weight_business")]
    pub weight_business: f64,
    /// Fraction of `limit` reserved for the primary lexical path.
    #[serde(default = "default_recall_primary_quota_ratio")]
    pub primary_quota_ratio: f64,
    /// Maximum memoir relation hops for the auxiliary path (`1..=2`).
    #[serde(default = "default_recall_aux_max_hops")]
    pub aux_max_hops: u32,
}

impl Default for MemoryRecallConfig {
    fn default() -> Self {
        Self {
            freshness_window_days: default_recall_freshness_window_days(),
            weight_time: default_recall_weight_time(),
            weight_business: default_recall_weight_business(),
            primary_quota_ratio: default_recall_primary_quota_ratio(),
            aux_max_hops: default_recall_aux_max_hops(),
        }
    }
}

fn default_recall_freshness_window_days() -> u32 {
    1
}

fn default_recall_weight_time() -> f64 {
    0.25
}

fn default_recall_weight_business() -> f64 {
    0.15
}

fn default_recall_primary_quota_ratio() -> f64 {
    0.75
}

fn default_recall_aux_max_hops() -> u32 {
    2
}

impl MemoryRecallConfig {
    /// Validates weights, quota, and hop bounds; fails `atlas doctor` clearly
    /// through `Config::load` on invalid config.
    pub fn validate(&self) -> Result<()> {
        if !self.weight_time.is_finite() || !(0.0..=1.0).contains(&self.weight_time) {
            anyhow::bail!("invalid config: memory.recall.weight_time must be between 0.0 and 1.0");
        }
        if !self.weight_business.is_finite() || !(0.0..=1.0).contains(&self.weight_business) {
            anyhow::bail!(
                "invalid config: memory.recall.weight_business must be between 0.0 and 1.0"
            );
        }
        if self.weight_time + self.weight_business >= 1.0 {
            anyhow::bail!(
                "invalid config: memory.recall.weight_time + memory.recall.weight_business must be less than 1.0"
            );
        }
        if !self.primary_quota_ratio.is_finite() || !(0.0..=1.0).contains(&self.primary_quota_ratio)
        {
            anyhow::bail!(
                "invalid config: memory.recall.primary_quota_ratio must be between 0.0 and 1.0"
            );
        }
        if !(1..=2).contains(&self.aux_max_hops) {
            anyhow::bail!("invalid config: memory.recall.aux_max_hops must be 1 or 2");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingBackendConfig {
    pub url: String,
    pub model: String,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
}

fn validate_usize_limit(name: &str, value: usize, max: usize) -> Result<usize> {
    if value == 0 {
        anyhow::bail!("invalid config: {name} must be greater than 0");
    }
    if value > max {
        anyhow::bail!("invalid config: {name}={value} exceeds safe maximum {max}");
    }
    Ok(value)
}

fn validate_u64_limit(name: &str, value: u64, max: usize) -> Result<u64> {
    if value == 0 {
        anyhow::bail!("invalid config: {name} must be greater than 0");
    }
    if value > max as u64 {
        anyhow::bail!("invalid config: {name}={value} exceeds safe maximum {max}");
    }
    Ok(value)
}

fn validate_positive_u64(name: &str, value: u64) -> Result<u64> {
    if value == 0 {
        anyhow::bail!("invalid config: {name} must be greater than 0");
    }
    Ok(value)
}

fn validate_positive_u32(name: &str, value: u32) -> Result<u32> {
    if value == 0 {
        anyhow::bail!("invalid config: {name} must be greater than 0");
    }
    Ok(value)
}

fn validate_positive_f64(name: &str, value: f64) -> Result<f64> {
    if !value.is_finite() || value <= 0.0 {
        anyhow::bail!("invalid config: {name} must be a finite value greater than 0");
    }
    Ok(value)
}

fn validate_f64_range(name: &str, value: f64, min: f64, max: f64) -> Result<f64> {
    if !value.is_finite() || value < min || value > max {
        anyhow::bail!("invalid config: {name}={value} must be within [{min}, {max}]");
    }
    Ok(value)
}

fn validate_ordered_score_thresholds(name: &str, low: f64, medium: f64, high: f64) -> Result<()> {
    validate_f64_range(&format!("{name}.low"), low, 0.0, 1.0)?;
    validate_f64_range(&format!("{name}.medium"), medium, 0.0, 1.0)?;
    validate_f64_range(&format!("{name}.high"), high, 0.0, 1.0)?;
    if !(low < medium && medium < high) {
        anyhow::bail!(
            "invalid config: {name} must satisfy low ({low}) < medium ({medium}) < high ({high})"
        );
    }
    Ok(())
}

fn validate_nonempty_string(name: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("invalid config: {name} must not be empty");
    }
    Ok(trimmed.to_owned())
}
