use super::*;

// ---------------------------------------------------------------------------
// Graph-aware expansion
// ---------------------------------------------------------------------------

/// Expand a set of FTS seed results through the graph, adding neighboring
/// nodes at a distance-decayed score.
///
/// The caller's original scored seeds occupy hop-0 (distance 0). Each
/// successive hop decays the maximum seed score by `1 / (hop + 1)`.
/// Nodes already present at a shorter distance are never overwritten.
/// The combined set is truncated to `limit` after sorting by score.
pub fn graph_expand(
    store: &Store,
    seeds: Vec<ScoredNode>,
    max_hops: u32,
    limit: usize,
) -> Result<Vec<ScoredNode>> {
    // Map qualified_name → ScoredNode; seeds are inserted at their own score.
    let mut result_map: HashMap<String, ScoredNode> = HashMap::new();

    for s in &seeds {
        let mut seeded = s.clone();
        annotate_graph_seed(&mut seeded, 0, s.node.qualified_name.clone());
        result_map
            .entry(seeded.node.qualified_name.clone())
            .or_insert(seeded);
    }

    let max_seed_score = seeds
        .iter()
        .map(|s| s.score)
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let mut frontier: Vec<(String, String)> = seeds
        .iter()
        .map(|s| (s.node.qualified_name.clone(), s.node.qualified_name.clone()))
        .collect();

    for hop in 1..=max_hops {
        if frontier.is_empty() || result_map.len() >= limit {
            break;
        }

        let hop_score = max_seed_score / (hop as f64 + 1.0);
        let mut next_frontier: Vec<(String, String)> = Vec::new();

        for (frontier_qn, seed_qn) in frontier {
            let neighbors = store.nodes_connected_to(&[frontier_qn.as_str()])?;
            debug!(
                hop,
                frontier = frontier_qn,
                neighbors = neighbors.len(),
                "graph expansion hop"
            );

            for neighbor in neighbors {
                let qn = neighbor.qualified_name.clone();
                if !result_map.contains_key(&qn) {
                    let mut expanded = ScoredNode::with_ranking_evidence(
                        neighbor,
                        hop_score,
                        RankingEvidence::new(RetrievalMode::GraphExpand, hop_score)
                            .with_raw_score(hop_score),
                    );
                    annotate_graph_seed(&mut expanded, hop, seed_qn.clone());
                    result_map.insert(qn.clone(), expanded);
                    next_frontier.push((qn, seed_qn.clone()));
                }
            }
        }

        frontier = next_frontier;
    }

    let mut results: Vec<ScoredNode> = result_map.into_values().collect();
    sort_scored_nodes(&mut results);
    results.truncate(limit);
    Ok(results)
}
