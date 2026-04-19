//! Agent-optimized output packaging.
//!
//! Full `Node` and `Edge` structs contain many fields that are redundant or
//! irrelevant in an agent-facing context.  The compact types here strip those
//! fields to reduce token overhead while keeping the information an agent
//! actually needs.

use atlas_core::model::{
    ChangedSymbolSummary, Edge, ImpactResult, Node, ReviewContext, ReviewImpactOverview,
};
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
    pub changed_symbol_summaries: Vec<PackagedChangedSymbolSummary<'a>>,
    pub impacted_neighbor_count: usize,
    pub impacted_neighbors: Vec<CompactNode<'a>>,
    pub critical_edges: Vec<CompactEdge<'a>>,
    pub impact_overview: PackagedImpactOverview,
    pub risk: PackagedRisk,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct PackagedChangedSymbolSummary<'a> {
    pub node: CompactNode<'a>,
    pub callers: Vec<CompactNode<'a>>,
    pub callees: Vec<CompactNode<'a>>,
    pub importers: Vec<CompactNode<'a>>,
    pub tests: Vec<CompactNode<'a>>,
}

#[derive(Serialize)]
pub struct PackagedImpactOverview {
    pub max_depth: u32,
    pub max_nodes: usize,
    pub impacted_node_count: usize,
    pub impacted_file_count: usize,
    pub relevant_edge_count: usize,
    pub reached_node_limit: bool,
}

#[derive(Serialize)]
pub struct PackagedRisk {
    pub changed_symbol_count: usize,
    pub public_api_changes: usize,
    pub test_adjacent: bool,
    pub affected_test_count: usize,
    pub uncovered_changed_symbol_count: usize,
    pub large_function_touched: bool,
    pub large_function_count: usize,
    pub cross_module_impact: bool,
    pub cross_package_impact: bool,
}

pub fn package_review<'a>(ctx: &'a ReviewContext) -> PackagedReview<'a> {
    let sym_total = ctx.changed_symbols.len();
    let nbr_total = ctx.impacted_neighbors.len();
    let edge_total = ctx.critical_edges.len();

    let sym_capped = sym_total.min(MAX_NODES);
    let nbr_capped = nbr_total.min(MAX_NODES);
    let edge_capped = edge_total.min(MAX_EDGES);
    let summary_capped = ctx.changed_symbol_summaries.len().min(MAX_NODES);

    PackagedReview {
        changed_files: &ctx.changed_files,
        changed_symbol_count: sym_total,
        changed_symbols: ctx.changed_symbols[..sym_capped]
            .iter()
            .map(compact_node)
            .collect(),
        changed_symbol_summaries: ctx.changed_symbol_summaries[..summary_capped]
            .iter()
            .map(package_changed_symbol_summary)
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
        impact_overview: package_impact_overview(&ctx.impact_overview),
        risk: PackagedRisk {
            changed_symbol_count: ctx.risk_summary.changed_symbol_count,
            public_api_changes: ctx.risk_summary.public_api_changes,
            test_adjacent: ctx.risk_summary.test_adjacent,
            affected_test_count: ctx.risk_summary.affected_test_count,
            uncovered_changed_symbol_count: ctx.risk_summary.uncovered_changed_symbol_count,
            large_function_touched: ctx.risk_summary.large_function_touched,
            large_function_count: ctx.risk_summary.large_function_count,
            cross_module_impact: ctx.risk_summary.cross_module_impact,
            cross_package_impact: ctx.risk_summary.cross_package_impact,
        },
        truncated: sym_capped < sym_total
            || summary_capped < ctx.changed_symbol_summaries.len()
            || nbr_capped < nbr_total
            || edge_capped < edge_total,
    }
}

fn package_changed_symbol_summary(
    summary: &ChangedSymbolSummary,
) -> PackagedChangedSymbolSummary<'_> {
    PackagedChangedSymbolSummary {
        node: compact_node(&summary.node),
        callers: summary.callers.iter().map(compact_node).collect(),
        callees: summary.callees.iter().map(compact_node).collect(),
        importers: summary.importers.iter().map(compact_node).collect(),
        tests: summary.tests.iter().map(compact_node).collect(),
    }
}

fn package_impact_overview(overview: &ReviewImpactOverview) -> PackagedImpactOverview {
    PackagedImpactOverview {
        max_depth: overview.max_depth,
        max_nodes: overview.max_nodes,
        impacted_node_count: overview.impacted_node_count,
        impacted_file_count: overview.impacted_file_count,
        relevant_edge_count: overview.relevant_edge_count,
        reached_node_limit: overview.reached_node_limit,
    }
}
