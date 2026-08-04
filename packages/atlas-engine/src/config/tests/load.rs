//! Budget-policy mapping and partial-section loading tests.

use super::super::*;
use super::*;
use std::fs;

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
