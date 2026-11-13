use super::*;
use atlas_core::model::{
    ContextOutcome, ContextSourceMix, ContextTelemetry, PayloadTruncationMeta, TokenAccountingMeta,
};
use atlas_token_count::{TokenCount, TokenCountMethod, TokenCounter};
use std::cmp::Ordering;

/// Load-time fallback info for the active counter (tokenizer load failed
/// and the heuristic was used instead).
#[derive(Debug, Clone, Default)]
pub(super) struct TokenFallbackInfo {
    pub used: bool,
    pub reason: Option<String>,
}

/// Serialized payload size plus tokenizer-backed token count.
///
/// `method`/`fallback_reason` are part of the measurement contract and are
/// surfaced in JSON output by a later phase; trimming itself only reads
/// `bytes`/`tokens`.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(super) struct PayloadMeasurement {
    /// Serialized payload bytes (payload metadata stripped).
    pub bytes: usize,
    /// Token count for the same serialized payload.
    pub tokens: usize,
    /// Counting method used for `tokens`.
    pub method: TokenCountMethod,
    /// Why a heuristic count was used, when tokenizer counting failed.
    pub fallback_reason: Option<String>,
}

pub(super) fn apply_payload_budgets(
    result: &mut ContextResult,
    policy: &BudgetPolicy,
    counter: &TokenCounter,
    fallback: &TokenFallbackInfo,
) {
    let requested = measure_payload(result, counter);
    let requested_bytes = requested.bytes;
    let initial_nodes = result.nodes.len();
    let initial_edges = result.edges.len();
    let initial_files = result.files.len();
    let initial_sources = result.saved_context_sources.len();
    let initial_content_assets = result.content_assets.len();
    let initial_overviews = result.overviews.len();
    let requested_tokens = requested.tokens;
    let initial_node_drops = result.truncation.nodes_dropped;
    let initial_edge_drops = result.truncation.edges_dropped;
    let initial_file_drops = result.truncation.files_dropped;
    let initial_content_asset_drops = result.truncation.content_assets_dropped;

    // CM13: compute effective token/byte limits, honouring per-request override.
    // The per-request token_budget is capped by the policy ceiling so callers
    // cannot bypass the central policy.
    let policy_token_limit = policy
        .mcp_cli_payload_serialization
        .context_tokens_estimate
        .default_limit;
    let token_ceiling = policy
        .mcp_cli_payload_serialization
        .context_tokens_estimate
        .max_limit;
    let effective_token_limit = match result.request.token_budget {
        Some(requested) => requested.min(token_ceiling).min(policy_token_limit),
        None => policy_token_limit,
    };
    // Byte caps stay independent of token accounting: the policy byte limit
    // is the transport/memory safety cap, while the token budget is enforced
    // via the configured counter (tokenizer or byte heuristic).
    let effective_byte_limit = policy
        .mcp_cli_payload_serialization
        .context_payload_bytes
        .default_limit;

    // Track whether the token budget was overridden by the caller.
    let token_budget_applied =
        if result.request.token_budget.is_some() && effective_token_limit < policy_token_limit {
            Some(effective_token_limit)
        } else {
            None
        };

    trim_file_excerpt_bytes(
        result,
        policy
            .mcp_cli_payload_serialization
            .file_excerpt_bytes
            .default_limit,
    );
    trim_saved_context_bytes(
        result,
        policy
            .mcp_cli_payload_serialization
            .saved_context_bytes
            .default_limit,
    );
    trim_review_source_bytes(
        result,
        policy
            .mcp_cli_payload_serialization
            .review_source_bytes
            .default_limit,
    );
    let emitted =
        trim_context_payload(result, effective_byte_limit, effective_token_limit, counter);
    update_file_node_counts(result);

    let payload_nodes_dropped = initial_nodes.saturating_sub(result.nodes.len());
    let payload_edges_dropped = initial_edges.saturating_sub(result.edges.len());
    let payload_files_dropped = initial_files.saturating_sub(result.files.len());
    let payload_sources_dropped =
        initial_sources.saturating_sub(result.saved_context_sources.len());
    let payload_content_assets_dropped =
        initial_content_assets.saturating_sub(result.content_assets.len());
    let emitted_bytes = emitted.bytes;
    let tokens_estimated = emitted.tokens;
    let omitted_byte_count = requested_bytes.saturating_sub(emitted_bytes);

    if payload_nodes_dropped > 0
        || payload_edges_dropped > 0
        || payload_files_dropped > 0
        || payload_sources_dropped > 0
        || payload_content_assets_dropped > 0
        || omitted_byte_count > 0
        || token_budget_applied.is_some()
    {
        result.truncation.nodes_dropped = initial_node_drops + payload_nodes_dropped;
        result.truncation.edges_dropped = initial_edge_drops + payload_edges_dropped;
        result.truncation.files_dropped = initial_file_drops + payload_files_dropped;
        result.truncation.content_assets_dropped =
            initial_content_asset_drops + payload_content_assets_dropped;
        result.truncation.truncated = payload_nodes_dropped > 0
            || payload_edges_dropped > 0
            || payload_files_dropped > 0
            || payload_sources_dropped > 0
            || payload_content_assets_dropped > 0
            || omitted_byte_count > 0;

        // CM13: compute per-source-kind token breakdown.
        let source_mix = compute_source_mix(
            result,
            &DropCounts {
                nodes: payload_nodes_dropped,
                edges: payload_edges_dropped,
                files: payload_files_dropped,
                sources: payload_sources_dropped,
                content_assets: payload_content_assets_dropped,
            },
            counter,
        );

        result.truncation.payload = Some(PayloadTruncationMeta {
            bytes_requested: requested_bytes,
            bytes_emitted: emitted_bytes,
            tokens_estimated,
            token_budget_applied,
            omitted_node_count: payload_nodes_dropped,
            omitted_file_count: payload_files_dropped,
            omitted_source_count: payload_sources_dropped,
            omitted_byte_count,
            continuation_hint: if omitted_byte_count > 0
                || payload_nodes_dropped > 0
                || payload_files_dropped > 0
            {
                Some(continuation_hint(result, payload_sources_dropped))
            } else {
                None
            },
            source_mix,
            token_accounting: Some(build_token_accounting_meta(&emitted, fallback)),
        });
    }

    // R7.1: always capture telemetry so the engine boundary can record metrics;
    // `ContextEngine::build` drops it again unless the request opted in, keeping
    // default output byte-identical.
    let budget_hit = result.truncation.truncated;
    let budget_name = context_budget_name(
        result,
        requested_tokens,
        effective_token_limit,
        requested_bytes,
        effective_byte_limit,
    );
    result.telemetry = Some(ContextTelemetry {
        intent: result.request.intent,
        requested_nodes: initial_nodes,
        emitted_nodes: result.nodes.len(),
        requested_edges: initial_edges,
        emitted_edges: result.edges.len(),
        requested_files: initial_files,
        emitted_files: result.files.len(),
        requested_saved_sources: initial_sources,
        emitted_saved_sources: result.saved_context_sources.len(),
        requested_content_assets: initial_content_assets,
        emitted_content_assets: result.content_assets.len(),
        requested_overviews: initial_overviews,
        emitted_overviews: result.overviews.len(),
        requested_bytes,
        emitted_bytes,
        requested_tokens,
        emitted_tokens: tokens_estimated,
        budget_hit,
        budget_name,
        duration_ms: 0,
        outcome: ContextOutcome::Ok,
    });
}

