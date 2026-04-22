use super::*;

fn reason_priority(reason: SelectionReason) -> u8 {
    match reason {
        SelectionReason::DirectTarget => 100,
        SelectionReason::Caller => 80,
        SelectionReason::Callee => 80,
        SelectionReason::Importer => 60,
        SelectionReason::Importee => 60,
        SelectionReason::TestAdjacent => 50,
        SelectionReason::ContainmentSibling => 40,
        SelectionReason::ImpactNeighbor => 30,
    }
}

fn node_score(sn: &SelectedNode, seed_file: &str, seed_qname: &str) -> f64 {
    let _ = seed_qname;
    let mut score = reason_priority(sn.selection_reason) as f64;

    let distance_bonus = (10.0_f64 - sn.distance as f64 * 5.0).max(0.0);
    score += distance_bonus;

    if sn.node.file_path == seed_file {
        score += 3.0;
    }

    if let Some(mods) = &sn.node.modifiers {
        let m = mods.to_lowercase();
        if m.contains("pub") || m.contains("public") || m.contains("export") {
            score += 5.0;
        }
    }

    use atlas_core::NodeKind;
    match sn.node.kind {
        NodeKind::Function | NodeKind::Method => score += 3.0,
        NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface => score += 2.0,
        _ => {}
    }

    if sn.node.is_test || sn.node.kind == NodeKind::Test {
        score -= 10.0;
    }

    score
}

fn edge_score(se: &SelectedEdge) -> f64 {
    let base = reason_priority(se.selection_reason) as f64;
    base + (se.edge.confidence as f64) * 10.0
}

pub(super) fn rank_context(result: &mut ContextResult) {
    let (seed_file, seed_qname) = result
        .nodes
        .iter()
        .find(|n| n.selection_reason == SelectionReason::DirectTarget)
        .map(|n| (n.node.file_path.clone(), n.node.qualified_name.clone()))
        .unwrap_or_default();

    let node_scores: Vec<f32> = result
        .nodes
        .iter()
        .map(|sn| node_score(sn, &seed_file, &seed_qname) as f32)
        .collect();
    for (sn, s) in result.nodes.iter_mut().zip(&node_scores) {
        sn.relevance_score = *s;
    }

    result.nodes.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.qualified_name.cmp(&b.node.qualified_name))
    });

    let edge_scores: Vec<f32> = result
        .edges
        .iter()
        .map(|se| edge_score(se) as f32)
        .collect();
    for (se, s) in result.edges.iter_mut().zip(&edge_scores) {
        se.relevance_score = *s;
    }

    result.edges.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.edge.source_qn.cmp(&b.edge.source_qn))
            .then_with(|| a.edge.target_qn.cmp(&b.edge.target_qn))
    });

    result.files.sort_by(|a, b| {
        let pa = reason_priority(a.selection_reason) as i32;
        let pb = reason_priority(b.selection_reason) as i32;
        pb.cmp(&pa).then_with(|| a.path.cmp(&b.path))
    });
}

pub(super) fn trim_context(result: &mut ContextResult) {
    use atlas_core::model::ContextIntent;

    let max_nodes = result.request.max_nodes.unwrap_or(DEFAULT_MAX_NODES);
    let max_edges = result.request.max_edges.unwrap_or(DEFAULT_MAX_EDGES);
    let max_files = result.request.max_files.unwrap_or(DEFAULT_MAX_FILES);

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
    };
}
