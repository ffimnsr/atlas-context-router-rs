//! Atlas configuration: `.atlas/config.toml` schema, defaults, validation,
//! template rendering, and budget-policy mapping.
//!
//! Module root: `Config` struct, shared validation helpers, and per-family
//! submodules (`build`, `search`, `mcp`, `analysis`, `insights`,
//! `sanitization`, `context`, `load`, `template`, `accessors`, `budget`).

mod accessors;
mod analysis;
mod budget;
mod build;
mod context;
mod insights;
mod load;
mod mcp;
mod parsers;
mod sanitization;
mod search;
mod template;

#[cfg(test)]
mod tests;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use analysis::{AnalysisConfig, FeedbackAdjustmentConfig};
pub use build::{BuildConfig, BuildRunBudget, DEFAULT_PARSE_BATCH_SIZE};
pub use context::{
    ContextConfig, ContextTokenizerConfig, TokenizerFallbackMode, TokenizerProvider,
};
pub use insights::{InsightsConfig, InsightsLayerRule};
pub use load::TokenCounterLoadResult;
pub use mcp::{
    DEFAULT_MCP_TOOL_TIMEOUT_MS, DEFAULT_MCP_WORKER_THREADS, McpConfig, McpHttpAuthConfig,
    ValidatedMcpHttpAuthConfig,
};
pub use parsers::ParsersConfig;
pub use sanitization::SanitizationConfig;
pub use search::{
    DEFAULT_EMBED_MAX_RETRIES, DEFAULT_EMBED_MODEL, DEFAULT_EMBED_RETRY_BACKOFF_MS,
    DEFAULT_EMBED_TIMEOUT_SECS, SearchConfig, SearchEmbeddingConfig,
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
    pub memory: MemoryConfig,
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
        self.wake_up.validate()
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
