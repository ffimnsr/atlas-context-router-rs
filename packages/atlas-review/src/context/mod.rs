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
        EvidenceGap, EvidenceSufficiencyReport, NoiseReductionSummary, SavedContextSource,
        SeedBudgetMeta, SelectedEdge, SelectedFile, SelectedNode, SelectionReason, TruncationMeta,
        WorkflowCallChain, WorkflowComponent, WorkflowFocusNode, WorkflowSummary,
    },
};
pub(super) use atlas_store_sqlite::Store;

mod build;
mod content;
mod experience;
mod overview;
mod payload;
mod rank;
mod resolve;
mod saved;
mod scope;
mod slim;
mod spans;
mod sufficiency;
mod symbol;
#[cfg(test)]
mod tests;
mod workflow;

pub use self::build::build_context;
pub use self::resolve::{
    ResolvedTarget, normalize_qn_kind_tokens, resolve_target, resolve_target_scoped,
};
pub use self::scope::ContextScope;

use self::content::retrieve_content_assets;
use self::experience::apply_experience_shortcuts;
use self::overview::overviews_for_context;
use self::payload::apply_payload_budgets;
use self::rank::{rank_context, trim_context};
use self::saved::retrieve_saved_context;
use self::scope::apply_context_scope;
use self::slim::slim_context_metadata;
use self::spans::apply_code_spans;
use self::sufficiency::{
    budget_exhausted, escalation_reason, evaluate as evaluate_sufficiency, merge_round_two,
};
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
/// [`ContentStore`] enables CM6 saved-context retrieval and the graph/content
/// companion unified selection policy (Patch N2).
pub struct ContextEngine<'a> {
    store: &'a Store,
    /// Optional content store for CM6 retrieval-backed restoration and
    /// graph/content companion content-asset lookup (Patch N2).
    content_store: Option<&'a ContentStore>,
    /// Optional continuity store for ICM-G7 overview memory.
    overview_store: Option<&'a atlas_session::SessionStore>,
    /// Repo root used to scope overview lookups.
    overview_repo_root: Option<String>,
    /// Optional continuity store for VR4 experience edges.
    experience_store: Option<&'a atlas_session::SessionStore>,
    /// Repo root used to scope experience lookups.
    experience_repo_root: Option<String>,
    /// Maximum activated experience shortcuts per build.
    experience_max_edges: usize,
    budget_policy: BudgetPolicy,
    /// Token accounting for serialized payload budgets; defaults to the
    /// byte heuristic (4 bytes per token) for behavior compatibility.
    token_counter: atlas_token_count::TokenCounter,
    /// Load-time fallback metadata for `token_counter`.
    token_fallback: self::payload::TokenFallbackInfo,
    /// Deterministic Round-2 evidence-gap escalation enabled (VR3).
    escalation_enabled: bool,
    /// Maximum escalation rounds (hard cap 3).
    escalation_max_rounds: u32,
}

