use std::collections::HashMap;
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
pub const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
pub const DEFAULT_EMBED_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_EMBED_MAX_RETRIES: u32 = 3;
pub const DEFAULT_EMBED_RETRY_BACKOFF_MS: u64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigTemplateProfile {
    Minimal,
    Standard,
    Full,
}

impl ConfigTemplateProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Standard => "standard",
            Self::Full => "full",
        }
    }
}

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingBackendConfig {
    pub url: String,
    pub model: String,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
}

/// MCP transport configuration.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Number of MCP worker threads (clamped to 1–64).
    pub worker_threads: usize,
    /// Default timeout in milliseconds for MCP tool requests without a per-tool override.
    pub tool_timeout_ms: u64,
    /// Optional per-tool timeout overrides in milliseconds.
    pub tool_timeout_ms_by_tool: HashMap<String, u64>,
    /// Maximum serialized MCP tool response size in bytes.
    pub max_mcp_response_bytes: u64,
    /// Optional Streamable HTTP protected-resource auth config.
    pub http_auth: McpHttpAuthConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpHttpAuthConfig {
    /// Enable protected-resource OAuth bearer validation for HTTP transport.
    pub enabled: bool,
    /// Authorization server issuer URL.
    pub issuer: Option<String>,
    /// Optional explicit OIDC discovery URL.
    pub discovery_url: Option<String>,
    /// Optional explicit JWKS URL.
    pub jwks_url: Option<String>,
    /// Resource identifier / audience expected by Atlas HTTP transport.
    pub resource: Option<String>,
    /// Required scopes per route family.
    pub required_scopes: HashMap<String, Vec<String>>,
    /// Optional browser origins allowed to call HTTP transport.
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedMcpHttpAuthConfig {
    pub issuer: String,
    pub discovery_url: Option<String>,
    pub jwks_url: Option<String>,
    pub resource: String,
    pub required_scopes: HashMap<String, Vec<String>>,
    pub allowed_origins: Vec<String>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            worker_threads: DEFAULT_MCP_WORKER_THREADS,
            tool_timeout_ms: DEFAULT_MCP_TOOL_TIMEOUT_MS,
            tool_timeout_ms_by_tool: HashMap::new(),
            max_mcp_response_bytes: BudgetPolicy::default()
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .default_limit as u64,
            http_auth: McpHttpAuthConfig::default(),
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
    /// HTTP embedding backend configuration used for hybrid retrieval.
    pub embedding: SearchEmbeddingConfig,
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
            embedding: SearchEmbeddingConfig::default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchEmbeddingConfig {
    /// Base URL for embedding requests. When unset, hybrid retrieval falls back to FTS.
    pub url: Option<String>,
    /// Embedding model name sent to the backend.
    pub model: String,
    /// Per-request timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum retry attempts on transient backend failures.
    pub max_retries: u32,
    /// Initial retry backoff in milliseconds.
    pub retry_backoff_ms: u64,
}

impl Default for SearchEmbeddingConfig {
    fn default() -> Self {
        Self {
            url: None,
            model: DEFAULT_EMBED_MODEL.to_owned(),
            timeout_secs: DEFAULT_EMBED_TIMEOUT_SECS,
            max_retries: DEFAULT_EMBED_MAX_RETRIES,
            retry_backoff_ms: DEFAULT_EMBED_RETRY_BACKOFF_MS,
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

fn validate_nonempty_string(name: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("invalid config: {name} must not be empty");
    }
    Ok(trimmed.to_owned())
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
        let config: Self =
            toml::from_str(&raw).with_context(|| format!("cannot parse {}", path.display()))?;
        config.insights.validate()?;
        config.sanitization.validate(atlas_dir)?;
        Ok(config)
    }

    /// Write the default config to `<atlas_dir>/config.toml`.
    ///
    /// Does not overwrite an existing file.
    pub fn write_default(atlas_dir: &Path) -> Result<bool> {
        Self::write_template(atlas_dir, ConfigTemplateProfile::Standard)
    }

    /// Write a commented config template to `<atlas_dir>/config.toml`.
    ///
    /// Does not overwrite an existing file.
    pub fn write_template(atlas_dir: &Path, profile: ConfigTemplateProfile) -> Result<bool> {
        let path = atlas_dir.join(crate::paths::ATLAS_CONFIG);
        if path.exists() {
            return Ok(false);
        }
        let content = Self::render_template(profile)?;
        fs::write(&path, content).with_context(|| format!("cannot write {}", path.display()))?;
        Ok(true)
    }

    pub fn render_template(profile: ConfigTemplateProfile) -> Result<String> {
        let active = Self::profile(profile);
        active.build_run_budget()?;
        active.budget_policy()?;
        active.insights.validate()?;

        let mut lines = vec![
            "# Atlas config template.",
            "#",
            "# Profile selected by `atlas init --profile`.",
            &format!("# profile = \"{}\"", profile.as_str()),
            "#",
            "# Lines that start with `# ` are examples. Remove leading `# ` to activate them.",
            "# All active values in this template validate against Atlas config rules.",
            "",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();

        match profile {
            ConfigTemplateProfile::Minimal => {
                lines.push(
                    "# Minimal profile: keep defaults, uncomment only overrides you need."
                        .to_owned(),
                );
                lines.push(String::new());
            }
            ConfigTemplateProfile::Standard => {
                lines.push(
                    "# Standard profile: common operational knobs shown with Atlas defaults."
                        .to_owned(),
                );
                lines.push(String::new());
            }
            ConfigTemplateProfile::Full => {
                lines.push(
                    "# Full profile: every key rendered as active config for copy-editing."
                        .to_owned(),
                );
                lines.push(String::new());
            }
        }

        lines.extend(render_section(
            "build",
            &[
                (
                    "parse_batch_size",
                    active.build.parse_batch_size.to_string(),
                ),
                (
                    "max_files_per_run",
                    active.build.max_files_per_run.to_string(),
                ),
                (
                    "max_total_bytes_per_run",
                    active.build.max_total_bytes_per_run.to_string(),
                ),
                ("max_file_bytes", active.build.max_file_bytes.to_string()),
                (
                    "max_parse_failures",
                    active.build.max_parse_failures.to_string(),
                ),
                (
                    "max_parse_failure_ratio",
                    active.build.max_parse_failure_ratio.to_string(),
                ),
                (
                    "max_wall_time_ms",
                    active.build.max_wall_time_ms.to_string(),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));

        lines.extend(render_section(
            "search",
            &[
                ("hybrid_enabled", active.search.hybrid_enabled.to_string()),
                ("top_k_fts", active.search.top_k_fts.to_string()),
                ("top_k_vector", active.search.top_k_vector.to_string()),
                ("rrf_k", active.search.rrf_k.to_string()),
                (
                    "max_query_candidates",
                    active.search.max_query_candidates.to_string(),
                ),
                (
                    "max_query_wall_time_ms",
                    active.search.max_query_wall_time_ms.to_string(),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));

        lines.extend(render_section(
            "search.embedding",
            &[
                (
                    "url",
                    render_optional_example_string(
                        active.search.embedding.url.as_deref(),
                        "http://localhost:11434",
                    ),
                ),
                ("model", format!("\"{}\"", active.search.embedding.model)),
                (
                    "timeout_secs",
                    active.search.embedding.timeout_secs.to_string(),
                ),
                (
                    "max_retries",
                    active.search.embedding.max_retries.to_string(),
                ),
                (
                    "retry_backoff_ms",
                    active.search.embedding.retry_backoff_ms.to_string(),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));

        lines.extend(render_section(
            "analysis",
            &[
                (
                    "dead_code_certainty_threshold",
                    format!("\"{}\"", active.analysis.dead_code_certainty_threshold),
                ),
                (
                    "refactor_safety_threshold",
                    active.analysis.refactor_safety_threshold.to_string(),
                ),
                (
                    "impact_max_depth",
                    active.analysis.impact_max_depth.to_string(),
                ),
                (
                    "impact_max_nodes",
                    active.analysis.impact_max_nodes.to_string(),
                ),
                (
                    "dynamic_usage_allowlist",
                    render_string_array(&active.analysis.dynamic_usage_allowlist),
                ),
                (
                    "entrypoint_allowlist",
                    render_string_array(&active.analysis.entrypoint_allowlist),
                ),
                (
                    "framework_conventions_file",
                    render_optional_string(active.analysis.framework_conventions_file.as_deref()),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));

        lines.extend(render_section(
            "insights",
            &[
                (
                    "large_function_loc",
                    active.insights.large_function_loc.to_string(),
                ),
                (
                    "repeated_call_chain_min_length",
                    active.insights.repeated_call_chain_min_length.to_string(),
                ),
                ("high_fan_in", active.insights.high_fan_in.to_string()),
                ("high_fan_out", active.insights.high_fan_out.to_string()),
                ("high_coupling", active.insights.high_coupling.to_string()),
                (
                    "deep_chain_length",
                    active.insights.deep_chain_length.to_string(),
                ),
                ("max_findings", active.insights.max_findings.to_string()),
                (
                    "high_cyclomatic_complexity",
                    active.insights.high_cyclomatic_complexity.to_string(),
                ),
                (
                    "high_cognitive_complexity",
                    active.insights.high_cognitive_complexity.to_string(),
                ),
                (
                    "max_nesting_depth",
                    active.insights.max_nesting_depth.to_string(),
                ),
                ("branch_count", active.insights.branch_count.to_string()),
                (
                    "outlier_percentile_cutoff",
                    active.insights.outlier_percentile_cutoff.to_string(),
                ),
                (
                    "risk_public_api_weight",
                    active.insights.risk_public_api_weight.to_string(),
                ),
                (
                    "risk_fan_in_weight",
                    active.insights.risk_fan_in_weight.to_string(),
                ),
                (
                    "risk_fan_out_weight",
                    active.insights.risk_fan_out_weight.to_string(),
                ),
                (
                    "risk_cross_module_dependency_weight",
                    active
                        .insights
                        .risk_cross_module_dependency_weight
                        .to_string(),
                ),
                (
                    "risk_test_adjacency_mitigation_weight",
                    active
                        .insights
                        .risk_test_adjacency_mitigation_weight
                        .to_string(),
                ),
                (
                    "risk_dependency_depth_weight",
                    active.insights.risk_dependency_depth_weight.to_string(),
                ),
                (
                    "risk_unresolved_edge_weight",
                    active.insights.risk_unresolved_edge_weight.to_string(),
                ),
                (
                    "risk_large_function_weight",
                    active.insights.risk_large_function_weight.to_string(),
                ),
                (
                    "risk_loc_weight",
                    active.insights.risk_loc_weight.to_string(),
                ),
                (
                    "risk_cyclomatic_complexity_weight",
                    active
                        .insights
                        .risk_cyclomatic_complexity_weight
                        .to_string(),
                ),
                (
                    "risk_cognitive_complexity_weight",
                    active.insights.risk_cognitive_complexity_weight.to_string(),
                ),
                (
                    "risk_nesting_depth_weight",
                    active.insights.risk_nesting_depth_weight.to_string(),
                ),
                (
                    "risk_cycle_participation_weight",
                    active.insights.risk_cycle_participation_weight.to_string(),
                ),
                (
                    "risk_medium_threshold",
                    active.insights.risk_medium_threshold.to_string(),
                ),
                (
                    "risk_high_threshold",
                    active.insights.risk_high_threshold.to_string(),
                ),
                (
                    "ignore_files",
                    render_string_array(&active.insights.ignore_files),
                ),
                (
                    "ignore_modules",
                    render_string_array(&active.insights.ignore_modules),
                ),
                (
                    "ignore_node_kinds",
                    render_string_array(&active.insights.ignore_node_kinds),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));

        lines.extend(render_insights_layer_rules(
            &active.insights.layer_rules,
            profile,
        ));

        lines.extend(render_section(
            "sanitization",
            &[(
                "redaction_rules_file",
                if profile == ConfigTemplateProfile::Full {
                    render_optional_string(active.sanitization.redaction_rules_file.as_deref())
                } else {
                    render_optional_example_string(
                        active.sanitization.redaction_rules_file.as_deref(),
                        "redaction-rules.toml",
                    )
                },
            )],
            profile == ConfigTemplateProfile::Full,
        ));

        lines.extend(render_section(
            "context",
            &[
                (
                    "max_context_nodes",
                    active.context.max_context_nodes.to_string(),
                ),
                (
                    "max_context_depth",
                    active.context.max_context_depth.to_string(),
                ),
                ("max_seed_nodes", active.context.max_seed_nodes.to_string()),
                ("max_seed_files", active.context.max_seed_files.to_string()),
                (
                    "max_traversal_depth",
                    active.context.max_traversal_depth.to_string(),
                ),
                (
                    "max_traversal_nodes",
                    active.context.max_traversal_nodes.to_string(),
                ),
                (
                    "max_traversal_edges",
                    active.context.max_traversal_edges.to_string(),
                ),
                (
                    "max_review_source_bytes",
                    active.context.max_review_source_bytes.to_string(),
                ),
                (
                    "max_context_payload_bytes",
                    active.context.max_context_payload_bytes.to_string(),
                ),
                (
                    "max_context_tokens_estimate",
                    active.context.max_context_tokens_estimate.to_string(),
                ),
                (
                    "max_file_excerpt_bytes",
                    active.context.max_file_excerpt_bytes.to_string(),
                ),
                (
                    "max_saved_context_bytes",
                    active.context.max_saved_context_bytes.to_string(),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));

        lines.extend(render_section(
            "mcp",
            &[
                ("worker_threads", active.mcp.worker_threads.to_string()),
                ("tool_timeout_ms", active.mcp.tool_timeout_ms.to_string()),
                (
                    "tool_timeout_ms_by_tool",
                    render_timeout_map(&active.mcp.tool_timeout_ms_by_tool),
                ),
                (
                    "max_mcp_response_bytes",
                    active.mcp.max_mcp_response_bytes.to_string(),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));

        lines.extend(render_section(
            "mcp.http_auth",
            &[
                ("enabled", active.mcp.http_auth.enabled.to_string()),
                (
                    "issuer",
                    render_optional_string(active.mcp.http_auth.issuer.as_deref()),
                ),
                (
                    "discovery_url",
                    render_optional_string(active.mcp.http_auth.discovery_url.as_deref()),
                ),
                (
                    "jwks_url",
                    render_optional_string(active.mcp.http_auth.jwks_url.as_deref()),
                ),
                (
                    "resource",
                    render_optional_string(active.mcp.http_auth.resource.as_deref()),
                ),
                (
                    "required_scopes",
                    render_string_array_map(&active.mcp.http_auth.required_scopes),
                ),
                (
                    "allowed_origins",
                    render_string_array(&active.mcp.http_auth.allowed_origins),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));

        Ok(lines.join("\n"))
    }

    pub fn profile(profile: ConfigTemplateProfile) -> Self {
        let mut config = Self::default();
        match profile {
            ConfigTemplateProfile::Minimal => {}
            ConfigTemplateProfile::Standard => {
                config.build.parse_batch_size = 64;
                config.search.max_query_wall_time_ms = 30_000;
                config.context.max_context_nodes = 100;
                config.mcp.tool_timeout_ms = 300_000;
            }
            ConfigTemplateProfile::Full => {
                config.search.hybrid_enabled = true;
                config.search.top_k_fts = 80;
                config.search.top_k_vector = 80;
                config.search.embedding.url = Some("http://localhost:11434".to_owned());
                config.analysis.dead_code_certainty_threshold = "medium".to_owned();
                config.analysis.refactor_safety_threshold = 0.6;
                config.insights.large_function_loc = 60;
                config.insights.repeated_call_chain_min_length = 4;
                config.insights.high_fan_in = 15;
                config.insights.high_fan_out = 12;
                config.insights.max_findings = 100;
                config.insights.outlier_percentile_cutoff = 90;
                config.insights.ignore_node_kinds = vec!["import".to_owned()];
                config.insights.layer_rules = vec![
                    InsightsLayerRule {
                        name: "api".to_owned(),
                        path_prefixes: vec!["src/api".to_owned()],
                        module_prefixes: vec![],
                    },
                    InsightsLayerRule {
                        name: "domain".to_owned(),
                        path_prefixes: vec!["src/domain".to_owned()],
                        module_prefixes: vec![],
                    },
                ];
                config.context.max_context_nodes = 150;
                config.context.max_context_depth = 3;
                config.mcp.worker_threads = 4;
                config
                    .mcp
                    .tool_timeout_ms_by_tool
                    .insert("build_or_update_graph".to_owned(), 900_000);
                config
                    .mcp
                    .tool_timeout_ms_by_tool
                    .insert("get_review_context".to_owned(), 120_000);
                config.mcp.http_auth.enabled = true;
                config.mcp.http_auth.issuer = Some("https://auth.atlas.test".to_owned());
                config.mcp.http_auth.resource = Some("https://atlas.test/mcp".to_owned());
                config.mcp.http_auth.required_scopes.insert(
                    "mcp".to_owned(),
                    vec!["atlas:mcp".to_owned(), "atlas:read".to_owned()],
                );
                config.mcp.http_auth.allowed_origins = vec!["https://app.atlas.test".to_owned()];
            }
        }
        config
    }

    /// Return the effective parse batch size, clamped to [1, 4096].
    pub fn parse_batch_size(&self) -> usize {
        self.build.parse_batch_size.clamp(1, 4096)
    }

    pub fn build_run_budget(&self) -> Result<BuildRunBudget> {
        self.build.run_budget()
    }

    pub fn insights_config(&self) -> Result<InsightsConfig> {
        self.insights.validate()?;
        Ok(self.insights.clone())
    }

    pub fn resolve_redaction_rules_file(
        &self,
        atlas_dir: &Path,
    ) -> Result<Option<std::path::PathBuf>> {
        self.sanitization.resolve_redaction_rules_file(atlas_dir)
    }

    pub fn embedding_backend(&self) -> Result<Option<EmbeddingBackendConfig>> {
        let Some(url) = self
            .search
            .embedding
            .url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };

        Ok(Some(EmbeddingBackendConfig {
            url: validate_nonempty_string("search.embedding.url", url)?,
            model: validate_nonempty_string(
                "search.embedding.model",
                &self.search.embedding.model,
            )?,
            timeout_secs: validate_positive_u64(
                "search.embedding.timeout_secs",
                self.search.embedding.timeout_secs,
            )?,
            max_retries: validate_positive_u32(
                "search.embedding.max_retries",
                self.search.embedding.max_retries,
            )?,
            retry_backoff_ms: validate_positive_u64(
                "search.embedding.retry_backoff_ms",
                self.search.embedding.retry_backoff_ms,
            )?,
        }))
    }

    /// Return effective MCP worker thread count, clamped to [1, 64].
    pub fn mcp_worker_threads(&self) -> usize {
        self.mcp.worker_threads.clamp(1, 64)
    }

    /// Return effective MCP tool timeout in milliseconds, clamped to [1_000, 3_600_000].
    pub fn mcp_tool_timeout_ms(&self) -> u64 {
        self.mcp.tool_timeout_ms.clamp(1_000, 3_600_000)
    }

    pub fn mcp_tool_timeout_ms_by_tool(&self) -> HashMap<String, u64> {
        self.mcp
            .tool_timeout_ms_by_tool
            .iter()
            .map(|(tool, timeout_ms)| (tool.clone(), (*timeout_ms).clamp(1_000, 3_600_000)))
            .collect()
    }

    pub fn mcp_tool_timeout_ms_for(&self, tool_name: &str) -> u64 {
        self.mcp_tool_timeout_ms_by_tool()
            .get(tool_name)
            .copied()
            .unwrap_or_else(|| self.mcp_tool_timeout_ms())
    }

    pub fn mcp_http_auth(&self) -> Result<Option<ValidatedMcpHttpAuthConfig>> {
        let auth = &self.mcp.http_auth;
        if !auth.enabled {
            return Ok(None);
        }

        let issuer = auth.issuer.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "invalid config: mcp.http_auth.issuer is required when mcp.http_auth.enabled=true"
            )
        })?;
        let resource = auth.resource.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "invalid config: mcp.http_auth.resource is required when mcp.http_auth.enabled=true"
            )
        })?;
        if auth.discovery_url.is_some() && auth.jwks_url.is_some() {
            anyhow::bail!(
                "invalid config: mcp.http_auth.discovery_url and mcp.http_auth.jwks_url are mutually exclusive"
            );
        }

        let issuer = validate_nonempty_string("mcp.http_auth.issuer", issuer)?;
        let resource = validate_nonempty_string("mcp.http_auth.resource", resource)?;
        let discovery_url = auth
            .discovery_url
            .as_deref()
            .map(|value| validate_nonempty_string("mcp.http_auth.discovery_url", value))
            .transpose()?;
        let jwks_url = auth
            .jwks_url
            .as_deref()
            .map(|value| validate_nonempty_string("mcp.http_auth.jwks_url", value))
            .transpose()?;

        let mut required_scopes = HashMap::new();
        for (route, scopes) in &auth.required_scopes {
            let route = validate_nonempty_string("mcp.http_auth.required_scopes.<route>", route)?;
            let mut cleaned = scopes
                .iter()
                .map(|scope| {
                    validate_nonempty_string("mcp.http_auth.required_scopes.<scope>", scope)
                })
                .collect::<Result<Vec<_>>>()?;
            cleaned.sort();
            cleaned.dedup();
            if cleaned.is_empty() {
                anyhow::bail!(
                    "invalid config: mcp.http_auth.required_scopes.{route} must contain at least one scope"
                );
            }
            required_scopes.insert(route, cleaned);
        }

        if !required_scopes.contains_key("mcp") {
            anyhow::bail!(
                "invalid config: mcp.http_auth.required_scopes.mcp is required when mcp.http_auth.enabled=true"
            );
        }

        let mut allowed_origins = auth
            .allowed_origins
            .iter()
            .map(|origin| validate_nonempty_string("mcp.http_auth.allowed_origins[]", origin))
            .collect::<Result<Vec<_>>>()?;
        allowed_origins.sort();
        allowed_origins.dedup();

        Ok(Some(ValidatedMcpHttpAuthConfig {
            issuer,
            discovery_url,
            jwks_url,
            resource,
            required_scopes,
            allowed_origins,
        }))
    }
}

fn render_section(name: &str, fields: &[(&str, String)], active: bool) -> Vec<String> {
    let mut lines = vec![format!("[{}]", name)];
    for (key, value) in fields {
        if active {
            lines.push(format!("{key} = {value}"));
        } else {
            lines.push(format!("# {key} = {value}"));
        }
    }
    lines.push(String::new());
    lines
}

fn render_optional_string(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{value}\""),
        None => "\"\"".to_owned(),
    }
}

fn render_optional_example_string(value: Option<&str>, example: &str) -> String {
    match value {
        Some(value) => format!("\"{value}\""),
        None => format!("\"{example}\""),
    }
}

fn render_string_array(values: &[String]) -> String {
    let rendered = values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rendered}]")
}

fn render_string_array_map(values: &HashMap<String, Vec<String>>) -> String {
    let mut items = values.iter().collect::<Vec<_>>();
    items.sort_by(|left, right| left.0.cmp(right.0));
    let rendered = items
        .into_iter()
        .map(|(key, value)| format!("{key} = {}", render_string_array(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{ {rendered} }}")
}

fn render_timeout_map(values: &HashMap<String, u64>) -> String {
    if values.is_empty() {
        return "{}".to_owned();
    }

    let mut pairs = values.iter().collect::<Vec<_>>();
    pairs.sort_by(|left, right| left.0.cmp(right.0));
    let rendered = pairs
        .into_iter()
        .map(|(key, value)| format!("{key} = {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{ {rendered} }}")
}

fn render_insights_layer_rules(
    rules: &[InsightsLayerRule],
    profile: ConfigTemplateProfile,
) -> Vec<String> {
    let active = profile == ConfigTemplateProfile::Full;
    if rules.is_empty() && active {
        return Vec::new();
    }

    let rendered_rules: Vec<InsightsLayerRule> = if rules.is_empty() {
        vec![InsightsLayerRule {
            name: "layer_1".to_owned(),
            path_prefixes: vec!["src/path-prefix".to_owned()],
            module_prefixes: vec!["crate::module_prefix".to_owned()],
        }]
    } else {
        rules.to_vec()
    };

    let mut lines = Vec::new();
    for (index, rule) in rendered_rules.iter().enumerate() {
        if active {
            lines.push("[[insights.layer_rules]]".to_owned());
            lines.push(format!("name = \"{}\"", rule.name));
            lines.push(format!(
                "path_prefixes = {}",
                render_string_array(&rule.path_prefixes)
            ));
            lines.push(format!(
                "module_prefixes = {}",
                render_string_array(&rule.module_prefixes)
            ));
        } else {
            lines.push("# [[insights.layer_rules]]".to_owned());
            lines.push(format!("# name = \"layer_{}\"", index + 1));
            lines.push("# path_prefixes = [\"src/path-prefix\"]".to_owned());
            lines.push("# module_prefixes = [\"crate::module_prefix\"]".to_owned());
        }
        lines.push(String::new());
    }
    lines
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

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InsightsLayerRule {
    pub name: String,
    pub path_prefixes: Vec<String>,
    pub module_prefixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SanitizationConfig {
    pub redaction_rules_file: Option<String>,
}

impl SanitizationConfig {
    pub fn resolve_redaction_rules_file(
        &self,
        atlas_dir: &Path,
    ) -> Result<Option<std::path::PathBuf>> {
        let Some(raw_path) = self.redaction_rules_file.as_deref() else {
            return Ok(None);
        };
        let trimmed = validate_nonempty_string("sanitization.redaction_rules_file", raw_path)?;
        let candidate = Path::new(&trimmed);
        let resolved = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            atlas_dir.join(candidate)
        };
        Ok(Some(resolved))
    }

    pub fn validate(&self, atlas_dir: &Path) -> Result<()> {
        let Some(path) = self.resolve_redaction_rules_file(atlas_dir)? else {
            return Ok(());
        };
        if !path.exists() {
            anyhow::bail!(
                "invalid config: sanitization.redaction_rules_file points to missing file {}",
                path.display()
            );
        }
        if !path.is_file() {
            anyhow::bail!(
                "invalid config: sanitization.redaction_rules_file must point to a readable file, got {}",
                path.display()
            );
        }
        atlas_adapters::load_redaction_rules_file(&path).with_context(|| {
            format!(
                "invalid config: sanitization.redaction_rules_file={} failed validation",
                path.display()
            )
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InsightsConfig {
    pub large_function_loc: usize,
    pub repeated_call_chain_min_length: usize,
    pub high_fan_in: usize,
    pub high_fan_out: usize,
    pub high_coupling: usize,
    pub deep_chain_length: usize,
    pub max_findings: usize,
    pub high_cyclomatic_complexity: usize,
    pub high_cognitive_complexity: usize,
    pub max_nesting_depth: usize,
    pub branch_count: usize,
    pub outlier_percentile_cutoff: usize,
    pub risk_public_api_weight: f64,
    pub risk_fan_in_weight: f64,
    pub risk_fan_out_weight: f64,
    pub risk_cross_module_dependency_weight: f64,
    pub risk_test_adjacency_mitigation_weight: f64,
    pub risk_dependency_depth_weight: f64,
    pub risk_unresolved_edge_weight: f64,
    pub risk_large_function_weight: f64,
    pub risk_loc_weight: f64,
    pub risk_cyclomatic_complexity_weight: f64,
    pub risk_cognitive_complexity_weight: f64,
    pub risk_nesting_depth_weight: f64,
    pub risk_cycle_participation_weight: f64,
    pub risk_medium_threshold: f64,
    pub risk_high_threshold: f64,
    pub ignore_files: Vec<String>,
    pub ignore_modules: Vec<String>,
    pub ignore_node_kinds: Vec<String>,
    pub layer_rules: Vec<InsightsLayerRule>,
}

impl Default for InsightsConfig {
    fn default() -> Self {
        Self {
            large_function_loc: 80,
            repeated_call_chain_min_length: 3,
            high_fan_in: 20,
            high_fan_out: 10,
            high_coupling: 15,
            deep_chain_length: 6,
            max_findings: 50,
            high_cyclomatic_complexity: 15,
            high_cognitive_complexity: 20,
            max_nesting_depth: 4,
            branch_count: 12,
            outlier_percentile_cutoff: 95,
            risk_public_api_weight: 1.5,
            risk_fan_in_weight: 1.25,
            risk_fan_out_weight: 0.75,
            risk_cross_module_dependency_weight: 1.0,
            risk_test_adjacency_mitigation_weight: 1.0,
            risk_dependency_depth_weight: 0.75,
            risk_unresolved_edge_weight: 1.25,
            risk_large_function_weight: 0.5,
            risk_loc_weight: 0.75,
            risk_cyclomatic_complexity_weight: 1.0,
            risk_cognitive_complexity_weight: 1.0,
            risk_nesting_depth_weight: 0.75,
            risk_cycle_participation_weight: 1.0,
            risk_medium_threshold: 35.0,
            risk_high_threshold: 70.0,
            ignore_files: Vec::new(),
            ignore_modules: Vec::new(),
            ignore_node_kinds: Vec::new(),
            layer_rules: Vec::new(),
        }
    }
}

impl InsightsConfig {
    pub fn validate(&self) -> Result<()> {
        validate_usize_limit(
            "insights.large_function_loc",
            self.large_function_loc,
            usize::MAX,
        )?;
        validate_usize_limit(
            "insights.repeated_call_chain_min_length",
            self.repeated_call_chain_min_length,
            usize::MAX,
        )?;
        if self.repeated_call_chain_min_length < 2 {
            anyhow::bail!(
                "invalid config: insights.repeated_call_chain_min_length={} must be at least 2",
                self.repeated_call_chain_min_length,
            );
        }
        validate_usize_limit("insights.high_fan_in", self.high_fan_in, usize::MAX)?;
        validate_usize_limit("insights.high_fan_out", self.high_fan_out, usize::MAX)?;
        validate_usize_limit("insights.high_coupling", self.high_coupling, usize::MAX)?;
        validate_usize_limit(
            "insights.deep_chain_length",
            self.deep_chain_length,
            usize::MAX,
        )?;
        validate_usize_limit("insights.max_findings", self.max_findings, usize::MAX)?;
        validate_usize_limit(
            "insights.high_cyclomatic_complexity",
            self.high_cyclomatic_complexity,
            usize::MAX,
        )?;
        validate_usize_limit(
            "insights.high_cognitive_complexity",
            self.high_cognitive_complexity,
            usize::MAX,
        )?;
        validate_usize_limit(
            "insights.max_nesting_depth",
            self.max_nesting_depth,
            usize::MAX,
        )?;
        validate_usize_limit("insights.branch_count", self.branch_count, usize::MAX)?;
        validate_usize_limit(
            "insights.outlier_percentile_cutoff",
            self.outlier_percentile_cutoff,
            100,
        )?;
        validate_positive_f64(
            "insights.risk_public_api_weight",
            self.risk_public_api_weight,
        )?;
        validate_positive_f64("insights.risk_fan_in_weight", self.risk_fan_in_weight)?;
        validate_positive_f64("insights.risk_fan_out_weight", self.risk_fan_out_weight)?;
        validate_positive_f64(
            "insights.risk_cross_module_dependency_weight",
            self.risk_cross_module_dependency_weight,
        )?;
        validate_positive_f64(
            "insights.risk_test_adjacency_mitigation_weight",
            self.risk_test_adjacency_mitigation_weight,
        )?;
        validate_positive_f64(
            "insights.risk_dependency_depth_weight",
            self.risk_dependency_depth_weight,
        )?;
        validate_positive_f64(
            "insights.risk_unresolved_edge_weight",
            self.risk_unresolved_edge_weight,
        )?;
        validate_positive_f64(
            "insights.risk_large_function_weight",
            self.risk_large_function_weight,
        )?;
        validate_positive_f64("insights.risk_loc_weight", self.risk_loc_weight)?;
        validate_positive_f64(
            "insights.risk_cyclomatic_complexity_weight",
            self.risk_cyclomatic_complexity_weight,
        )?;
        validate_positive_f64(
            "insights.risk_cognitive_complexity_weight",
            self.risk_cognitive_complexity_weight,
        )?;
        validate_positive_f64(
            "insights.risk_nesting_depth_weight",
            self.risk_nesting_depth_weight,
        )?;
        validate_positive_f64(
            "insights.risk_cycle_participation_weight",
            self.risk_cycle_participation_weight,
        )?;
        validate_f64_range(
            "insights.risk_medium_threshold",
            self.risk_medium_threshold,
            0.0,
            100.0,
        )?;
        validate_f64_range(
            "insights.risk_high_threshold",
            self.risk_high_threshold,
            0.0,
            100.0,
        )?;
        if self.risk_medium_threshold >= self.risk_high_threshold {
            anyhow::bail!(
                "invalid config: insights.risk_medium_threshold ({}) must be less than insights.risk_high_threshold ({})",
                self.risk_medium_threshold,
                self.risk_high_threshold,
            );
        }

        for (index, value) in self.ignore_files.iter().enumerate() {
            validate_nonempty_string(&format!("insights.ignore_files[{index}]"), value)?;
        }
        for (index, value) in self.ignore_modules.iter().enumerate() {
            validate_nonempty_string(&format!("insights.ignore_modules[{index}]"), value)?;
        }
        for (index, value) in self.ignore_node_kinds.iter().enumerate() {
            validate_nonempty_string(&format!("insights.ignore_node_kinds[{index}]"), value)?;
        }

        let mut seen_names = std::collections::BTreeSet::new();
        for (index, rule) in self.layer_rules.iter().enumerate() {
            let name = validate_nonempty_string(
                &format!("insights.layer_rules[{index}].name"),
                &rule.name,
            )?;
            if !seen_names.insert(name.clone()) {
                anyhow::bail!(
                    "invalid config: insights.layer_rules[{index}].name duplicates layer `{name}`"
                );
            }
            if rule.path_prefixes.is_empty() && rule.module_prefixes.is_empty() {
                anyhow::bail!(
                    "invalid config: insights.layer_rules[{index}] must define path_prefixes or module_prefixes"
                );
            }
            for (matcher_index, matcher) in rule.path_prefixes.iter().enumerate() {
                validate_nonempty_string(
                    &format!("insights.layer_rules[{index}].path_prefixes[{matcher_index}]"),
                    matcher,
                )?;
            }
            for (matcher_index, matcher) in rule.module_prefixes.iter().enumerate() {
                validate_nonempty_string(
                    &format!("insights.layer_rules[{index}].module_prefixes[{matcher_index}]"),
                    matcher,
                )?;
            }
        }

        Ok(())
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
        assert!(
            config.mcp_tool_timeout_ms_by_tool().is_empty(),
            "default config should not invent per-tool timeout overrides"
        );
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

        config
            .mcp
            .tool_timeout_ms_by_tool
            .insert("query_graph".to_owned(), 5);
        config
            .mcp
            .tool_timeout_ms_by_tool
            .insert("build_or_update_graph".to_owned(), 9_999_999);
        let overrides = config.mcp_tool_timeout_ms_by_tool();
        assert_eq!(overrides.get("query_graph"), Some(&1_000));
        assert_eq!(overrides.get("build_or_update_graph"), Some(&3_600_000));
    }

    #[test]
    fn mcp_tool_timeout_prefers_per_tool_override() {
        let mut config = Config::default();
        config.mcp.tool_timeout_ms = 30_000;
        config
            .mcp
            .tool_timeout_ms_by_tool
            .insert("query_graph".to_owned(), 5_000);

        assert_eq!(config.mcp_tool_timeout_ms_for("query_graph"), 5_000);
        assert_eq!(
            config.mcp_tool_timeout_ms_for("build_or_update_graph"),
            30_000
        );
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
            "[mcp]\nmax_mcp_response_bytes = 4096\n\n[context]\nmax_saved_context_bytes = 256\n\n[search.embedding]\nurl = \"http://embed.test\"\n",
        )
        .expect("write config");

        let config = Config::load(atlas_dir).expect("load config");

        assert_eq!(config.mcp.max_mcp_response_bytes, 4096);
        assert_eq!(config.context.max_saved_context_bytes, 256);
        assert_eq!(
            config.search.embedding.url.as_deref(),
            Some("http://embed.test")
        );
        assert_eq!(config.mcp.worker_threads, DEFAULT_MCP_WORKER_THREADS);
        assert!(config.mcp.tool_timeout_ms_by_tool.is_empty());
    }

    #[test]
    fn mcp_http_auth_exact_config_parsing_round_trips() {
        let dir = tempdir().expect("tempdir");
        let atlas_dir = dir.path();
        fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            "[mcp.http_auth]\nenabled = true\nissuer = \"https://auth.example\"\ndiscovery_url = \"https://auth.example/.well-known/openid-configuration\"\nresource = \"https://atlas.example/mcp\"\nrequired_scopes = { mcp = [\"atlas:mcp\", \"atlas:read\"] }\nallowed_origins = [\"https://app.example\"]\n",
        )
        .expect("write config");

        let config = Config::load(atlas_dir).expect("load config");
        let auth = config
            .mcp_http_auth()
            .expect("validated auth config")
            .expect("auth config present");
        assert_eq!(auth.issuer, "https://auth.example");
        assert_eq!(
            auth.discovery_url.as_deref(),
            Some("https://auth.example/.well-known/openid-configuration")
        );
        assert_eq!(auth.jwks_url, None);
        assert_eq!(auth.resource, "https://atlas.example/mcp");
        assert_eq!(
            auth.required_scopes.get("mcp"),
            Some(&vec!["atlas:mcp".to_owned(), "atlas:read".to_owned()])
        );
        assert_eq!(auth.allowed_origins, vec!["https://app.example".to_owned()]);
    }

    #[test]
    fn mcp_http_auth_missing_required_fields_fail_closed() {
        let dir = tempdir().expect("tempdir");
        let atlas_dir = dir.path();
        fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            "[mcp.http_auth]\nenabled = true\nissuer = \"https://auth.example\"\n",
        )
        .expect("write config");

        let config = Config::load(atlas_dir).expect("load config");
        let error = config
            .mcp_http_auth()
            .expect_err("auth config should fail closed");
        assert!(
            error
                .to_string()
                .contains("mcp.http_auth.resource is required")
        );
    }

    #[test]
    fn mcp_http_auth_rejects_discovery_and_jwks_together() {
        let dir = tempdir().expect("tempdir");
        let atlas_dir = dir.path();
        fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            "[mcp.http_auth]\nenabled = true\nissuer = \"https://auth.example\"\ndiscovery_url = \"https://auth.example/.well-known/openid-configuration\"\njwks_url = \"https://auth.example/jwks\"\nresource = \"https://atlas.example/mcp\"\nrequired_scopes = { mcp = [\"atlas:mcp\"] }\n",
        )
        .expect("write config");

        let config = Config::load(atlas_dir).expect("load config");
        let error = config
            .mcp_http_auth()
            .expect_err("discovery+jwks should be rejected");
        assert!(error.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn render_template_minimal_comments_all_keys() {
        let template = Config::render_template(ConfigTemplateProfile::Minimal).expect("template");

        assert!(template.contains("# parse_batch_size = 64"));
        assert!(template.contains("[search.embedding]\n# url = \"http://localhost:11434\""));
        assert!(template.contains("[insights]\n# large_function_loc = 80"));
        assert!(template.contains("# repeated_call_chain_min_length = 3"));
        assert!(template.contains("# outlier_percentile_cutoff = 95"));
        assert!(template.contains("# [[insights.layer_rules]]\n# name = \"layer_1\""));
        assert!(
            template.contains("[sanitization]\n# redaction_rules_file = \"redaction-rules.toml\"")
        );
        assert!(template.contains("# worker_threads = 2"));
        assert!(template.contains("[mcp.http_auth]\n# enabled = false"));
        assert!(!template.contains("\nparse_batch_size = 64\n"));
    }

    #[test]
    fn rendered_minimal_template_loads_without_layer_rule_validation_error() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join(crate::paths::ATLAS_CONFIG),
            Config::render_template(ConfigTemplateProfile::Minimal).expect("template"),
        )
        .expect("write config");

        Config::load(dir.path()).expect("minimal template should load");
    }

    #[test]
    fn render_template_full_activates_keys() {
        let template = Config::render_template(ConfigTemplateProfile::Full).expect("template");

        assert!(template.contains("[build]\nparse_batch_size = 64"));
        assert!(template.contains("tool_timeout_ms_by_tool = { build_or_update_graph = 900000, get_review_context = 120000 }"));
        assert!(template.contains("hybrid_enabled = true"));
        assert!(template.contains("[search.embedding]\nurl = \"http://localhost:11434\""));
        assert!(template.contains("[insights]\nlarge_function_loc = 60"));
        assert!(template.contains("repeated_call_chain_min_length = 4"));
        assert!(template.contains("outlier_percentile_cutoff = 90"));
        assert!(template.contains("ignore_node_kinds = [\"import\"]"));
        assert!(template.contains("[[insights.layer_rules]]\nname = \"api\""));
        assert!(template.contains("[sanitization]\nredaction_rules_file = \"\""));
        assert!(template.contains("[mcp.http_auth]\nenabled = true"));
        assert!(template.contains("required_scopes = { mcp = [\"atlas:mcp\", \"atlas:read\"] }"));
    }

    #[test]
    fn write_template_uses_selected_profile() {
        let dir = tempdir().expect("tempdir");
        let created = Config::write_template(dir.path(), ConfigTemplateProfile::Full)
            .expect("write template");

        assert!(created);
        let text =
            fs::read_to_string(dir.path().join(crate::paths::ATLAS_CONFIG)).expect("read config");
        assert!(text.contains("# profile = \"full\""));
        assert!(text.contains("hybrid_enabled = true"));
        assert!(text.contains("url = \"http://localhost:11434\""));
        assert!(text.contains("large_function_loc = 60"));
        assert!(text.contains("repeated_call_chain_min_length = 4"));
    }

    #[test]
    fn insights_config_rejects_non_positive_thresholds() {
        let mut config = Config::default();
        config.insights.max_findings = 0;

        let err = config
            .insights_config()
            .expect_err("invalid insights config");
        assert!(
            err.to_string()
                .contains("insights.max_findings must be greater than 0")
        );
    }

    #[test]
    fn insights_config_rejects_outlier_percentile_above_100() {
        let mut config = Config::default();
        config.insights.outlier_percentile_cutoff = 101;

        let err = config
            .insights_config()
            .expect_err("invalid insights percentile cutoff");
        assert!(
            err.to_string()
                .contains("insights.outlier_percentile_cutoff=101 exceeds safe maximum 100")
        );
    }

    #[test]
    fn insights_config_rejects_invalid_risk_threshold_order() {
        let mut config = Config::default();
        config.insights.risk_medium_threshold = 80.0;
        config.insights.risk_high_threshold = 70.0;

        let err = config
            .insights_config()
            .expect_err("invalid risk threshold order");
        assert!(err.to_string().contains("insights.risk_medium_threshold (80) must be less than insights.risk_high_threshold (70)"));
    }

    #[test]
    fn insights_config_rejects_non_positive_risk_weight() {
        let mut config = Config::default();
        config.insights.risk_unresolved_edge_weight = 0.0;

        let err = config.insights_config().expect_err("invalid risk weight");
        assert!(err.to_string().contains(
            "insights.risk_unresolved_edge_weight must be a finite value greater than 0"
        ));
    }

    #[test]
    fn load_rejects_invalid_insights_layer_rule() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join(crate::paths::ATLAS_CONFIG),
            "[insights]\nmax_findings = 10\n\n[[insights.layer_rules]]\nname = \"app\"\n",
        )
        .expect("write config");

        let err = Config::load(dir.path()).expect_err("invalid layer rule");
        assert!(
            err.to_string()
                .contains("insights.layer_rules[0] must define path_prefixes or module_prefixes")
        );
    }

    #[test]
    fn load_accepts_valid_external_redaction_rules_file() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("redaction-rules.toml"),
            "token_prefixes = [\"zz-\"]\nsecret_key_patterns = [\"sessionid\"]\ntoken_min_len = 3\nhex_secret_min_len = 16\nbase64_secret_min_len = 20\n",
        )
        .expect("write rules");
        fs::write(
            dir.path().join(crate::paths::ATLAS_CONFIG),
            "[sanitization]\nredaction_rules_file = \"redaction-rules.toml\"\n",
        )
        .expect("write config");

        let config = Config::load(dir.path()).expect("config should load");
        let resolved = config
            .resolve_redaction_rules_file(dir.path())
            .expect("resolve path")
            .expect("configured path");
        assert!(resolved.ends_with("redaction-rules.toml"));
    }

    #[test]
    fn load_rejects_missing_external_redaction_rules_file() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join(crate::paths::ATLAS_CONFIG),
            "[sanitization]\nredaction_rules_file = \"missing-rules.toml\"\n",
        )
        .expect("write config");

        let err = Config::load(dir.path()).expect_err("missing rules must fail");
        assert!(
            err.to_string()
                .contains("sanitization.redaction_rules_file points to missing file")
        );
    }

    #[test]
    fn load_rejects_unreadable_external_redaction_rules_file() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("rules-dir")).expect("create rules dir");
        fs::write(
            dir.path().join(crate::paths::ATLAS_CONFIG),
            "[sanitization]\nredaction_rules_file = \"rules-dir\"\n",
        )
        .expect("write config");

        let err = Config::load(dir.path()).expect_err("directory path must fail");
        assert!(
            err.to_string()
                .contains("sanitization.redaction_rules_file must point to a readable file")
        );
    }

    #[test]
    fn load_rejects_malformed_external_redaction_rules_file() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("redaction-rules.toml"),
            "token_prefixes = [",
        )
        .expect("write malformed rules");
        fs::write(
            dir.path().join(crate::paths::ATLAS_CONFIG),
            "[sanitization]\nredaction_rules_file = \"redaction-rules.toml\"\n",
        )
        .expect("write config");

        let err = Config::load(dir.path()).expect_err("malformed rules must fail");
        let message = err.to_string();
        assert!(message.contains("sanitization.redaction_rules_file"));
        assert!(
            message.contains("cannot parse redaction rules file")
                || message.contains("failed validation")
        );
    }

    #[test]
    fn embedding_backend_returns_none_when_url_missing() {
        let config = Config::default();

        assert!(
            config
                .embedding_backend()
                .expect("embedding backend")
                .is_none()
        );
    }

    #[test]
    fn embedding_backend_validates_and_returns_values() {
        let mut config = Config::default();
        config.search.embedding.url = Some(" http://embed.test ".to_owned());

        let backend = config
            .embedding_backend()
            .expect("embedding backend")
            .expect("configured backend");

        assert_eq!(backend.url, "http://embed.test");
        assert_eq!(backend.model, DEFAULT_EMBED_MODEL);
        assert_eq!(backend.timeout_secs, DEFAULT_EMBED_TIMEOUT_SECS);
        assert_eq!(backend.max_retries, DEFAULT_EMBED_MAX_RETRIES);
        assert_eq!(backend.retry_backoff_ms, DEFAULT_EMBED_RETRY_BACKOFF_MS);
    }

    #[test]
    fn embedding_backend_rejects_zero_timeout() {
        let mut config = Config::default();
        config.search.embedding.url = Some("http://embed.test".to_owned());
        config.search.embedding.timeout_secs = 0;

        let err = config
            .embedding_backend()
            .expect_err("invalid embedding config");
        assert!(
            err.to_string()
                .contains("search.embedding.timeout_secs must be greater than 0")
        );
    }
}
