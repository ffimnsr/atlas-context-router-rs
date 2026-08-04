//! MCP transport configuration (worker threads, timeouts, HTTP auth).

use std::collections::HashMap;

use atlas_core::BudgetPolicy;
use serde::{Deserialize, Serialize};

pub const DEFAULT_MCP_WORKER_THREADS: usize = 2;
pub const DEFAULT_MCP_TOOL_TIMEOUT_MS: u64 = 300_000;

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