impl<'a> ContextEngine<'a> {
    /// Create a new engine backed by `store`.
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            content_store: None,
            overview_store: None,
            overview_repo_root: None,
            experience_store: None,
            experience_repo_root: None,
            experience_max_edges: atlas_session::DEFAULT_EXPERIENCE_MAX_EDGES,
            budget_policy: BudgetPolicy::default(),
            token_counter: atlas_token_count::TokenCounter::heuristic(4)
                .expect("bytes_per_token=4 is a valid heuristic"),
            token_fallback: self::payload::TokenFallbackInfo::default(),
            escalation_enabled: true,
            escalation_max_rounds: 1,
        }
    }

    /// Attach a content store to enable saved-context retrieval (CM6) and
    /// graph/content companion content-asset lookup (Patch N2).
    ///
    /// When attached and `request.include_saved_context` is `true`, the engine
    /// queries the content store for relevant saved artifacts after graph
    /// retrieval and merges them into `ContextResult::saved_context_sources`.
    ///
    /// When attached and `request.include_content_assets` is `true`, the engine
    /// also queries for non-code file assets (docs, config, templates, SQL)
    /// adjacent to the request target or changed files, merging them into
    /// `ContextResult::content_assets` under the unified selection ordering
    /// policy (N2).
    pub fn with_content_store(mut self, cs: &'a ContentStore) -> Self {
        self.content_store = Some(cs);
        self
    }

    /// Attach the continuity store so overview memory can be included when
    /// `request.include_overviews` is true (ICM-G7).
    pub fn with_overview_store(
        mut self,
        repo_root: impl Into<String>,
        store: &'a atlas_session::SessionStore,
    ) -> Self {
        self.overview_store = Some(store);
        self.overview_repo_root = Some(repo_root.into());
        self
    }

    /// Attach the continuity store so fresh experience edges can seed Round 1
    /// context with [`SelectionReason::ExperienceShortcut`] targets (VR4).
    pub fn with_experience_store(
        mut self,
        repo_root: impl Into<String>,
        store: &'a atlas_session::SessionStore,
        max_edges: usize,
    ) -> Self {
        self.experience_store = Some(store);
        self.experience_repo_root = Some(repo_root.into());
        self.experience_max_edges = max_edges;
        self
    }

    pub fn with_budget_policy(mut self, policy: BudgetPolicy) -> Self {
        self.budget_policy = policy;
        self
    }

    /// Attach a token counter for serialized payload budget accounting
    /// (tokenizer-backed or heuristic). Defaults to the byte heuristic with
    /// `bytes_per_token = 4` when not set.
    pub fn with_token_counter(mut self, counter: atlas_token_count::TokenCounter) -> Self {
        self.token_counter = counter;
        self
    }

    /// Record load-time fallback metadata for the active counter: true when
    /// the configured tokenizer failed to load and the heuristic was used.
    pub fn with_token_fallback(mut self, used: bool, reason: Option<String>) -> Self {
        self.token_fallback = self::payload::TokenFallbackInfo { used, reason };
        self
    }

    /// Configure bounded evidence-gap escalation (VR3). `max_rounds` is
    /// clamped to the hard cap of 3.
    pub fn with_escalation(mut self, enabled: bool, max_rounds: u32) -> Self {
        self.escalation_enabled = enabled;
        self.escalation_max_rounds = max_rounds.clamp(1, 3);
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
    /// When `request.include_content_assets` is `true` and a content store is
    /// attached, also populates `content_assets` under the unified graph/content
    /// selection ordering policy (Patch N2).
    pub fn build(&self, request: &ContextRequest) -> Result<ContextResult> {
        let started = std::time::Instant::now();
        let intent = context_intent_str(request.intent);
        match self.build_inner(request) {
            Ok(mut result) => {
                let duration_ms = started.elapsed().as_millis() as u64;
                let outcome = classify_context_outcome(&result);
                if let Some(telemetry) = result.telemetry.as_mut() {
                    telemetry.duration_ms = duration_ms;
                    telemetry.outcome = outcome;
                }
                let emitted_bytes = result
                    .telemetry
                    .as_ref()
                    .map(|telemetry| telemetry.emitted_bytes as u64)
                    .unwrap_or(0);
                let emitted_tokens = result
                    .telemetry
                    .as_ref()
                    .map(|telemetry| telemetry.emitted_tokens as u64)
                    .unwrap_or(0);
                atlas_metrics::record_context_build(
                    intent,
                    outcome.as_str(),
                    duration_ms,
                    emitted_bytes,
                    emitted_tokens,
                );
                if !result.request.include_telemetry {
                    result.telemetry = None;
                }
                Ok(result)
            }
            Err(error) => {
                atlas_metrics::record_context_build(
                    intent,
                    "error",
                    started.elapsed().as_millis() as u64,
                    0,
                    0,
                );
                Err(error)
            }
        }
    }

    fn build_inner(&self, request: &ContextRequest) -> Result<ContextResult> {
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

        // N2: resolve content asset limit against policy, then request override.
        let content_asset_limit = budgets.resolve_limit(
            self.budget_policy
                .content_saved_context_lookup
                .content_assets,
            "content_saved_context_lookup.max_content_assets",
            effective_request.max_content_assets,
        );

        let mut result = build_context(self.store, &effective_request, &self.budget_policy)?;
        let scope = ContextScope::from_request(&effective_request)?;
        apply_context_scope(&mut result, &scope);
        if let (Some(session), Some(repo_root)) =
            (self.experience_store, self.experience_repo_root.as_deref())
        {
            apply_experience_shortcuts(
                self.store,
                session,
                repo_root,
                &effective_request,
                self.experience_max_edges,
                &mut result,
            )?;
        }
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
        // N2: retrieve non-code content assets under the unified selection policy.
        // Content assets are populated after graph assembly so the selection
        // ordering is preserved: graph targets first, content assets second,
        // saved artifacts last.
        if effective_request.include_content_assets
            && let Some(cs) = self.content_store
        {
            let assets =
                retrieve_content_assets(cs, &effective_request, &result, content_asset_limit);
            let matched_count = assets.len();
            budgets.record_usage(
                self.budget_policy
                    .content_saved_context_lookup
                    .content_assets,
                "content_saved_context_lookup.max_content_assets",
                content_asset_limit,
                matched_count,
                matched_count >= content_asset_limit,
            );
            result.content_assets = assets;
        }
        // ICM-G7: overview memory is supporting context. Fresh records are
        // included by default; stale records only when the request allows them.
        if effective_request.include_overviews
            && let (Some(session), Some(repo_root)) =
                (self.overview_store, self.overview_repo_root.as_deref())
        {
            result.overviews =
                overviews_for_context(session, repo_root, &effective_request, &result)?;
        }
        // VR3: deterministic evidence-gap escalation. Round 2 fetches only the
        // evidence kinds behind unsatisfied required constraints.
        let mut report = evaluate_sufficiency(&effective_request, &result);
        if self.escalation_enabled && report.is_unsatisfied() {
            report.escalation_reason = escalation_reason(&report.evidence_gaps);
            for round in 0..self.escalation_max_rounds {
                if budget_exhausted(&self.token_counter, &result, effective_request.token_budget) {
                    report.skipped_reason = Some("budget_exhausted".to_owned());
                    break;
                }
                let mut escalated = effective_request.clone();
                escalated.include_tests |=
                    report.evidence_gaps.contains(&EvidenceGap::TestsMissing);
                escalated.include_callers |= report
                    .evidence_gaps
                    .contains(&EvidenceGap::CallersTruncated);
                escalated.include_callees |= report
                    .evidence_gaps
                    .contains(&EvidenceGap::CalleesTruncated);
                escalated.depth = Some(effective_request.depth.unwrap_or(1).saturating_add(1));
                escalated.max_nodes = Some(
                    effective_request
                        .max_nodes
                        .unwrap_or(DEFAULT_MAX_NODES)
                        .saturating_mul(2),
                );
                let extra = build_context(self.store, &escalated, &self.budget_policy)?;
                if extra.nodes.is_empty() && extra.edges.is_empty() {
                    break;
                }
                merge_round_two(&mut result, extra);
                let rounds_used = round + 1;
                report = evaluate_sufficiency(&effective_request, &result);
                report.rounds_used = rounds_used;
                report.escalation_reason = escalation_reason(&report.evidence_gaps)
                    .or_else(|| Some("requested evidence satisfied after round 2".to_owned()));
                if !report.is_unsatisfied() {
                    break;
                }
            }
        }
        result.sufficiency = Some(report);
        slim_context_metadata(&mut result, effective_request.include_ranking_evidence);
        apply_payload_budgets(
            &mut result,
            &self.budget_policy,
            &self.token_counter,
            &self.token_fallback,
        );
        let node_limit = result.request.max_nodes.unwrap_or(DEFAULT_MAX_NODES);
        let edge_limit = result.request.max_edges.unwrap_or(DEFAULT_MAX_EDGES);
        let file_limit = result.request.max_files.unwrap_or(DEFAULT_MAX_FILES);
        let prefix = extraction_budget_prefix(effective_request.intent);
        // Report each budget against the limit that actually governed it. A
        // report is only emitted when the observed count exceeds its limit, so
        // collateral drops (edges removed with a trimmed node) no longer surface
        // as a phantom `max_edges` hit at 18/100.
        let observed_nodes = result.nodes.len() + result.truncation.nodes_dropped;
        let observed_edges = result.edges.len() + result.truncation.edges_dropped;
        let observed_files = result.files.len() + result.truncation.files_dropped;
        budgets.record_usage(
            self.budget_policy.review_context_extraction.nodes,
            format!("{prefix}.max_nodes"),
            node_limit,
            observed_nodes,
            observed_nodes > node_limit,
        );
        budgets.record_usage(
            self.budget_policy.review_context_extraction.edges,
            format!("{prefix}.max_edges"),
            edge_limit,
            observed_edges,
            observed_edges > edge_limit,
        );
        budgets.record_usage(
            self.budget_policy.review_context_extraction.files,
            format!("{prefix}.max_files"),
            file_limit,
            observed_files,
            observed_files > file_limit,
        );
        record_seed_budget_reports(&mut budgets, &result);
        record_payload_budget_reports(&mut budgets, &result, &self.budget_policy);
        result.budget = budgets.summary(
            format!("{prefix}.max_nodes"),
            node_limit,
            result.nodes.len(),
        );
        Ok(result)
    }
}

