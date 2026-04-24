use super::*;
use atlas_core::model::{ContextSourceMix, PayloadTruncationMeta};
use std::cmp::Ordering;

pub(super) fn apply_payload_budgets(result: &mut ContextResult, policy: &BudgetPolicy) {
    let requested_bytes = context_bytes(result);
    let initial_nodes = result.nodes.len();
    let initial_edges = result.edges.len();
    let initial_files = result.files.len();
    let initial_sources = result.saved_context_sources.len();
    let initial_node_drops = result.truncation.nodes_dropped;
    let initial_edge_drops = result.truncation.edges_dropped;
    let initial_file_drops = result.truncation.files_dropped;

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
    // When a per-request token budget is tighter than the byte cap, derive an
    // equivalent byte limit (4 bytes/token approximation).
    let policy_byte_limit = policy
        .mcp_cli_payload_serialization
        .context_payload_bytes
        .default_limit;
    let effective_byte_limit = (effective_token_limit * 4).min(policy_byte_limit);

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
    trim_context_payload(result, effective_byte_limit, effective_token_limit);
    update_file_node_counts(result);

    let payload_nodes_dropped = initial_nodes.saturating_sub(result.nodes.len());
    let payload_edges_dropped = initial_edges.saturating_sub(result.edges.len());
    let payload_files_dropped = initial_files.saturating_sub(result.files.len());
    let payload_sources_dropped =
        initial_sources.saturating_sub(result.saved_context_sources.len());
    let emitted_bytes = context_bytes(result);
    let tokens_estimated = estimate_tokens(emitted_bytes);
    let omitted_byte_count = requested_bytes.saturating_sub(emitted_bytes);

    if payload_nodes_dropped > 0
        || payload_edges_dropped > 0
        || payload_files_dropped > 0
        || payload_sources_dropped > 0
        || omitted_byte_count > 0
        || token_budget_applied.is_some()
    {
        result.truncation.nodes_dropped = initial_node_drops + payload_nodes_dropped;
        result.truncation.edges_dropped = initial_edge_drops + payload_edges_dropped;
        result.truncation.files_dropped = initial_file_drops + payload_files_dropped;
        result.truncation.truncated = payload_nodes_dropped > 0
            || payload_edges_dropped > 0
            || payload_files_dropped > 0
            || payload_sources_dropped > 0
            || omitted_byte_count > 0;

        // CM13: compute per-source-kind token breakdown.
        let source_mix = compute_source_mix(
            result,
            &DropCounts {
                nodes: payload_nodes_dropped,
                edges: payload_edges_dropped,
                files: payload_files_dropped,
                sources: payload_sources_dropped,
            },
            emitted_bytes,
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
        });
    }
}

struct DropCounts {
    nodes: usize,
    edges: usize,
    files: usize,
    sources: usize,
}

/// Build a compact per-source-kind token usage breakdown.
///
/// Source priority (highest first): graph_context > saved_artifacts.
/// Resume snapshot is not a separate source type in the current model; saved
/// artifacts produced by resume are included in `saved_artifacts`.
fn compute_source_mix(
    result: &ContextResult,
    dropped: &DropCounts,
    emitted_bytes: usize,
) -> Vec<ContextSourceMix> {
    let dropped_nodes = dropped.nodes;
    let dropped_edges = dropped.edges;
    let dropped_files = dropped.files;
    let dropped_sources = dropped.sources;
    let total_tokens = estimate_tokens(emitted_bytes);
    if total_tokens == 0 {
        return Vec::new();
    }

    // Estimate how many bytes each source kind occupies in the result.
    let graph_node_bytes = serde_json::to_vec(&result.nodes)
        .map(|b| b.len())
        .unwrap_or(0);
    let graph_edge_bytes = serde_json::to_vec(&result.edges)
        .map(|b| b.len())
        .unwrap_or(0);
    let graph_file_bytes = serde_json::to_vec(&result.files)
        .map(|b| b.len())
        .unwrap_or(0);
    let graph_bytes = graph_node_bytes + graph_edge_bytes + graph_file_bytes;
    let saved_bytes = serde_json::to_vec(&result.saved_context_sources)
        .map(|b| b.len())
        .unwrap_or(0);

    let mut mix = Vec::new();

    // graph_context: nodes + edges + files
    let graph_included = result.nodes.len() + result.edges.len() + result.files.len();
    let graph_dropped = dropped_nodes + dropped_edges + dropped_files;
    if graph_included > 0 || graph_dropped > 0 {
        mix.push(ContextSourceMix {
            source_kind: "graph_context".to_owned(),
            items_included: graph_included,
            items_dropped: graph_dropped,
            tokens_used: estimate_tokens(graph_bytes),
        });
    }

    // saved_artifacts: saved context sources
    let saved_included = result.saved_context_sources.len();
    if saved_included > 0 || dropped_sources > 0 {
        mix.push(ContextSourceMix {
            source_kind: "saved_artifacts".to_owned(),
            items_included: saved_included,
            items_dropped: dropped_sources,
            tokens_used: estimate_tokens(saved_bytes),
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

fn continuation_hint(result: &ContextResult, omitted_sources: usize) -> String {
    if omitted_sources > 0 {
        return "reduce saved-context scope or retrieve full artifacts by source_id".to_owned();
    }
    if result.request.include_code_spans {
        return "narrow query, lower depth, or disable code_spans for larger payloads".to_owned();
    }
    "narrow query, lower depth, or split changed-file set".to_owned()
}

fn context_bytes(result: &ContextResult) -> usize {
    let mut clone = result.clone();
    clone.truncation.payload = None;
    serde_json::to_vec(&clone)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
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

fn estimate_tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
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

fn trim_context_payload(result: &mut ContextResult, byte_limit: usize, token_limit: usize) {
    loop {
        let current_bytes = context_bytes(result);
        if current_bytes <= byte_limit && estimate_tokens(current_bytes) <= token_limit {
            break;
        }

        let changed = trim_one_payload_unit(result);
        if !changed {
            break;
        }
    }
}

fn trim_one_payload_unit(result: &mut ContextResult) -> bool {
    if let Some(index) = select_saved_source_drop_index(result) {
        result.saved_context_sources.remove(index);
        return true;
    }
    if trim_workflow(result) {
        return true;
    }
    if trim_ambiguity(result) {
        return true;
    }
    if let Some(index) = select_file_drop_index(result) {
        result.files.remove(index);
        return true;
    }
    if let Some(index) = select_edge_drop_index(result) {
        result.edges.remove(index);
        return true;
    }
    if let Some(index) = select_node_drop_index(result) {
        let removed_qn = result.nodes.remove(index).node.qualified_name;
        result
            .edges
            .retain(|edge| edge.edge.source_qn != removed_qn && edge.edge.target_qn != removed_qn);
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
        SelectionReason::DirectTarget => 6,
        SelectionReason::TestAdjacent => 5,
        SelectionReason::ImpactNeighbor => 4,
        SelectionReason::Caller | SelectionReason::Callee => 3,
        SelectionReason::Importer | SelectionReason::Importee => 2,
        SelectionReason::ContainmentSibling => 1,
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
