use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use atlas_core::{BudgetLimitRule, BudgetPolicy};
use atlas_repo::DEFAULT_MAX_FILE_BYTES;
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
#[serde(default)]
pub struct McpConfig {
    /// Number of MCP worker threads (clamped to 1–64).
    pub worker_threads: usize,
    /// Hard timeout in milliseconds for each MCP tool request (clamped to 1_000–3_600_000).
    pub tool_timeout_ms: u64,
    /// Maximum serialized MCP tool response size in bytes.
    pub max_mcp_response_bytes: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            worker_threads: DEFAULT_MCP_WORKER_THREADS,
            tool_timeout_ms: DEFAULT_MCP_TOOL_TIMEOUT_MS,
            max_mcp_response_bytes: BudgetPolicy::default()
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .default_limit as u64,
        }
    }
}

/// Search-phase configuration.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
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
    /// Maximum seed candidates accepted before graph expansion or semantic rerank.
    pub max_query_candidates: usize,
    /// Maximum wall time for one query path before Atlas reports a budget hit.
    pub max_query_wall_time_ms: u64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            hybrid_enabled: false,
            top_k_fts: 60,
            top_k_vector: 60,
            rrf_k: 60,
            max_query_candidates: BudgetPolicy::default()
                .query_candidates_and_seeds
                .candidates
                .default_limit,
            max_query_wall_time_ms: BudgetPolicy::default()
                .query_candidates_and_seeds
                .wall_time_ms
                .default_limit as u64,
        }
    }
}

/// Build-phase configuration.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildConfig {
    /// Number of files parsed in parallel per batch (clamped to 1–4096).
    pub parse_batch_size: usize,
    /// Maximum accepted files in one build/update run.
    pub max_files_per_run: usize,
    /// Maximum accepted total bytes in one build/update run.
    pub max_total_bytes_per_run: u64,
    /// Maximum accepted bytes for a single file.
    pub max_file_bytes: u64,
    /// Maximum parse failures tolerated before the run becomes build_failed.
    pub max_parse_failures: usize,
    /// Maximum tolerated parse failure ratio in the range [0.0, 1.0].
    pub max_parse_failure_ratio: f64,
    /// Maximum wall-clock time in milliseconds before the run becomes degraded.
    pub max_wall_time_ms: u64,
}

