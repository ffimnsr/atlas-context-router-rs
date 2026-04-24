//! Agent-optimized output packaging.
//!
//! Full `Node` and `Edge` structs contain many fields that are redundant or
//! irrelevant in an agent-facing context.  The compact types here strip those
//! fields to reduce token overhead while keeping the information an agent
//! actually needs.

use atlas_core::BudgetReport;
use atlas_core::model::{
    ContextResult, ContextSourceMix, Edge, ImpactResult, Node, PayloadTruncationMeta,
    SavedContextSource, SeedBudgetMeta, SelectedEdge, SelectedNode, TraversalBudgetMeta,
};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::output::{OutputFormat, render_value};

// ---------------------------------------------------------------------------
// Compact node
// ---------------------------------------------------------------------------

/// Compact node representation: qualified name, kind, location, language.
#[derive(Serialize)]
pub struct CompactNode<'a> {
    pub qn: &'a str,
    pub kind: &'a str,
    pub file: &'a str,
    pub line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig: Option<&'a str>,
    pub lang: &'a str,
}

pub fn compact_node(n: &Node) -> CompactNode<'_> {
    CompactNode {
        qn: &n.qualified_name,
        kind: n.kind.as_str(),
        file: &n.file_path,
        line: n.line_start,
        parent: n.parent_name.as_deref(),
        sig: n.params.as_deref(),
        lang: &n.language,
    }
}

// ---------------------------------------------------------------------------
// Compact edge
// ---------------------------------------------------------------------------

/// Compact edge: just the relationship triple.
#[derive(Serialize)]
pub struct CompactEdge<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub kind: &'a str,
}

pub fn compact_edge(e: &Edge) -> CompactEdge<'_> {
    CompactEdge {
        from: &e.source_qn,
        to: &e.target_qn,
        kind: e.kind.as_str(),
    }
}

// ---------------------------------------------------------------------------
// Packaged impact output
// ---------------------------------------------------------------------------

/// Agent-ready impact result with capped counts and compact nodes/edges.
///
/// `changed_nodes` and `impacted_nodes` are capped at `MAX_NODES`.
/// `relevant_edges` are capped at `MAX_EDGES`.
const MAX_NODES: usize = 100;
const MAX_EDGES: usize = 100;

#[derive(Serialize)]
pub struct PackagedImpact<'a> {
    pub changed_file_count: usize,
    pub changed_node_count: usize,
    pub changed_nodes: Vec<CompactNode<'a>>,
    pub impacted_node_count: usize,
    pub impacted_nodes: Vec<CompactNode<'a>>,
    pub impacted_file_count: usize,
    pub impacted_files: &'a [String],
    pub relevant_edge_count: usize,
    pub relevant_edges: Vec<CompactEdge<'a>>,
    /// True when any list was capped.
    pub truncated: bool,
    #[serde(skip_serializing_if = "<[SeedBudgetMeta]>::is_empty")]
    pub seed_budgets: &'a [SeedBudgetMeta],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traversal_budget: Option<&'a TraversalBudgetMeta>,
    #[serde(flatten)]
    pub budget: BudgetReport,
}

pub fn package_impact<'a>(
    result: &'a ImpactResult,
    seed_files: &'a [String],
) -> PackagedImpact<'a> {
    let cn_total = result.changed_nodes.len();
    let inp_total = result.impacted_nodes.len();
    let edge_total = result.relevant_edges.len();

    let cn_capped = cn_total.min(MAX_NODES);
    let inp_capped = inp_total.min(MAX_NODES);
    let edge_capped = edge_total.min(MAX_EDGES);

    PackagedImpact {
        changed_file_count: seed_files.len(),
        changed_node_count: cn_total,
        changed_nodes: result.changed_nodes[..cn_capped]
            .iter()
            .map(compact_node)
            .collect(),
        impacted_node_count: inp_total,
        impacted_nodes: result.impacted_nodes[..inp_capped]
            .iter()
            .map(compact_node)
            .collect(),
        impacted_file_count: result.impacted_files.len(),
        impacted_files: &result.impacted_files,
        relevant_edge_count: edge_total,
        relevant_edges: result.relevant_edges[..edge_capped]
            .iter()
            .map(compact_edge)
            .collect(),
        truncated: cn_capped < cn_total || inp_capped < inp_total || edge_capped < edge_total,
        seed_budgets: &result.seed_budgets,
        traversal_budget: result.traversal_budget.as_ref(),
        budget: result.budget.clone(),
    }
}

