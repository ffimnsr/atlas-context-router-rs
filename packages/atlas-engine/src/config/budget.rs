//! `Config::budget_policy`: map validated config values onto the central
//! `BudgetPolicy` used by CLI/MCP payload serialization.

use anyhow::Result;
use atlas_core::{BudgetLimitRule, BudgetPolicy};

use super::{Config, validate_u64_limit, validate_usize_limit};

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
