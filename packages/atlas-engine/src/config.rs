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
mod sanitization;
mod search;
mod template;

#[cfg(test)]
mod tests;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use analysis::AnalysisConfig;
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
    pub memory: MemoryConfig,
}

/// Memory surface configuration (ICM-A).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Allow arbitrary frontend identities beyond the known set
    /// (`claude`, `codex`, `copilot`, `cli`, `mcp`) for memory writes and
    /// visibility. Defaults to false: unknown frontends are rejected.
    #[serde(default)]
    pub allow_custom_frontends: bool,
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