fn context_budget_name(
    result: &ContextResult,
    requested_tokens: usize,
    token_limit: usize,
    requested_bytes: usize,
    byte_limit: usize,
) -> Option<String> {
    if result
        .truncation
        .payload
        .as_ref()
        .and_then(|payload| payload.token_budget_applied)
        .is_some()
        || requested_tokens > token_limit
    {
        return Some("mcp_cli_payload_serialization.max_context_tokens_estimate".to_owned());
    }
    if requested_bytes > byte_limit {
        return Some("mcp_cli_payload_serialization.max_context_payload_bytes".to_owned());
    }
    if result.truncation.nodes_dropped > 0 {
        return Some(format!("{}.max_nodes", context_budget_prefix(result)));
    }
    if result.truncation.edges_dropped > 0 {
        return Some(format!("{}.max_edges", context_budget_prefix(result)));
    }
    if result.truncation.files_dropped > 0 {
        return Some(format!("{}.max_files", context_budget_prefix(result)));
    }
    if result.truncation.content_assets_dropped > 0 {
        return Some("content_saved_context_lookup.max_content_assets".to_owned());
    }
    None
}

/// Budget names follow the active intent so a symbol/file request does not
/// report `review_context_extraction.*` for a non-review budget.
fn context_budget_prefix(result: &ContextResult) -> &'static str {
    use atlas_core::model::ContextIntent;
    match result.request.intent {
        ContextIntent::Review => "review_context_extraction",
        ContextIntent::File => "file_context_extraction",
        ContextIntent::Impact
        | ContextIntent::ImpactAnalysis
        | ContextIntent::RefactorSafety
        | ContextIntent::DependencyRemoval => "impact_context_extraction",
        _ => "context_extraction",
    }
}

