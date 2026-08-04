//! Effective-value accessors for the top-level `Config` (clamped limits,
//! embedding backend resolution, MCP timeouts, HTTP auth validation).

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use super::build::BuildRunBudget;
use super::insights::InsightsConfig;
use super::{
    Config, EmbeddingBackendConfig, ValidatedMcpHttpAuthConfig, validate_nonempty_string,
    validate_positive_u32, validate_positive_u64,
};

impl Config {
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

    /// Whether memory surfaces accept frontend identities beyond the known set.
    pub fn allow_custom_frontends(&self) -> bool {
        self.memory.allow_custom_frontends
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
