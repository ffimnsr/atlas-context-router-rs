//! Config template rendering: profiles and per-section render helpers.

use std::collections::HashMap;

use anyhow::Result;

use super::Config;
use super::insights::InsightsLayerRule;

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

impl Config {
    pub fn render_template(profile: ConfigTemplateProfile) -> Result<String> {
        let active = Self::profile(profile);
        active.build_run_budget()?;
        active.budget_policy()?;
        active.insights.validate()?;
        active.context.tokenizer.validate()?;

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
                    "similarity_high_threshold",
                    active.insights.similarity_high_threshold.to_string(),
                ),
                (
                    "similarity_medium_threshold",
                    active.insights.similarity_medium_threshold.to_string(),
                ),
                (
                    "similarity_low_threshold",
                    active.insights.similarity_low_threshold.to_string(),
                ),
                (
                    "duplicate_high_threshold",
                    active.insights.duplicate_high_threshold.to_string(),
                ),
                (
                    "duplicate_medium_threshold",
                    active.insights.duplicate_medium_threshold.to_string(),
                ),
                (
                    "duplicate_low_threshold",
                    active.insights.duplicate_low_threshold.to_string(),
                ),
                (
                    "duplicate_suppressions",
                    render_string_array(&active.insights.duplicate_suppressions),
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

        lines.push("# layer_rules_file = \"layer-rules.toml\"".to_owned());

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
            "context.tokenizer",
            &[
                (
                    "provider",
                    format!("\"{}\"", active.context.tokenizer.provider.as_str()),
                ),
                (
                    "model",
                    if profile == ConfigTemplateProfile::Full {
                        render_optional_string(active.context.tokenizer.model.as_deref())
                    } else {
                        render_optional_example_string(
                            active.context.tokenizer.model.as_deref(),
                            "cl100k_base",
                        )
                    },
                ),
                (
                    "tokenizer_file",
                    if profile == ConfigTemplateProfile::Full {
                        render_optional_string(active.context.tokenizer.tokenizer_file.as_deref())
                    } else {
                        render_optional_example_string(
                            active.context.tokenizer.tokenizer_file.as_deref(),
                            "tokenizer.json",
                        )
                    },
                ),
                (
                    "fallback",
                    format!("\"{}\"", active.context.tokenizer.fallback.as_str()),
                ),
                (
                    "bytes_per_token",
                    active.context.tokenizer.bytes_per_token.to_string(),
                ),
            ],
            profile == ConfigTemplateProfile::Full,
        ));
        // Tokenizer-backed example: keep commented unless a real local file
        // exists, so generated templates never activate a missing file.
        lines.push(
            "# To count with a local tokenizer instead of the byte heuristic, activate:".to_owned(),
        );
        lines.push("# provider = \"tokenizers\"".to_owned());
        lines.push(String::new());

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
