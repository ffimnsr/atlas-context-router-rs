//! Analysis-phase configuration (dead-code, refactor safety, impact traversal).

use serde::{Deserialize, Serialize};

/// Analysis-phase configuration (dead-code, refactor safety, impact traversal).
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisConfig {
    /// Minimum certainty tier for dead-code candidates to surface.
    /// Accepted values: `"high"`, `"medium"`, `"low"` (default: `"low"`).
    pub dead_code_certainty_threshold: String,
    /// Minimum safety score [0.0, 1.0] required before auto-applying a refactor.
    /// Dry-run always works regardless of this value.
    pub refactor_safety_threshold: f64,
    /// Maximum BFS depth for impact analysis (default: 5).
    pub impact_max_depth: u32,
    /// Maximum nodes returned by impact analysis (default: 200).
    pub impact_max_nodes: usize,
    /// Qualified names treated as live even when no inbound edges are found.
    /// Useful for framework entry points not captured by the parser.
    pub dynamic_usage_allowlist: Vec<String>,
    /// Simple function/symbol names never auto-removed regardless of usage.
    /// Extends the built-in entrypoint list (`main`, `new`, `init`, …).
    pub entrypoint_allowlist: Vec<String>,
    /// Optional path to a TOML file mapping framework names to convention rules.
    /// Relative paths are resolved from the repo root.
    pub framework_conventions_file: Option<String>,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            dead_code_certainty_threshold: "low".to_owned(),
            refactor_safety_threshold: 0.5,
            impact_max_depth: 5,
            impact_max_nodes: 200,
            dynamic_usage_allowlist: Vec::new(),
            entrypoint_allowlist: Vec::new(),
            framework_conventions_file: None,
        }
    }
}