impl Default for BuildConfig {
    fn default() -> Self {
        let policy = BudgetPolicy::default();
        Self {
            parse_batch_size: DEFAULT_PARSE_BATCH_SIZE,
            max_files_per_run: policy.build_update.files_per_run.default_limit,
            max_total_bytes_per_run: policy.build_update.total_bytes_per_run.default_limit as u64,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_parse_failures: policy.build_update.parse_failures.default_limit,
            max_parse_failure_ratio: policy.build_update.parse_failure_ratio_bps.default_limit
                as f64
                / 10_000.0,
            max_wall_time_ms: policy.build_update.wall_time_ms.default_limit as u64,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BuildRunBudget {
    pub max_files_per_run: usize,
    pub max_total_bytes_per_run: u64,
    pub max_file_bytes: u64,
    pub max_parse_failures: usize,
    pub max_parse_failure_ratio_bps: usize,
    pub max_wall_time_ms: u64,
}

impl Default for BuildRunBudget {
    fn default() -> Self {
        let policy = BudgetPolicy::default();
        Self {
            max_files_per_run: policy.build_update.files_per_run.default_limit,
            max_total_bytes_per_run: policy.build_update.total_bytes_per_run.default_limit as u64,
            max_file_bytes: policy.build_update.file_bytes.default_limit as u64,
            max_parse_failures: policy.build_update.parse_failures.default_limit,
            max_parse_failure_ratio_bps: policy.build_update.parse_failure_ratio_bps.default_limit,
            max_wall_time_ms: policy.build_update.wall_time_ms.default_limit as u64,
        }
    }
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

impl BuildConfig {
    pub fn run_budget(&self) -> Result<BuildRunBudget> {
        let policy = BudgetPolicy::default();
        if !(0.0..=1.0).contains(&self.max_parse_failure_ratio) {
            anyhow::bail!(
                "invalid config: build.max_parse_failure_ratio={} must be within [0.0, 1.0]",
                self.max_parse_failure_ratio
            );
        }

        if self.max_parse_failures > policy.build_update.parse_failures.max_limit {
            anyhow::bail!(
                "invalid config: build.max_parse_failures={} exceeds safe maximum {}",
                self.max_parse_failures,
                policy.build_update.parse_failures.max_limit
            );
        }

        let ratio_bps = (self.max_parse_failure_ratio * 10_000.0).round() as usize;
        if ratio_bps > policy.build_update.parse_failure_ratio_bps.max_limit {
            anyhow::bail!(
                "invalid config: build.max_parse_failure_ratio={} exceeds safe maximum {}",
                self.max_parse_failure_ratio,
                policy.build_update.parse_failure_ratio_bps.max_limit as f64 / 10_000.0
            );
        }

        Ok(BuildRunBudget {
            max_files_per_run: validate_usize_limit(
                "build.max_files_per_run",
                self.max_files_per_run,
                policy.build_update.files_per_run.max_limit,
            )?,
            max_total_bytes_per_run: validate_u64_limit(
                "build.max_total_bytes_per_run",
                self.max_total_bytes_per_run,
                policy.build_update.total_bytes_per_run.max_limit,
            )?,
            max_file_bytes: validate_u64_limit(
                "build.max_file_bytes",
                self.max_file_bytes,
                policy.build_update.file_bytes.max_limit,
            )?,
            max_parse_failures: self.max_parse_failures,
            max_parse_failure_ratio_bps: ratio_bps,
            max_wall_time_ms: validate_u64_limit(
                "build.max_wall_time_ms",
                self.max_wall_time_ms,
                policy.build_update.wall_time_ms.max_limit,
            )?,
        })
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

    pub fn build_run_budget(&self) -> Result<BuildRunBudget> {
        self.build.run_budget()
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

/// Context-engine configuration (symbol/file/review context bounds).
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextConfig {
    /// Default maximum nodes returned by the context engine (default: 100).
    pub max_context_nodes: usize,
    /// Default maximum traversal depth for context queries (default: 2).
    pub max_context_depth: u32,
    /// Maximum accepted changed-symbol or query seed nodes before expansion.
    pub max_seed_nodes: usize,
    /// Maximum accepted changed-file seeds before impact/review context assembly.
    pub max_seed_files: usize,
    /// Maximum traversal depth for graph-backed context/impact work.
    pub max_traversal_depth: u32,
    /// Maximum traversal nodes for graph-backed context/impact work.
    pub max_traversal_nodes: usize,
    /// Maximum traversal edges for graph-backed context/impact work.
    pub max_traversal_edges: usize,
    /// Maximum serialized bytes retained for file/review-source sections.
    pub max_review_source_bytes: usize,
    /// Maximum serialized bytes retained for one context payload before CLI/MCP rendering.
    pub max_context_payload_bytes: usize,
    /// Maximum estimated tokens retained for one context payload before rendering.
    pub max_context_tokens_estimate: usize,
    /// Maximum serialized bytes retained for file excerpt/code-span metadata.
    pub max_file_excerpt_bytes: usize,
    /// Maximum serialized bytes retained for saved-context sources.
    pub max_saved_context_bytes: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_context_nodes: 100,
            max_context_depth: 2,
            max_seed_nodes: BudgetPolicy::default()
                .graph_traversal
                .seed_nodes
                .default_limit,
            max_seed_files: BudgetPolicy::default()
                .graph_traversal
                .seed_files
                .default_limit,
            max_traversal_depth: BudgetPolicy::default().graph_traversal.depth.default_limit as u32,
            max_traversal_nodes: BudgetPolicy::default().graph_traversal.nodes.default_limit,
            max_traversal_edges: BudgetPolicy::default().graph_traversal.edges.default_limit,
            max_review_source_bytes: BudgetPolicy::default()
                .mcp_cli_payload_serialization
                .review_source_bytes
                .default_limit,
            max_context_payload_bytes: BudgetPolicy::default()
                .mcp_cli_payload_serialization
                .context_payload_bytes
                .default_limit,
            max_context_tokens_estimate: BudgetPolicy::default()
                .mcp_cli_payload_serialization
                .context_tokens_estimate
                .default_limit,
            max_file_excerpt_bytes: BudgetPolicy::default()
                .mcp_cli_payload_serialization
                .file_excerpt_bytes
                .default_limit,
            max_saved_context_bytes: BudgetPolicy::default()
                .mcp_cli_payload_serialization
                .saved_context_bytes
                .default_limit,
        }
    }
}

impl Config {
    pub fn budget_policy(&self) -> Result<BudgetPolicy> {
        let mut policy = BudgetPolicy::default();

        policy.query_candidates_and_seeds.candidates = BudgetLimitRule::new(
            validate_usize_limit(
                "search.max_query_candidates",
                self.search.max_query_candidates,
                policy.query_candidates_and_seeds.candidates.max_limit,
            )?,
            policy.query_candidates_and_seeds.candidates.max_limit,
            policy.query_candidates_and_seeds.candidates.hit_behavior,
            policy
                .query_candidates_and_seeds
                .candidates
                .safe_to_answer_on_hit,
        );
        policy.query_candidates_and_seeds.wall_time_ms = BudgetLimitRule::new(
            validate_u64_limit(
                "search.max_query_wall_time_ms",
                self.search.max_query_wall_time_ms,
                policy.query_candidates_and_seeds.wall_time_ms.max_limit,
            )? as usize,
            policy.query_candidates_and_seeds.wall_time_ms.max_limit,
            policy.query_candidates_and_seeds.wall_time_ms.hit_behavior,
            policy
                .query_candidates_and_seeds
                .wall_time_ms
                .safe_to_answer_on_hit,
        );
        policy.graph_traversal.seed_nodes = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_seed_nodes",
                self.context.max_seed_nodes,
                policy.graph_traversal.seed_nodes.max_limit,
            )?,
            policy.graph_traversal.seed_nodes.max_limit,
            policy.graph_traversal.seed_nodes.hit_behavior,
            policy.graph_traversal.seed_nodes.safe_to_answer_on_hit,
        );
        policy.graph_traversal.seed_files = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_seed_files",
                self.context.max_seed_files,
                policy.graph_traversal.seed_files.max_limit,
            )?,
            policy.graph_traversal.seed_files.max_limit,
            policy.graph_traversal.seed_files.hit_behavior,
            policy.graph_traversal.seed_files.safe_to_answer_on_hit,
        );
        policy.graph_traversal.depth = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_traversal_depth",
                self.context.max_traversal_depth as usize,
                policy.graph_traversal.depth.max_limit,
            )?,
            policy.graph_traversal.depth.max_limit,
            policy.graph_traversal.depth.hit_behavior,
            policy.graph_traversal.depth.safe_to_answer_on_hit,
        );
        policy.graph_traversal.nodes = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_traversal_nodes",
                self.context.max_traversal_nodes,
                policy.graph_traversal.nodes.max_limit,
            )?,
            policy.graph_traversal.nodes.max_limit,
            policy.graph_traversal.nodes.hit_behavior,
            policy.graph_traversal.nodes.safe_to_answer_on_hit,
        );
        policy.graph_traversal.edges = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_traversal_edges",
                self.context.max_traversal_edges,
                policy.graph_traversal.edges.max_limit,
            )?,
            policy.graph_traversal.edges.max_limit,
            policy.graph_traversal.edges.hit_behavior,
            policy.graph_traversal.edges.safe_to_answer_on_hit,
        );
        policy.mcp_cli_payload_serialization.review_source_bytes = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_review_source_bytes",
                self.context.max_review_source_bytes,
                policy
                    .mcp_cli_payload_serialization
                    .review_source_bytes
                    .max_limit,
            )?,
            policy
                .mcp_cli_payload_serialization
                .review_source_bytes
                .max_limit,
            policy
                .mcp_cli_payload_serialization
                .review_source_bytes
                .hit_behavior,
            policy
                .mcp_cli_payload_serialization
                .review_source_bytes
                .safe_to_answer_on_hit,
        );
        policy.mcp_cli_payload_serialization.context_payload_bytes = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_context_payload_bytes",
                self.context.max_context_payload_bytes,
                policy
                    .mcp_cli_payload_serialization
                    .context_payload_bytes
                    .max_limit,
            )?,
            policy
                .mcp_cli_payload_serialization
                .context_payload_bytes
                .max_limit,
            policy
                .mcp_cli_payload_serialization
                .context_payload_bytes
                .hit_behavior,
            policy
                .mcp_cli_payload_serialization
                .context_payload_bytes
                .safe_to_answer_on_hit,
        );
        policy.mcp_cli_payload_serialization.context_tokens_estimate = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_context_tokens_estimate",
                self.context.max_context_tokens_estimate,
                policy
                    .mcp_cli_payload_serialization
                    .context_tokens_estimate
                    .max_limit,
            )?,
            policy
                .mcp_cli_payload_serialization
                .context_tokens_estimate
                .max_limit,
            policy
                .mcp_cli_payload_serialization
                .context_tokens_estimate
                .hit_behavior,
            policy
                .mcp_cli_payload_serialization
                .context_tokens_estimate
                .safe_to_answer_on_hit,
        );
        policy.mcp_cli_payload_serialization.file_excerpt_bytes = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_file_excerpt_bytes",
                self.context.max_file_excerpt_bytes,
                policy
                    .mcp_cli_payload_serialization
                    .file_excerpt_bytes
                    .max_limit,
            )?,
            policy
                .mcp_cli_payload_serialization
                .file_excerpt_bytes
                .max_limit,
            policy
                .mcp_cli_payload_serialization
                .file_excerpt_bytes
                .hit_behavior,
            policy
                .mcp_cli_payload_serialization
                .file_excerpt_bytes
                .safe_to_answer_on_hit,
        );
        policy.mcp_cli_payload_serialization.saved_context_bytes = BudgetLimitRule::new(
            validate_usize_limit(
                "context.max_saved_context_bytes",
                self.context.max_saved_context_bytes,
                policy
                    .mcp_cli_payload_serialization
                    .saved_context_bytes
                    .max_limit,
            )?,
            policy
                .mcp_cli_payload_serialization
                .saved_context_bytes
                .max_limit,
            policy
                .mcp_cli_payload_serialization
                .saved_context_bytes
                .hit_behavior,
            policy
                .mcp_cli_payload_serialization
                .saved_context_bytes
                .safe_to_answer_on_hit,
        );
        policy.mcp_cli_payload_serialization.mcp_response_bytes = BudgetLimitRule::new(
            validate_u64_limit(
                "mcp.max_mcp_response_bytes",
                self.mcp.max_mcp_response_bytes,
                policy
                    .mcp_cli_payload_serialization
                    .mcp_response_bytes
                    .max_limit,
            )? as usize,
            policy
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .max_limit,
            policy
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .hit_behavior,
            policy
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .safe_to_answer_on_hit,
        );

        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn mcp_config_defaults_match_expected_values() {
        let config = Config::default();
        assert_eq!(config.mcp_worker_threads(), DEFAULT_MCP_WORKER_THREADS);
        assert_eq!(config.mcp_tool_timeout_ms(), DEFAULT_MCP_TOOL_TIMEOUT_MS);
        assert_eq!(
            config.mcp.max_mcp_response_bytes,
            BudgetPolicy::default()
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .default_limit as u64
        );
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

    #[test]
    fn budget_policy_maps_payload_budget_fields() {
        let mut config = Config::default();
        config.context.max_review_source_bytes = 2048;
        config.context.max_context_payload_bytes = 4096;
        config.context.max_context_tokens_estimate = 512;
        config.context.max_file_excerpt_bytes = 256;
        config.context.max_saved_context_bytes = 128;
        config.mcp.max_mcp_response_bytes = 8192;

        let policy = config.budget_policy().expect("budget policy");

        assert_eq!(
            policy
                .mcp_cli_payload_serialization
                .review_source_bytes
                .default_limit,
            2048
        );
        assert_eq!(
            policy
                .mcp_cli_payload_serialization
                .context_payload_bytes
                .default_limit,
            4096
        );
        assert_eq!(
            policy
                .mcp_cli_payload_serialization
                .context_tokens_estimate
                .default_limit,
            512
        );
        assert_eq!(
            policy
                .mcp_cli_payload_serialization
                .file_excerpt_bytes
                .default_limit,
            256
        );
        assert_eq!(
            policy
                .mcp_cli_payload_serialization
                .saved_context_bytes
                .default_limit,
            128
        );
        assert_eq!(
            policy
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .default_limit,
            8192
        );
    }

    #[test]
    fn load_accepts_partial_nested_sections() {
        let dir = tempdir().expect("tempdir");
        let atlas_dir = dir.path();
        fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            "[mcp]\nmax_mcp_response_bytes = 4096\n\n[context]\nmax_saved_context_bytes = 256\n",
        )
        .expect("write config");

        let config = Config::load(atlas_dir).expect("load config");

        assert_eq!(config.mcp.max_mcp_response_bytes, 4096);
        assert_eq!(config.context.max_saved_context_bytes, 256);
        assert_eq!(config.mcp.worker_threads, DEFAULT_MCP_WORKER_THREADS);
    }
}
