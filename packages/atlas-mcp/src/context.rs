//! Agent-optimized output packaging.
//!
//! Full `Node` and `Edge` structs contain many fields that are redundant or
//! irrelevant in an agent-facing context.  The compact types here strip those
//! fields to reduce token overhead while keeping the information an agent
//! actually needs.

use atlas_core::model::{ContextResult, Edge, ImpactResult, Node, SelectedEdge, SelectedNode};
use serde::Serialize;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambiguity_query: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ambiguity_candidates: Vec<&'a str>,
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
        ambiguity_query,
        ambiguity_candidates,
    }
}
