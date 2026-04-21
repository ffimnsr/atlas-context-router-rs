use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default parse-worker batch size.  Can be overridden in `.atlas/config.toml`.
pub const DEFAULT_PARSE_BATCH_SIZE: usize = 64;
pub const DEFAULT_MCP_WORKER_THREADS: usize = 2;
pub const DEFAULT_MCP_TOOL_TIMEOUT_MS: u64 = 300_000;

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
    pub context: ContextConfig,
    #[serde(default)]
    pub mcp: McpConfig,
}

/// MCP transport configuration.
#[derive(Debug, Serialize, Deserialize)]
pub struct McpConfig {
    /// Number of MCP worker threads (clamped to 1–64).
    pub worker_threads: usize,
    /// Hard timeout in milliseconds for each MCP tool request (clamped to 1_000–3_600_000).
    pub tool_timeout_ms: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            worker_threads: DEFAULT_MCP_WORKER_THREADS,
            tool_timeout_ms: DEFAULT_MCP_TOOL_TIMEOUT_MS,
        }
    }
}

/// Search-phase configuration.
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Enable hybrid (FTS + vector) retrieval when an embedding backend is configured.
    /// Falls back to FTS-only when no backend is available regardless of this flag.
    pub hybrid_enabled: bool,
    /// FTS candidate pool size fetched before Reciprocal Rank Fusion merge.
    pub top_k_fts: usize,
    /// Vector candidate pool size fetched before Reciprocal Rank Fusion merge.
    pub top_k_vector: usize,
    /// RRF k constant (higher = less rank-position sensitivity, default 60).
    pub rrf_k: u32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            hybrid_enabled: false,
            top_k_fts: 60,
            top_k_vector: 60,
            rrf_k: 60,
        }
    }
}

/// Build-phase configuration.
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Number of files parsed in parallel per batch (clamped to 1–4096).
    pub parse_batch_size: usize,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            parse_batch_size: DEFAULT_PARSE_BATCH_SIZE,
        }
    }
}

impl Config {
    /// Load config from `<atlas_dir>/config.toml`.
    ///
    /// Returns a default `Config` if the file does not exist.
    pub fn load(atlas_dir: &Path) -> Result<Self> {
        let path = atlas_dir.join(crate::paths::ATLAS_CONFIG);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("cannot parse {}", path.display()))
    }

    /// Write the default config to `<atlas_dir>/config.toml`.
    ///
    /// Does not overwrite an existing file.
    pub fn write_default(atlas_dir: &Path) -> Result<bool> {
        let path = atlas_dir.join(crate::paths::ATLAS_CONFIG);
        if path.exists() {
            return Ok(false);
        }
        let default = Self::default();
        let content =
            toml::to_string_pretty(&default).context("cannot serialize default config")?;
        fs::write(&path, content).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(true)
    }

    /// Return the effective parse batch size, clamped to [1, 4096].
    pub fn parse_batch_size(&self) -> usize {
        self.build.parse_batch_size.clamp(1, 4096)
    }

    /// Return effective MCP worker thread count, clamped to [1, 64].
    pub fn mcp_worker_threads(&self) -> usize {
        self.mcp.worker_threads.clamp(1, 64)
    }

    /// Return effective MCP tool timeout in milliseconds, clamped to [1_000, 3_600_000].
    pub fn mcp_tool_timeout_ms(&self) -> u64 {
        self.mcp.tool_timeout_ms.clamp(1_000, 3_600_000)
    }
}

/// Analysis-phase configuration (dead-code, refactor safety, impact traversal).
#[derive(Debug, Serialize, Deserialize)]
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

/// Context-engine configuration (symbol/file/review context bounds).
#[derive(Debug, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Default maximum nodes returned by the context engine (default: 100).
    pub max_context_nodes: usize,
    /// Default maximum traversal depth for context queries (default: 2).
    pub max_context_depth: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_context_nodes: 100,
            max_context_depth: 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_config_defaults_match_expected_values() {
        let config = Config::default();
        assert_eq!(config.mcp_worker_threads(), DEFAULT_MCP_WORKER_THREADS);
        assert_eq!(config.mcp_tool_timeout_ms(), DEFAULT_MCP_TOOL_TIMEOUT_MS);
    }

    #[test]
    fn mcp_config_values_are_clamped() {
        let mut config = Config::default();
        config.mcp.worker_threads = 0;
        config.mcp.tool_timeout_ms = 10;
        assert_eq!(config.mcp_worker_threads(), 1);
        assert_eq!(config.mcp_tool_timeout_ms(), 1_000);

        config.mcp.worker_threads = 999;
        config.mcp.tool_timeout_ms = 9_999_999;
        assert_eq!(config.mcp_worker_threads(), 64);
        assert_eq!(config.mcp_tool_timeout_ms(), 3_600_000);
    }
}