fn extraction_budget_prefix(intent: atlas_core::model::ContextIntent) -> &'static str {
    match intent {
        atlas_core::model::ContextIntent::Review => "review_context_extraction",
        atlas_core::model::ContextIntent::File => "file_context_extraction",
        atlas_core::model::ContextIntent::Impact
        | atlas_core::model::ContextIntent::ImpactAnalysis
        | atlas_core::model::ContextIntent::RefactorSafety
        | atlas_core::model::ContextIntent::DependencyRemoval => "impact_context_extraction",
        _ => "context_extraction",
    }
}

fn context_intent_str(intent: atlas_core::model::ContextIntent) -> &'static str {
    match intent {
        atlas_core::model::ContextIntent::Symbol => "symbol",
        atlas_core::model::ContextIntent::File => "file",
        atlas_core::model::ContextIntent::Review => "review",
        atlas_core::model::ContextIntent::Impact => "impact",
        atlas_core::model::ContextIntent::ImpactAnalysis => "impact_analysis",
        atlas_core::model::ContextIntent::UsageLookup => "usage_lookup",
        atlas_core::model::ContextIntent::RefactorSafety => "refactor_safety",
        atlas_core::model::ContextIntent::DeadCodeCheck => "dead_code_check",
        atlas_core::model::ContextIntent::RenamePreview => "rename_preview",
        atlas_core::model::ContextIntent::DependencyRemoval => "dependency_removal",
    }
}

