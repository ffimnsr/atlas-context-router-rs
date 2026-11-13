//! Wire shaping for `traverse_graph` edges.
//!
//! Traversal collects edges from the raw store, which can include expression
//! fragments that are not nodes and duplicate `(from, kind, to)` triples. This
//! module produces the emitted edge list: deduped, endpoint-valid, and capped to
//! root adjacency when `max_depth == 1`.

use std::collections::HashSet;

use atlas_core::Edge;
use serde_json::{Value, json};

/// Build the emitted `edges[]` array.
///
/// - `node_qns` must contain every qualified name present in `nodes[]` plus the
///   root symbol. Edges with an endpoint outside that set target expression
///   fragments that are not graph nodes.
/// - When `max_depth <= 1`, only root-adjacent edges are emitted.
pub(super) fn traversal_edges(
    edges: &[Edge],
    root_qn: &str,
    max_depth: u32,
    node_qns: &HashSet<String>,
) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for edge in edges {
        if !node_qns.contains(edge.source_qn.as_str())
            || !node_qns.contains(edge.target_qn.as_str())
        {
            continue;
        }
        // At depth 1 the contract is root adjacency only; neighbor↔neighbor
        // edges are transitive and must not appear.
        if max_depth <= 1 && edge.source_qn != root_qn && edge.target_qn != root_qn {
            continue;
        }
        if !seen.insert((
            edge.source_qn.as_str(),
            edge.kind.as_str(),
            edge.target_qn.as_str(),
        )) {
            continue;
        }
        out.push(json!({
            "from": edge.source_qn,
            "to": edge.target_qn,
            "kind": edge.kind.as_str(),
            "direction": if edge.source_qn == root_qn {
                "outbound"
            } else if edge.target_qn == root_qn {
                "inbound"
            } else {
                "transitive"
            }
        }));
    }
    out
}