// ---------------------------------------------------------------------------
// Compact ContextResult packaging (Slice 9 — thin MCP adapter)
// ---------------------------------------------------------------------------

/// Agent-optimized compact representation of a [`ContextResult`].
#[derive(Serialize)]
pub struct PackagedContextResult<'a> {
    pub intent: &'a str,
    pub node_count: usize,
    pub nodes: Vec<PackagedSelectedNode<'a>>,
    pub edge_count: usize,
    pub edges: Vec<PackagedSelectedEdge<'a>>,
    pub file_count: usize,
    pub files: Vec<PackagedSelectedFile<'a>>,
    pub truncated: bool,
    pub nodes_dropped: usize,
    pub edges_dropped: usize,
    pub files_dropped: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambiguity_query: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ambiguity_candidates: Vec<&'a str>,
    /// Saved-context sources from the content store (CM6).
    /// Only present when `include_saved_context` was `true` in the request.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub saved_context_sources: Vec<PackagedSavedSource<'a>>,
    #[serde(skip_serializing_if = "<[SeedBudgetMeta]>::is_empty")]
    pub seed_budgets: &'a [SeedBudgetMeta],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traversal_budget: Option<&'a TraversalBudgetMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_truncation: Option<&'a PayloadTruncationMeta>,
    /// Per-source-kind token usage breakdown (CM13).
    /// Only present when payload trimming ran (i.e. when `payload_truncation` is set).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_mix: Vec<&'a ContextSourceMix>,
    /// Effective token budget applied for this result (CM13).
    /// Only present when a per-request token_budget was set and enforced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_budget_applied: Option<usize>,
    #[serde(flatten)]
    pub budget: BudgetReport,
}

/// Compact representation of a [`SavedContextSource`].
#[derive(Serialize)]
pub struct PackagedSavedSource<'a> {
    pub source_id: &'a str,
    pub label: &'a str,
    pub source_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<&'a str>,
    pub preview: &'a str,
    pub retrieval_hint: &'a str,
    pub relevance_score: f32,
}

#[derive(Serialize)]
pub struct PackagedSelectedNode<'a> {
    pub reason: &'a str,
    pub distance: u32,
    #[serde(flatten)]
    pub node: CompactNode<'a>,
}

#[derive(Serialize)]
pub struct PackagedSelectedEdge<'a> {
    pub reason: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub kind: &'a str,
}

#[derive(Serialize)]
pub struct PackagedSelectedFile<'a> {
    pub path: &'a str,
    pub reason: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub line_ranges: Vec<(u32, u32)>,
}

