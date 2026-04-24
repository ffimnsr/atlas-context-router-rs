// Phase 22 — Context Engine: Slices 3, 4, 5, 6, 8
// Phase CM6 — Retrieval-backed restoration
//
// Slice 3: resolve_target
// Slice 4: build_symbol_context
// Slice 5: rank_context / trim_context
// Slice 6: build_review_context / build_impact_context
// Slice 8: apply_code_spans
// CM6: retrieve_saved_context

pub(super) use std::collections::{HashMap, HashSet};

pub(super) use atlas_contentstore::{
    ContentStore,
    store::{SearchFilters, SourceRow},
};
pub(super) use atlas_core::{
    BudgetManager, BudgetPolicy, BudgetReport, Result,
    model::{
        AmbiguityMeta, ContextRankingEvidence, ContextRequest, ContextResult, ContextTarget,
        NoiseReductionSummary, SavedContextSource, SeedBudgetMeta, SelectedEdge, SelectedFile,
        SelectedNode, SelectionReason, TruncationMeta, WorkflowCallChain, WorkflowComponent,
        WorkflowFocusNode, WorkflowSummary,
    },
};
pub(super) use atlas_store_sqlite::Store;

mod build;
mod payload;
mod rank;
mod resolve;
mod saved;
mod spans;
mod symbol;
#[cfg(test)]
mod tests;
mod workflow;

pub use self::build::build_context;
pub use self::resolve::{ResolvedTarget, normalize_qn_kind_tokens, resolve_target};

use self::payload::apply_payload_budgets;
use self::rank::{rank_context, trim_context};
use self::saved::retrieve_saved_context;
use self::spans::apply_code_spans;
use self::symbol::{
    build_ambiguous_result, build_not_found_result, build_symbol_context, collect_files,
    update_file_node_counts,
};
use self::workflow::build_workflow_summary;

/// Default caps when the request does not specify limits.
/// Shared trimming primitive lives in `crate::ranking::TrimmingPrimitives`.
pub(super) const DEFAULT_MAX_NODES: usize = 50;
pub(super) const DEFAULT_MAX_EDGES: usize = 100;
pub(super) const DEFAULT_MAX_FILES: usize = 20;

/// Per-bucket limits fed to store helpers.
pub(super) const BUCKET_CALLERS: usize = 15;
pub(super) const BUCKET_CALLEES: usize = 15;
pub(super) const BUCKET_IMPORTS: usize = 10;
pub(super) const BUCKET_SIBLINGS: usize = 10;
pub(super) const BUCKET_TESTS: usize = 10;

/// Stateless context engine facade.
///
/// Wraps the free-function pipeline into a struct so callers inject one
/// [`Store`] reference and call engine operations as methods.  An optional
/// [`ContentStore`] enables CM6 saved-context retrieval.
pub struct ContextEngine<'a> {
    store: &'a Store,
    /// Optional content store for CM6 retrieval-backed restoration.
    content_store: Option<&'a ContentStore>,
    budget_policy: BudgetPolicy,
}