fn classify_context_outcome(result: &ContextResult) -> atlas_core::model::ContextOutcome {
    if result.ambiguity.is_some() {
        return atlas_core::model::ContextOutcome::Ambiguous;
    }
    let telemetry = result.telemetry.as_ref();
    let emitted_items = telemetry.map_or(
        result.nodes.len()
            + result.edges.len()
            + result.files.len()
            + result.content_assets.len()
            + result.saved_context_sources.len()
            + result.overviews.len(),
        |telemetry| {
            telemetry.emitted_nodes
                + telemetry.emitted_edges
                + telemetry.emitted_files
                + telemetry.emitted_content_assets
                + telemetry.emitted_saved_sources
                + telemetry.emitted_overviews
        },
    );
    if emitted_items == 0 {
        return atlas_core::model::ContextOutcome::Empty;
    }
    if telemetry.is_some_and(|telemetry| telemetry.budget_hit) {
        return atlas_core::model::ContextOutcome::BudgetExhausted;
    }
    atlas_core::model::ContextOutcome::Ok
}

fn record_seed_budget_reports(budgets: &mut BudgetManager, result: &ContextResult) {
    for seed in &result.seed_budgets {
        if !seed.budget_hit {
            continue;
        }

        let budget_name = match seed.seed_kind.as_str() {
            "changed_files" => "graph_traversal.max_seed_files",
            // File targets seed the symbols they contain; keep the same central
            // seed-node budget name instead of leaking the synthetic kind.
            "changed_symbols" | "file_symbols" => "graph_traversal.max_seed_nodes",
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
