//! Agent-optimized output packaging.
//!
//! Full `Node` and `Edge` structs contain many fields that are redundant or
//! irrelevant in an agent-facing context.  The compact types here strip those
//! fields to reduce token overhead while keeping the information an agent
//! actually needs.

use atlas_core::model::{Edge, ImpactResult, Node, ReviewContext};
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
// Packaged review context
// ---------------------------------------------------------------------------

/// Agent-ready review context with compact nodes.
#[derive(Serialize)]
pub struct PackagedReview<'a> {
    pub changed_files: &'a [String],
    pub changed_symbol_count: usize,
    pub changed_symbols: Vec<CompactNode<'a>>,
    pub impacted_neighbor_count: usize,
    pub impacted_neighbors: Vec<CompactNode<'a>>,
    pub critical_edges: Vec<CompactEdge<'a>>,
    pub risk: PackagedRisk,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct PackagedRisk {
    pub changed_symbol_count: usize,
    pub public_api_changes: usize,
    pub test_adjacent: bool,
    pub cross_module_impact: bool,
}

pub fn package_review<'a>(ctx: &'a ReviewContext) -> PackagedReview<'a> {
    let sym_total = ctx.changed_symbols.len();
    let nbr_total = ctx.impacted_neighbors.len();
    let edge_total = ctx.critical_edges.len();

    let sym_capped = sym_total.min(MAX_NODES);
    let nbr_capped = nbr_total.min(MAX_NODES);
    let edge_capped = edge_total.min(MAX_EDGES);

    PackagedReview {
        changed_files: &ctx.changed_files,
        changed_symbol_count: sym_total,
        changed_symbols: ctx.changed_symbols[..sym_capped]
            .iter()
            .map(compact_node)
            .collect(),
        impacted_neighbor_count: nbr_total,
        impacted_neighbors: ctx.impacted_neighbors[..nbr_capped]
            .iter()
            .map(compact_node)
            .collect(),
        critical_edges: ctx.critical_edges[..edge_capped]
            .iter()
            .map(compact_edge)
            .collect(),
        risk: PackagedRisk {
            changed_symbol_count: ctx.risk_summary.changed_symbol_count,
            public_api_changes: ctx.risk_summary.public_api_changes,
            test_adjacent: ctx.risk_summary.test_adjacent,
            cross_module_impact: ctx.risk_summary.cross_module_impact,
        },
        truncated: sym_capped < sym_total || nbr_capped < nbr_total || edge_capped < edge_total,
    }
}