/// Map a payload measurement plus load-time fallback info into JSON metadata.
fn build_token_accounting_meta(
    emitted: &PayloadMeasurement,
    fallback: &TokenFallbackInfo,
) -> TokenAccountingMeta {
    let (provider, model, bytes_per_token) = match &emitted.method {
        TokenCountMethod::Tokenizer { provider, model } => (provider.clone(), model.clone(), None),
        TokenCountMethod::HeuristicBytes { bytes_per_token } => {
            ("heuristic".to_owned(), None, Some(*bytes_per_token))
        }
    };
    let fallback_used = fallback.used || emitted.fallback_reason.is_some();
    let fallback_reason = fallback
        .reason
        .clone()
        .or_else(|| emitted.fallback_reason.clone());
    TokenAccountingMeta {
        provider,
        model,
        fallback_used,
        fallback_reason,
        bytes_per_token,
    }
}

/// Serialize the payload without payload metadata and measure bytes and
/// tokens. Tokenizer failures never panic: they degrade to the byte
/// heuristic with a recorded reason.
fn measure_payload(result: &ContextResult, counter: &TokenCounter) -> PayloadMeasurement {
    let mut clone = result.clone();
    clone.truncation.payload = None;
    let Some(bytes) = serde_json::to_vec(&clone).ok() else {
        return PayloadMeasurement {
            bytes: 0,
            tokens: 0,
            method: TokenCountMethod::HeuristicBytes { bytes_per_token: 4 },
            fallback_reason: Some("payload serialization failed".to_owned()),
        };
    };
    let count = counter
        .count_json_bytes(&bytes)
        .unwrap_or_else(|_| TokenCount {
            tokens: bytes.len().div_ceil(4),
            method: TokenCountMethod::HeuristicBytes { bytes_per_token: 4 },
            fallback_reason: Some("tokenizer count failed".to_owned()),
        });
    PayloadMeasurement {
        bytes: bytes.len(),
        tokens: count.tokens,
        method: count.method,
        fallback_reason: count.fallback_reason,
    }
}

struct DropCounts {
    nodes: usize,
    edges: usize,
    files: usize,
    sources: usize,
    content_assets: usize,
}

