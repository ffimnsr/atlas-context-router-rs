use super::*;
use crate::ranking::{
    ContextRankingPrimitives, TrimmingPrimitives, compare_edge_scores, compare_file_priorities,
    compare_node_scores,
};

pub(super) fn rank_context(result: &mut ContextResult) {
    let ranking = ContextRankingPrimitives::default();
    let (seed_file, seed_qname) = result
        .nodes
        .iter()
        .find(|n| n.selection_reason == SelectionReason::DirectTarget)
        .map(|n| (n.node.file_path.clone(), n.node.qualified_name.clone()))
        .unwrap_or_default();

    let node_scores: Vec<f32> = result
        .nodes
        .iter()
        .map(|sn| {
            let _ = &seed_qname;
            ranking.node_score(sn, &seed_file) as f32
        })
        .collect();
    for (sn, s) in result.nodes.iter_mut().zip(&node_scores) {
        sn.relevance_score = *s;
    }

    result.nodes.sort_by(|a, b| {
        compare_node_scores(
            a.relevance_score as f64,
            b.relevance_score as f64,
            &a.node.qualified_name,
            &b.node.qualified_name,
        )
    });

    let edge_scores: Vec<f32> = result
        .edges
        .iter()
        .map(|se| ranking.edge_score(se) as f32)
        .collect();
    for (se, s) in result.edges.iter_mut().zip(&edge_scores) {
        se.relevance_score = *s;
    }

    result.edges.sort_by(|a, b| {
        compare_edge_scores(
            a.relevance_score as f64,
            b.relevance_score as f64,
            &a.edge.source_qn,
            &b.edge.source_qn,
            &a.edge.target_qn,
            &b.edge.target_qn,
        )
    });

    result.files.sort_by(|a, b| {
        compare_file_priorities(
            ranking.selection_priority(a.selection_reason),
            ranking.selection_priority(b.selection_reason),
            &a.path,
            &b.path,
        )
    });
}

pub(super) fn trim_context(result: &mut ContextResult) {
    use atlas_core::model::ContextIntent;

    let limits = TrimmingPrimitives::from_request(&result.request);
    let max_nodes = limits.max_nodes;
    let max_edges = limits.max_edges;
    let max_files = limits.max_files;

    let original_node_count = result.nodes.len();
    if original_node_count > max_nodes {
        let (targets, rest): (Vec<_>, Vec<_>) = result
            .nodes
            .drain(..)
            .partition(|n| n.selection_reason == SelectionReason::DirectTarget);

        let reserve_non_target = usize::from(
            result.request.intent == ContextIntent::Review
                && max_nodes > 1
                && !rest.is_empty()
                && targets.len() >= max_nodes,
        );
        let keep_targets = max_nodes.saturating_sub(reserve_non_target);

        result.nodes = targets.into_iter().take(keep_targets).collect();
        let budget = max_nodes.saturating_sub(result.nodes.len());
        result.nodes.extend(rest.into_iter().take(budget));
    }
    let dropped_nodes = original_node_count.saturating_sub(result.nodes.len());

    let remaining_qnames: HashSet<&str> = result
        .nodes
        .iter()
        .map(|n| n.node.qualified_name.as_str())
        .collect();

    let original_edge_count = result.edges.len();
    result.edges.retain(|se| {
        remaining_qnames.contains(se.edge.source_qn.as_str())
            && remaining_qnames.contains(se.edge.target_qn.as_str())
    });
    let edges_after_prune = result.edges.len();
    if edges_after_prune > max_edges {
        result.edges.truncate(max_edges);
    }
    let dropped_edges = original_edge_count.saturating_sub(result.edges.len());

    let original_file_count = result.files.len();
    if original_file_count > max_files {
        let (target_files, rest): (Vec<_>, Vec<_>) = result
            .files
            .drain(..)
            .partition(|f| f.selection_reason == SelectionReason::DirectTarget);
        result.files = target_files;
        let budget = max_files.saturating_sub(result.files.len());
        result.files.extend(rest.into_iter().take(budget));
    }
    let dropped_files = original_file_count.saturating_sub(result.files.len());

    result.truncation = TruncationMeta {
        nodes_dropped: dropped_nodes,
        edges_dropped: dropped_edges,
        files_dropped: dropped_files,
        truncated: dropped_nodes > 0 || dropped_edges > 0 || dropped_files > 0,
        payload: None,
    };
}