impl<'a> ContextEngine<'a> {
    /// Create a new engine backed by `store`.
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            content_store: None,
            budget_policy: BudgetPolicy::default(),
        }
    }

    /// Attach a content store to enable saved-context retrieval (CM6).
    ///
    /// When attached and `request.include_saved_context` is `true`, the engine
    /// queries the content store for relevant saved artifacts after graph
    /// retrieval and merges them into `ContextResult::saved_context_sources`.
    pub fn with_content_store(mut self, cs: &'a ContentStore) -> Self {
        self.content_store = Some(cs);
        self
    }

    pub fn with_budget_policy(mut self, policy: BudgetPolicy) -> Self {
        self.budget_policy = policy;
        self
    }

    /// Resolve a [`ContextTarget`] to a concrete node, file, or ambiguity result.
    pub fn resolve(&self, target: &ContextTarget) -> Result<ResolvedTarget> {
        resolve_target(self.store, target)
    }

    /// Build a bounded [`ContextResult`] for the given request.
    ///
    /// Routes by intent (Review / Impact / Symbol / File), resolves target,
    /// retrieves neighbors, ranks, trims, and optionally applies code spans.
    /// When `request.include_saved_context` is `true` and a content store is
    /// attached, also populates `saved_context_sources` (CM6).
    pub fn build(&self, request: &ContextRequest) -> Result<ContextResult> {
        let mut budgets = BudgetManager::new();
        let mut effective_request = request.clone();
        effective_request.max_nodes = Some(budgets.resolve_limit(
            self.budget_policy.review_context_extraction.nodes,
            "review_context_extraction.max_nodes",
            effective_request.max_nodes,
        ));
        effective_request.max_edges = Some(budgets.resolve_limit(
            self.budget_policy.review_context_extraction.edges,
            "review_context_extraction.max_edges",
            effective_request.max_edges,
        ));
        effective_request.max_files = Some(budgets.resolve_limit(
            self.budget_policy.review_context_extraction.files,
            "review_context_extraction.max_files",
            effective_request.max_files,
        ));
        effective_request.depth = Some(budgets.resolve_limit(
            self.budget_policy.graph_traversal.depth,
            "graph_traversal.max_depth",
            effective_request.depth.map(|depth| depth as usize),
        ) as u32);

        let saved_source_limit = budgets.resolve_limit(
            self.budget_policy.content_saved_context_lookup.sources,
            "content_saved_context_lookup.max_sources",
            None,
        );

        let mut result = build_context(self.store, &effective_request, &self.budget_policy)?;
        budgets.record_report(result.budget.clone());
        if effective_request.include_saved_context
            && let Some(cs) = self.content_store
        {
            let retrieval =
                retrieve_saved_context(cs, &effective_request, &result, saved_source_limit);
            budgets.record_usage(
                self.budget_policy.content_saved_context_lookup.sources,
                "content_saved_context_lookup.max_sources",
                saved_source_limit,
                retrieval.matched_source_count,
                retrieval.matched_source_count > saved_source_limit,
            );
            result.saved_context_sources = retrieval.sources;
        }
        apply_payload_budgets(&mut result, &self.budget_policy);
        let node_limit = result.request.max_nodes.unwrap_or(DEFAULT_MAX_NODES);
        let edge_limit = result.request.max_edges.unwrap_or(DEFAULT_MAX_EDGES);
        let file_limit = result.request.max_files.unwrap_or(DEFAULT_MAX_FILES);
        budgets.record_usage(
            self.budget_policy.review_context_extraction.nodes,
            "review_context_extraction.max_nodes",
            node_limit,
            result.nodes.len() + result.truncation.nodes_dropped,
            result.truncation.nodes_dropped > 0,
        );
        budgets.record_usage(
            self.budget_policy.review_context_extraction.edges,
            "review_context_extraction.max_edges",
            edge_limit,
            result.edges.len() + result.truncation.edges_dropped,
            result.truncation.edges_dropped > 0,
        );
        budgets.record_usage(
            self.budget_policy.review_context_extraction.files,
            "review_context_extraction.max_files",
            file_limit,
            result.files.len() + result.truncation.files_dropped,
            result.truncation.files_dropped > 0,
        );
        record_seed_budget_reports(&mut budgets, &result);
        record_payload_budget_reports(&mut budgets, &result, &self.budget_policy);
        result.budget = budgets.summary(
            "review_context_extraction.max_nodes",
            node_limit,
            result.nodes.len(),
        );
        Ok(result)
    }
}

fn record_seed_budget_reports(budgets: &mut BudgetManager, result: &ContextResult) {
    for seed in &result.seed_budgets {
        if !seed.budget_hit {
            continue;
        }

        let budget_name = match seed.seed_kind.as_str() {
            "changed_files" => "graph_traversal.max_seed_files",
            "changed_symbols" => "graph_traversal.max_seed_nodes",
            "symbol_resolution" => "query_candidates_and_seeds.symbol_resolution",
            other => other,
        };
        let limit = seed.accepted_seed_count.max(1);
        let observed = seed.requested_seed_count.max(seed.accepted_seed_count);
        let report = if !seed.safe_to_answer {
            BudgetReport::blocked(budget_name, limit, observed)
        } else {
            BudgetReport::partial_result(budget_name, limit, observed, true)
        };
        budgets.record_report(report);
    }
}

fn record_payload_budget_reports(
    budgets: &mut BudgetManager,
    result: &ContextResult,
    policy: &BudgetPolicy,
) {
    let Some(payload) = result.truncation.payload.as_ref() else {
        return;
    };

    if payload.bytes_requested
        > policy
            .mcp_cli_payload_serialization
            .context_payload_bytes
            .default_limit
    {
        budgets.record_usage(
            policy.mcp_cli_payload_serialization.context_payload_bytes,
            "mcp_cli_payload_serialization.max_context_payload_bytes",
            policy
                .mcp_cli_payload_serialization
                .context_payload_bytes
                .default_limit,
            payload.bytes_requested,
            true,
        );
    }

    let requested_tokens = payload.bytes_requested.div_ceil(4);
    if requested_tokens
        > policy
            .mcp_cli_payload_serialization
            .context_tokens_estimate
            .default_limit
    {
        budgets.record_usage(
            policy.mcp_cli_payload_serialization.context_tokens_estimate,
            "mcp_cli_payload_serialization.max_context_tokens_estimate",
            policy
                .mcp_cli_payload_serialization
                .context_tokens_estimate
                .default_limit,
            requested_tokens,
            true,
        );
    }
}