/// Build a compact per-source-kind token usage breakdown.
///
/// Source priority (highest first): graph_context > saved_artifacts.
/// Resume snapshot is not a separate source type in the current model; saved
/// artifacts produced by resume are included in `saved_artifacts`.
///
/// Sections are counted with the configured counter. Heuristic fallback is
/// only used when the counter itself is heuristic; tokenizer count errors
/// yield zero tokens rather than silently switching methods.
fn compute_source_mix(
    result: &ContextResult,
    dropped: &DropCounts,
    counter: &TokenCounter,
) -> Vec<ContextSourceMix> {
    let dropped_nodes = dropped.nodes;
    let dropped_edges = dropped.edges;
    let dropped_files = dropped.files;
    let dropped_sources = dropped.sources;
    let dropped_content_assets = dropped.content_assets;

    // Serialize each source-kind section once and count it with the counter.
    let graph_node_bytes = serde_json::to_vec(&result.nodes).unwrap_or_default();
    let graph_edge_bytes = serde_json::to_vec(&result.edges).unwrap_or_default();
    let graph_file_bytes = serde_json::to_vec(&result.files).unwrap_or_default();
    let saved_bytes = serde_json::to_vec(&result.saved_context_sources).unwrap_or_default();
    let content_asset_bytes = serde_json::to_vec(&result.content_assets).unwrap_or_default();
    let graph_tokens = count_section_tokens(&graph_node_bytes, counter)
        + count_section_tokens(&graph_edge_bytes, counter)
        + count_section_tokens(&graph_file_bytes, counter);
    let saved_tokens = count_section_tokens(&saved_bytes, counter);
    let content_asset_tokens = count_section_tokens(&content_asset_bytes, counter);
    if graph_tokens + saved_tokens + content_asset_tokens == 0 {
        return Vec::new();
    }

    let mut mix = Vec::new();

    // graph_context: nodes + edges + files (highest priority surface)
    let graph_included = result.nodes.len() + result.edges.len() + result.files.len();
    let graph_dropped = dropped_nodes + dropped_edges + dropped_files;
    if graph_included > 0 || graph_dropped > 0 {
        mix.push(ContextSourceMix {
            source_kind: "graph_context".to_owned(),
            items_included: graph_included,
            items_dropped: graph_dropped,
            tokens_used: graph_tokens,
        });
    }

    // content_assets: non-code file assets (docs, config, templates, SQL)
    // Priority: between graph_context and saved_artifacts.
    let ca_included = result.content_assets.len();
    if ca_included > 0 || dropped_content_assets > 0 {
        mix.push(ContextSourceMix {
            source_kind: "content_assets".to_owned(),
            items_included: ca_included,
            items_dropped: dropped_content_assets,
            tokens_used: content_asset_tokens,
        });
    }

    // saved_artifacts: prior Atlas session outputs (lowest priority)
    let saved_included = result.saved_context_sources.len();
    if saved_included > 0 || dropped_sources > 0 {
        mix.push(ContextSourceMix {
            source_kind: "saved_artifacts".to_owned(),
            items_included: saved_included,
            items_dropped: dropped_sources,
            tokens_used: saved_tokens,
        });
    }

    // Suppress empty mix to keep output compact.
    if mix
        .iter()
        .all(|m| m.items_included == 0 && m.items_dropped == 0)
    {
        return Vec::new();
    }

    mix
}

/// Count tokens for one serialized JSON section. Tokenizer count failures
/// yield zero rather than falling back to a different method.
fn count_section_tokens(bytes: &[u8], counter: &TokenCounter) -> usize {
    counter
        .count_json_bytes(bytes)
        .map(|count| count.tokens)
        .unwrap_or(0)
}

fn continuation_hint(result: &ContextResult, omitted_sources: usize) -> String {
    if omitted_sources > 0 {
        return "reduce saved-context scope or retrieve full artifacts by source_id".to_owned();
    }
    if result.request.include_code_spans {
        return "narrow query, lower depth, or disable code_spans for larger payloads".to_owned();
    }
    "narrow query, lower depth, or split changed-file set".to_owned()
}

