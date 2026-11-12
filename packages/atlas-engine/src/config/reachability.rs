//! `[reachability]` configuration for the Rust module reachability guard.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Default wall-clock budget for building reachability indexes.
pub const DEFAULT_REACHABILITY_BUDGET_MS: u64 = 250;

/// Default maximum package owners indexed per reconcile run.
pub const DEFAULT_REACHABILITY_MAX_OWNERS: usize = 64;

/// Reachability guard configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReachabilityConfig {
    /// Enable the Rust module reachability guard for same-package fallbacks.
    #[serde(default = "default_reachability_enabled")]
    pub enabled: bool,
    /// Wall-clock budget for building indexes in one reconcile run.
    #[serde(default = "default_reachability_budget_ms")]
    pub budget_ms: u64,
    /// Maximum package owners indexed in one reconcile run.
    #[serde(default = "default_reachability_max_owners")]
    pub max_owners_per_run: usize,
}

fn default_reachability_enabled() -> bool {
    true
}

fn default_reachability_budget_ms() -> u64 {
    DEFAULT_REACHABILITY_BUDGET_MS
}

fn default_reachability_max_owners() -> usize {
    DEFAULT_REACHABILITY_MAX_OWNERS
}

impl Default for ReachabilityConfig {
    fn default() -> Self {
        Self {
            enabled: default_reachability_enabled(),
            budget_ms: default_reachability_budget_ms(),
            max_owners_per_run: default_reachability_max_owners(),
        }
    }
}

impl ReachabilityConfig {
    /// Validate guard bounds with config-key-named errors.
    pub fn validate(&self) -> Result<()> {
        if self.budget_ms < 1 {
            anyhow::bail!("invalid config: reachability.budget_ms must be greater than 0");
        }
        if self.max_owners_per_run < 1 {
            anyhow::bail!("invalid config: reachability.max_owners_per_run must be greater than 0");
        }
        Ok(())
    }
}
