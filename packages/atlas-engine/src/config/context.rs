//! Context-engine configuration (symbol/file/review context bounds).

use anyhow::Result;
use atlas_core::BudgetPolicy;
use serde::{Deserialize, Serialize};

use super::validate_nonempty_string;

/// Tokenizer accounting provider for context payload budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TokenizerProvider {
    /// Byte heuristic: `bytes.div_ceil(bytes_per_token)`.
    #[default]
    Heuristic,
    /// Local tokenizer JSON file loaded via `atlas-token-count`.
    Tokenizers,
}

impl TokenizerProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::Tokenizers => "tokenizers",
        }
    }
}

/// Behavior when tokenizer loading fails at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TokenizerFallbackMode {
    /// Fall back to the byte heuristic and record fallback metadata.
    #[default]
    Heuristic,
    /// Error before payload truncation when the tokenizer cannot load.
    FailClosed,
}

impl TokenizerFallbackMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::FailClosed => "fail_closed",
        }
    }
}

/// Token counting configuration (`context.tokenizer`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextTokenizerConfig {
    /// Counting provider: byte heuristic (default) or local tokenizer file.
    pub provider: TokenizerProvider,
    /// Optional model identifier preserved as count-method metadata.
    pub model: Option<String>,
    /// Local tokenizer JSON path; relative paths resolve from `.atlas/`.
    pub tokenizer_file: Option<String>,
    /// Behavior when tokenizer loading fails.
    pub fallback: TokenizerFallbackMode,
    /// Bytes per token for the heuristic (`bytes.div_ceil(bytes_per_token)`).
    pub bytes_per_token: usize,
}

impl Default for ContextTokenizerConfig {
    fn default() -> Self {
        Self {
            provider: TokenizerProvider::Heuristic,
            model: None,
            tokenizer_file: None,
            fallback: TokenizerFallbackMode::Heuristic,
            bytes_per_token: 4,
        }
    }
}

impl ContextTokenizerConfig {
    /// Structural validation; does not check that `tokenizer_file` exists.
    /// Existence is checked by the runtime builder so missing files can
    /// fall back or fail closed per `fallback`.
    pub fn validate(&self) -> Result<()> {
        if self.bytes_per_token == 0 {
            anyhow::bail!(
                "invalid config: context.tokenizer.bytes_per_token must be greater than 0"
            );
        }
        if let Some(model) = self.model.as_deref() {
            validate_nonempty_string("context.tokenizer.model", model)?;
        }
        if let Some(file) = self.tokenizer_file.as_deref() {
            validate_nonempty_string("context.tokenizer.tokenizer_file", file)?;
        }
        match self.provider {
            TokenizerProvider::Tokenizers => {
                if self.tokenizer_file.is_none() {
                    anyhow::bail!(
                        "invalid config: context.tokenizer.tokenizer_file is required when context.tokenizer.provider = \"tokenizers\""
                    );
                }
            }
            TokenizerProvider::Heuristic => {
                if self.tokenizer_file.is_some() {
                    anyhow::bail!(
                        "invalid config: context.tokenizer.tokenizer_file must not be set when context.tokenizer.provider = \"heuristic\""
                    );
                }
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
    /// Tokenizer accounting for context payload budgets.
    pub tokenizer: ContextTokenizerConfig,
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
            tokenizer: ContextTokenizerConfig::default(),
        }
    }
}