fn files_bytes(result: &ContextResult) -> usize {
    serde_json::to_vec(&result.files)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn file_excerpt_bytes(result: &ContextResult) -> usize {
    let excerpts: Vec<Vec<(u32, u32)>> = result
        .files
        .iter()
        .map(|file| file.line_ranges.clone())
        .collect();
    serde_json::to_vec(&excerpts)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn saved_context_bytes(result: &ContextResult) -> usize {
    serde_json::to_vec(&result.saved_context_sources)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}

fn trim_file_excerpt_bytes(result: &mut ContextResult, limit: usize) {
    while file_excerpt_bytes(result) > limit {
        let Some(index) = select_excerpt_drop_index(result) else {
            break;
        };
        if result.files[index].line_ranges.is_empty() {
            break;
        }
        result.files[index].line_ranges.clear();
    }
}

fn trim_saved_context_bytes(result: &mut ContextResult, limit: usize) {
    while saved_context_bytes(result) > limit {
        let Some(index) = select_saved_source_drop_index(result) else {
            break;
        };
        result.saved_context_sources.remove(index);
    }
}

fn trim_review_source_bytes(result: &mut ContextResult, limit: usize) {
    while files_bytes(result) > limit {
        let Some(index) = select_file_drop_index(result) else {
            break;
        };
        result.files.remove(index);
    }
}

/// Trim payload items until both byte and token limits hold. Returns the
/// final measurement of the trimmed payload.
fn trim_context_payload(
    result: &mut ContextResult,
    byte_limit: usize,
    token_limit: usize,
    counter: &TokenCounter,
) -> PayloadMeasurement {
    let mut set_aside = Vec::new();
    let measurement = loop {
        let measurement = measure_payload(result, counter);
        if fits_payload_budget(&measurement, byte_limit, token_limit) {
            break measurement;
        }
        if !trim_one_payload_unit_recording(result, &mut set_aside) {
            // Only irreducible units (the seed node) remain. Degrade optional
            // enrichment sections so the caller budget still holds instead of
            // silently returning an oversized payload.
            return trim_floor_sections(result, byte_limit, token_limit, counter);
        }
    };

    if set_aside.is_empty() {
        return measurement;
    }
    refill_payload(result, set_aside, byte_limit, token_limit, counter)
}

fn fits_payload_budget(
    measurement: &PayloadMeasurement,
    byte_limit: usize,
    token_limit: usize,
) -> bool {
    measurement.bytes <= byte_limit && measurement.tokens <= token_limit
}

/// Degrade optional enrichment sections when the payload cannot fit the budget
/// even after every removable graph unit is gone.
///
/// Order is lowest-value first so the seed node, its edges, and the truncation
/// accounting survive longest. Each step re-measures and stops as soon as the
/// budget holds.
fn trim_floor_sections(
    result: &mut ContextResult,
    byte_limit: usize,
    token_limit: usize,
    counter: &TokenCounter,
) -> PayloadMeasurement {
    let mut measurement = measure_payload(result, counter);
    macro_rules! degrade {
        ($apply:expr) => {{
            $apply;
            measurement = measure_payload(result, counter);
            if fits_payload_budget(&measurement, byte_limit, token_limit) {
                return measurement;
            }
        }};
    }

    if result.telemetry.is_some() {
        degrade!(result.telemetry = None);
    }
    if !result.seed_budgets.is_empty() {
        degrade!(result.seed_budgets.clear());
    }
    if result.traversal_budget.is_some() {
        degrade!(result.traversal_budget = None);
    }
    if result.repo_provenance.is_some() {
        degrade!(result.repo_provenance = None);
    }
    // `sufficiency` and `ambiguity` are contract-bearing (VR3 escalation and
    // outcome classification), so they are never degraded here.
    if result.workflow.is_some() {
        degrade!(result.workflow = None);
    }

    measurement
}

/// A graph payload unit removed during trimming, retained so a later refill
/// can spend leftover token/byte headroom instead of discarding useful
/// callers, callees, or edges.
enum SetAsideUnit {
    Node(SelectedNode),
    Edge(SelectedEdge),
    File(SelectedFile),
}

/// Spend any remaining budget headroom on the units trimmed last.
///
/// The trim loop drops whole units in ascending priority order, so headroom
/// below one unit's cost used to be left unused. Refilling in reverse trim
/// order restores the highest-priority unit that still fits.
fn refill_payload(
    result: &mut ContextResult,
    set_aside: Vec<SetAsideUnit>,
    byte_limit: usize,
    token_limit: usize,
    counter: &TokenCounter,
) -> PayloadMeasurement {
    let mut accepted = measure_payload(result, counter);
    for unit in set_aside.into_iter().rev() {
        insert_set_aside_unit(result, &unit);
        sort_payload_units(result);
        let candidate = measure_payload(result, counter);
        if fits_payload_budget(&candidate, byte_limit, token_limit) {
            accepted = candidate;
        } else {
            remove_set_aside_unit(result, &unit);
        }
    }
    accepted
}

fn insert_set_aside_unit(result: &mut ContextResult, unit: &SetAsideUnit) {
    match unit {
        SetAsideUnit::Node(node) => result.nodes.push(node.clone()),
        SetAsideUnit::Edge(edge) => result.edges.push(edge.clone()),
        SetAsideUnit::File(file) => result.files.push(file.clone()),
    }
}

fn remove_set_aside_unit(result: &mut ContextResult, unit: &SetAsideUnit) {
    match unit {
        SetAsideUnit::Node(node) => {
            let qname = &node.node.qualified_name;
            result
                .nodes
                .retain(|item| item.node.qualified_name != *qname);
        }
        SetAsideUnit::Edge(edge) => {
            let edge = &edge.edge;
            result.edges.retain(|item| {
                item.edge.source_qn != edge.source_qn
                    || item.edge.target_qn != edge.target_qn
                    || item.edge.kind != edge.kind
            });
        }
        SetAsideUnit::File(file) => {
            let path = &file.path;
            result.files.retain(|item| item.path != *path);
        }
    }
}

/// Restore deterministic rank ordering after a refill inserts a unit.
fn sort_payload_units(result: &mut ContextResult) {
    use crate::ranking::{compare_edge_scores, compare_file_priorities, compare_node_scores};

    result.nodes.sort_by(|left, right| {
        compare_node_scores(
            left.relevance_score as f64,
            right.relevance_score as f64,
            &left.node.qualified_name,
            &right.node.qualified_name,
        )
    });
    result.edges.sort_by(|left, right| {
        compare_edge_scores(
            left.relevance_score as f64,
            right.relevance_score as f64,
            &left.edge.source_qn,
            &right.edge.source_qn,
            &left.edge.target_qn,
            &right.edge.target_qn,
        )
    });
    let ranking = crate::ranking::ContextRankingPrimitives::default();
    result.files.sort_by(|left, right| {
        compare_file_priorities(
            ranking.selection_priority(left.selection_reason),
            ranking.selection_priority(right.selection_reason),
            &left.path,
            &right.path,
        )
    });
}

/// Drop exactly one payload unit in deterministic priority order; returns
/// false when nothing removable remains.
#[cfg(test)]
pub(super) fn trim_one_payload_unit(result: &mut ContextResult) -> bool {
    trim_one_payload_unit_recording(result, &mut Vec::new())
}

/// Same as [`trim_one_payload_unit`], but retains graph units so the refill
/// pass can restore anything that still fits in the remaining headroom.
fn trim_one_payload_unit_recording(
    result: &mut ContextResult,
    set_aside: &mut Vec<SetAsideUnit>,
) -> bool {
    if let Some(index) = select_saved_source_drop_index(result) {
        result.saved_context_sources.remove(index);
        return true;
    }
    // ICM-G7: overview previews are supporting context; drop them before
    // graph nodes when the payload budget is exceeded.
    if !result.overviews.is_empty() {
        result.overviews.pop();
        return true;
    }
    if trim_workflow(result) {
        return true;
    }
    if trim_ambiguity(result) {
        return true;
    }
    if let Some(index) = select_file_drop_index(result) {
        set_aside.push(SetAsideUnit::File(result.files.remove(index)));
        return true;
    }
    if let Some(index) = select_edge_drop_index(result) {
        set_aside.push(SetAsideUnit::Edge(result.edges.remove(index)));
        return true;
    }
    if let Some(index) = select_node_drop_index(result) {
        let removed = result.nodes.remove(index);
        let removed_qn = removed.node.qualified_name.clone();
        result
            .edges
            .retain(|edge| edge.edge.source_qn != removed_qn && edge.edge.target_qn != removed_qn);
        set_aside.push(SetAsideUnit::Node(removed));
        return true;
    }
    false
}

fn trim_workflow(result: &mut ContextResult) -> bool {
    let Some(workflow) = result.workflow.as_mut() else {
        return false;
    };
    if !workflow.call_chains.is_empty() {
        workflow.call_chains.pop();
        return true;
    }
    if !workflow.ripple_effects.is_empty() {
        workflow.ripple_effects.pop();
        return true;
    }
    if !workflow.impacted_components.is_empty() {
        workflow.impacted_components.pop();
        return true;
    }
    if workflow.high_impact_nodes.len() > 1 {
        workflow.high_impact_nodes.pop();
        return true;
    }
    false
}

fn trim_ambiguity(result: &mut ContextResult) -> bool {
    let Some(ambiguity) = result.ambiguity.as_mut() else {
        return false;
    };
    if ambiguity.candidates.len() > 1 {
        ambiguity.candidates.pop();
        return true;
    }
    false
}

fn select_excerpt_drop_index(result: &ContextResult) -> Option<usize> {
    select_best_drop(
        result
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| !file.line_ranges.is_empty()),
        |(index, file)| DropCandidate {
            index,
            keep_priority: file_keep_priority(file.selection_reason),
            score: file.node_count_included as f32,
            key_a: file.path.clone(),
            key_b: String::new(),
        },
    )
}

fn select_saved_source_drop_index(result: &ContextResult) -> Option<usize> {
    select_best_drop(
        result.saved_context_sources.iter().enumerate(),
        |(index, source)| DropCandidate {
            index,
            keep_priority: 0,
            score: source.relevance_score,
            key_a: source.source_id.clone(),
            key_b: String::new(),
        },
    )
}

fn select_file_drop_index(result: &ContextResult) -> Option<usize> {
    let has_non_direct = result
        .files
        .iter()
        .any(|file| file.selection_reason != SelectionReason::DirectTarget);
    select_best_drop(
        result.files.iter().enumerate().filter(|(_, file)| {
            !has_non_direct || file.selection_reason != SelectionReason::DirectTarget
        }),
        |(index, file)| DropCandidate {
            index,
            keep_priority: file_keep_priority(file.selection_reason),
            score: file.node_count_included as f32,
            key_a: file.path.clone(),
            key_b: String::new(),
        },
    )
}

fn select_edge_drop_index(result: &ContextResult) -> Option<usize> {
    select_best_drop(result.edges.iter().enumerate(), |(index, edge)| {
        DropCandidate {
            index,
            keep_priority: edge_keep_priority(edge.selection_reason),
            score: edge.relevance_score,
            key_a: edge.edge.source_qn.clone(),
            key_b: edge.edge.target_qn.clone(),
        }
    })
}

fn select_node_drop_index(result: &ContextResult) -> Option<usize> {
    if result.nodes.len() <= 1 {
        return None;
    }
    let has_non_direct = result
        .nodes
        .iter()
        .any(|node| node.selection_reason != SelectionReason::DirectTarget);
    select_best_drop(
        result.nodes.iter().enumerate().filter(|(_, node)| {
            !has_non_direct || node.selection_reason != SelectionReason::DirectTarget
        }),
        |(index, node)| DropCandidate {
            index,
            keep_priority: node_keep_priority(node),
            score: node.relevance_score,
            key_a: node.node.qualified_name.clone(),
            key_b: String::new(),
        },
    )
}

fn select_best_drop<T, F>(iter: impl Iterator<Item = (usize, T)>, map: F) -> Option<usize>
where
    F: Fn((usize, T)) -> DropCandidate,
{
    iter.map(map)
        .min_by(compare_drop_candidates)
        .map(|candidate| candidate.index)
}

fn compare_drop_candidates(left: &DropCandidate, right: &DropCandidate) -> Ordering {
    left.keep_priority
        .cmp(&right.keep_priority)
        .then_with(|| {
            left.score
                .partial_cmp(&right.score)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| left.key_a.cmp(&right.key_a))
        .then_with(|| left.key_b.cmp(&right.key_b))
}

struct DropCandidate {
    index: usize,
    keep_priority: u8,
    score: f32,
    key_a: String,
    key_b: String,
}

fn file_keep_priority(reason: SelectionReason) -> u8 {
    match reason {
        SelectionReason::DirectTarget => 7,
        // Preserve direct call relationships over looser adjacency under a tight
        // payload budget; otherwise callers are dropped before test/impact rows.
        SelectionReason::Caller | SelectionReason::Callee => 6,
        SelectionReason::TestAdjacent => 5,
        SelectionReason::ImpactNeighbor => 4,
        SelectionReason::Importer | SelectionReason::Importee => 2,
        SelectionReason::ContainmentSibling => 1,
        SelectionReason::OverviewLevel => 4,
        SelectionReason::ExperienceShortcut => 4,
    }
}

fn edge_keep_priority(reason: SelectionReason) -> u8 {
    file_keep_priority(reason)
}

fn node_keep_priority(node: &SelectedNode) -> u8 {
    if node.node.is_test {
        return 5;
    }
    file_keep_priority(node.selection_reason)
}
