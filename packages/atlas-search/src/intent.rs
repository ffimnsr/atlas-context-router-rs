//! Execution of supported query intent phrases.
//!
//! `who calls`, `what breaks`, and `tests for` resolve a concrete target symbol
//! and then answer from graph edges instead of lexical FTS hits. When the target
//! cannot be resolved, callers fall back to the normal search path.

use atlas_core::{Result, ScoredNode};
use atlas_store_sqlite::Store;

use crate::resolve::{SymbolResolveRequest, resolve_symbol_nodes};

/// Result of executing one intent phrase against the graph.
#[derive(Debug, Clone)]
pub struct IntentExecution {
    /// Graph-derived matches, ranked deterministically.
    pub results: Vec<ScoredNode>,
    /// `true` when the results came from usage/test edges rather than FTS.
    pub usage_edges_included: bool,
    /// Canonical qualified name of the resolved intent target.
    pub target_qname: Option<String>,
}

fn scored(node: atlas_core::Node, rank: usize) -> ScoredNode {
    ScoredNode::new(node, 1.0 - (rank as f64 * 0.001))
}

fn dedupe(nodes: Vec<atlas_core::Node>, limit: usize) -> Vec<ScoredNode> {
    let mut seen = std::collections::HashSet::new();
    nodes
        .into_iter()
        .filter(|node| seen.insert(node.qualified_name.clone()))
        .take(limit)
        .enumerate()
        .map(|(rank, node)| scored(node, rank))
        .collect()
}

/// Resolve `target` and execute the intent phrase identified by `kind`.
///
/// Returns `Ok(None)` for unsupported kinds or when the target symbol cannot be
/// resolved, so callers can keep the lexical search results.
pub fn execute_intent_query(
    store: &Store,
    kind: &str,
    target: &str,
    limit: usize,
) -> Result<Option<IntentExecution>> {
    if !matches!(kind, "who_calls" | "what_breaks" | "tests_for") {
        return Ok(None);
    }
    let limit = limit.max(1);

    let resolution = resolve_symbol_nodes(
        store,
        &SymbolResolveRequest {
            name: target,
            kind: None,
            language: None,
            file: None,
            limit: 1,
            fuzzy: false,
        },
    )?;
    let Some(node) = resolution.matches.first() else {
        return Ok(None);
    };
    let qname = node.qualified_name.clone();

    let nodes: Vec<atlas_core::Node> = match kind {
        "who_calls" => store
            .direct_callers(&qname, limit)?
            .into_iter()
            .map(|(node, _)| node)
            .collect(),
        "what_breaks" => {
            let max_edges = (limit * 8).max(32);
            let traversal = store.traverse_from_qnames(&[qname.as_str()], 2, limit, max_edges)?;
            traversal.impacted_nodes
        }
        "tests_for" => store
            .test_neighbors(&qname, limit)?
            .into_iter()
            .map(|(node, _)| node)
            .filter(|node| node.is_test || node.kind == atlas_core::NodeKind::Test)
            .collect(),
        _ => unreachable!("intent kind checked above"),
    };

    Ok(Some(IntentExecution {
        results: dedupe(nodes, limit),
        usage_edges_included: true,
        target_qname: Some(qname),
    }))
}
