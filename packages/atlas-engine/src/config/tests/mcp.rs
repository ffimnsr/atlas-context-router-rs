//! MCP config defaults/clamping, http-auth validation, and embedding
//! backend resolution tests.

use super::super::*;
use super::*;
use atlas_core::BudgetPolicy;
use std::fs;

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
fn memory_config_defaults_reject_custom_frontends_and_parse_flag() {
    assert!(!Config::default().allow_custom_frontends());

    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[memory]\nallow_custom_frontends = true\n",
    )
    .unwrap();
    let config = Config::load(dir.path()).unwrap();
    assert!(config.allow_custom_frontends());
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