/// Package a [`ContextResult`] into an agent-optimized compact form.
pub fn package_context_result(result: &ContextResult) -> PackagedContextResult<'_> {
    let intent_str = match result.request.intent {
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
    };

    let all_nodes = &result.nodes;
    let all_edges = &result.edges;
    let node_count = all_nodes.len();
    let edge_count = all_edges.len();

    let node_cap = node_count.min(MAX_NODES);
    let edge_cap = edge_count.min(MAX_EDGES);

    let nodes: Vec<PackagedSelectedNode<'_>> = all_nodes[..node_cap]
        .iter()
        .map(|sn: &SelectedNode| PackagedSelectedNode {
            reason: sn.selection_reason.as_str(),
            distance: sn.distance,
            node: compact_node(&sn.node),
        })
        .collect();

    let edges: Vec<PackagedSelectedEdge<'_>> = all_edges[..edge_cap]
        .iter()
        .map(|se: &SelectedEdge| PackagedSelectedEdge {
            reason: se.selection_reason.as_str(),
            from: &se.edge.source_qn,
            to: &se.edge.target_qn,
            kind: se.edge.kind.as_str(),
        })
        .collect();

    let files: Vec<PackagedSelectedFile<'_>> = result
        .files
        .iter()
        .map(|sf| PackagedSelectedFile {
            path: &sf.path,
            reason: sf.selection_reason.as_str(),
            line_ranges: sf.line_ranges.clone(),
        })
        .collect();

    let (ambiguity_query, ambiguity_candidates) = if let Some(amb) = &result.ambiguity {
        (
            Some(amb.query.as_str()),
            amb.candidates.iter().map(String::as_str).collect(),
        )
    } else {
        (None, vec![])
    };

    PackagedContextResult {
        intent: intent_str,
        node_count,
        nodes,
        edge_count,
        edges,
        file_count: result.files.len(),
        files,
        truncated: result.truncation.truncated || node_cap < node_count || edge_cap < edge_count,
        nodes_dropped: result.truncation.nodes_dropped + node_count.saturating_sub(node_cap),
        edges_dropped: result.truncation.edges_dropped + edge_count.saturating_sub(edge_cap),
        files_dropped: result.truncation.files_dropped,
        ambiguity_query,
        ambiguity_candidates,
        saved_context_sources: result
            .saved_context_sources
            .iter()
            .map(|s: &SavedContextSource| PackagedSavedSource {
                source_id: &s.source_id,
                label: &s.label,
                source_type: &s.source_type,
                session_id: s.session_id.as_deref(),
                preview: &s.preview,
                retrieval_hint: &s.retrieval_hint,
                relevance_score: s.relevance_score,
            })
            .collect(),
        seed_budgets: &result.seed_budgets,
        traversal_budget: result.traversal_budget.as_ref(),
        payload_truncation: result.truncation.payload.as_ref(),
        source_mix: result
            .truncation
            .payload
            .as_ref()
            .map(|p| p.source_mix.iter().collect())
            .unwrap_or_default(),
        token_budget_applied: result
            .truncation
            .payload
            .as_ref()
            .and_then(|p| p.token_budget_applied),
        budget: result.budget.clone(),
    }
}

pub fn enforce_mcp_response_budget(
    value: &mut Value,
    output_format: OutputFormat,
    max_bytes: usize,
) -> anyhow::Result<Option<BudgetReport>> {
    let requested_bytes = rendered_response_bytes(value, output_format)?;

    while rendered_response_bytes(value, output_format)? > max_bytes {
        if !trim_packaged_context_once(value) {
            break;
        }
    }

    let emitted_bytes = rendered_response_bytes(value, output_format)?;
    if emitted_bytes > max_bytes {
        anyhow::bail!(
            "MCP response exceeds max_mcp_response_bytes after trimming (emitted={emitted_bytes}, limit={max_bytes})"
        );
    }

    if emitted_bytes < requested_bytes {
        let payload = ensure_payload_truncation(value);
        payload.insert(
            "bytes_requested".to_owned(),
            Value::from(requested_bytes as u64),
        );
        payload.insert(
            "bytes_emitted".to_owned(),
            Value::from(emitted_bytes as u64),
        );
        payload.insert(
            "tokens_estimated".to_owned(),
            Value::from(estimate_tokens(emitted_bytes) as u64),
        );
        let omitted_bytes = requested_bytes.saturating_sub(emitted_bytes);
        payload.insert(
            "omitted_byte_count".to_owned(),
            Value::from(omitted_bytes as u64),
        );
        payload.insert(
            "continuation_hint".to_owned(),
            Value::from("narrow query, lower depth, or request JSON with fewer saved artifacts"),
        );
        value["truncated"] = Value::Bool(true);
        return Ok(Some(BudgetReport::partial_result(
            "mcp_cli_payload_serialization.max_mcp_response_bytes",
            max_bytes,
            requested_bytes,
            true,
        )));
    }

    Ok(None)
}

fn rendered_response_bytes(value: &Value, output_format: OutputFormat) -> anyhow::Result<usize> {
    let rendered = render_value(value, output_format)?;
    let mut response = serde_json::json!({
        "content": [{
            "type": "text",
            "text": rendered.text,
            "mimeType": rendered.actual_format.mime_type(),
        }],
        "atlas_output_format": rendered.actual_format.as_str(),
        "atlas_requested_output_format": rendered.requested_format.as_str(),
    });

    if let Some(reason) = rendered.fallback_reason {
        response["atlas_fallback_reason"] = Value::String(reason);
    }

    Ok(serde_json::to_vec(&response)?.len())
}

fn trim_packaged_context_once(value: &mut Value) -> bool {
    if drop_array_entry(value, "saved_context_sources", |entry| {
        (
            0_u8,
            score_value(entry.get("relevance_score")),
            string_key(entry, "source_id"),
            "",
        )
    }) {
        increment_u64(value, "payload_truncation", "omitted_source_count", 1);
        return true;
    }
    if drop_array_entry(value, "files", |entry| {
        (
            keep_priority(entry.get("reason").and_then(Value::as_str)),
            score_value(entry.get("path").map(|_| &Value::Null)),
            string_key(entry, "path"),
            "",
        )
    }) {
        increment_u64(value, "files_dropped", "", 1);
        increment_u64(value, "payload_truncation", "omitted_file_count", 1);
        return true;
    }
    if drop_array_entry(value, "edges", |entry| {
        (
            keep_priority(entry.get("reason").and_then(Value::as_str)),
            0.0,
            string_key(entry, "from"),
            string_key(entry, "to"),
        )
    }) {
        increment_u64(value, "edges_dropped", "", 1);
        return true;
    }
    if drop_array_entry(value, "nodes", |entry| {
        let reason = entry.get("reason").and_then(Value::as_str);
        let qn = entry
            .get("qn")
            .and_then(Value::as_str)
            .or_else(|| {
                entry
                    .get("node")
                    .and_then(|node| node.get("qn"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("");
        (
            keep_priority(reason),
            score_value(entry.get("distance")),
            qn,
            "",
        )
    }) {
        increment_u64(value, "nodes_dropped", "", 1);
        increment_u64(value, "payload_truncation", "omitted_node_count", 1);
        return true;
    }
    false
}

fn drop_array_entry<F>(value: &mut Value, key: &str, score: F) -> bool
where
    F: Fn(&Map<String, Value>) -> (u8, f64, &str, &str),
{
    let Some(array) = value.get_mut(key).and_then(Value::as_array_mut) else {
        return false;
    };
    let candidate = array
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.as_object().map(|entry| (index, score(entry))))
        .min_by(|left, right| {
            left.1
                .0
                .cmp(&right.1.0)
                .then_with(|| {
                    left.1
                        .1
                        .partial_cmp(&right.1.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.1.2.cmp(right.1.2))
                .then_with(|| left.1.3.cmp(right.1.3))
        })
        .map(|(index, _)| index);

    if let Some(index) = candidate {
        array.remove(index);
        return true;
    }
    false
}

fn ensure_payload_truncation(value: &mut Value) -> &mut Map<String, Value> {
    if !value
        .get("payload_truncation")
        .is_some_and(Value::is_object)
    {
        value["payload_truncation"] = serde_json::json!({
            "bytes_requested": 0,
            "bytes_emitted": 0,
            "tokens_estimated": 0,
            "omitted_node_count": 0,
            "omitted_file_count": 0,
            "omitted_source_count": 0,
            "omitted_byte_count": 0,
        });
    }
    value
        .get_mut("payload_truncation")
        .and_then(Value::as_object_mut)
        .expect("payload_truncation object")
}

fn increment_u64(value: &mut Value, root_key: &str, nested_key: &str, amount: u64) {
    if nested_key.is_empty() {
        let current = value.get(root_key).and_then(Value::as_u64).unwrap_or(0);
        value[root_key] = Value::from(current + amount);
        value["truncated"] = Value::Bool(true);
        return;
    }

    let object = ensure_payload_truncation(value);
    let current = object.get(nested_key).and_then(Value::as_u64).unwrap_or(0);
    object.insert(nested_key.to_owned(), Value::from(current + amount));
    value["truncated"] = Value::Bool(true);
}

fn keep_priority(reason: Option<&str>) -> u8 {
    match reason.unwrap_or_default() {
        "direct_target" => 6,
        "test_adjacent" => 5,
        "impact_neighbor" => 4,
        "caller" | "callee" => 3,
        "importer" | "importee" => 2,
        _ => 1,
    }
}

fn score_value(value: Option<&Value>) -> f64 {
    value
        .and_then(Value::as_f64)
        .or_else(|| value.and_then(Value::as_u64).map(|value| value as f64))
        .unwrap_or(0.0)
}

fn string_key<'a>(entry: &'a Map<String, Value>, key: &str) -> &'a str {
    entry.get(key).and_then(Value::as_str).unwrap_or("")
}

fn estimate_tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}
