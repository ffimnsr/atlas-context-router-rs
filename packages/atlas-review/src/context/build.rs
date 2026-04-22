use super::*;

pub fn build_context(store: &Store, request: &ContextRequest) -> Result<ContextResult> {
    use atlas_core::model::ContextIntent;
    match request.intent {
        ContextIntent::Review => return build_review_context(store, request),
        ContextIntent::Impact
        | ContextIntent::ImpactAnalysis
        | ContextIntent::RefactorSafety
        | ContextIntent::DependencyRemoval => return build_impact_context(store, request),
        _ => {}
    }

    match &request.target {
        ContextTarget::ChangedSymbols { qnames } => {
            let mut paths: Vec<String> = Vec::new();
            for qn in qnames {
                if let Some(node) = store.node_by_qname(qn)? {
                    paths.push(node.file_path);
                }
            }
            paths.dedup();
            let derived = ContextRequest {
                target: ContextTarget::ChangedFiles { paths },
                ..request.clone()
            };
            return build_impact_context(store, &derived);
        }
        ContextTarget::EdgeQuerySeed {
            source_qname,
            edge_kind: _,
        } => {
            return match store.node_by_qname(source_qname)? {
                Some(node) => build_symbol_context(store, node, request),
                None => Ok(build_not_found_result(request, vec![])),
            };
        }
        _ => {}
    }

    let resolved = resolve_target(store, &request.target)?;
    match resolved {
        ResolvedTarget::Node(node) => build_symbol_context(store, *node, request),
        ResolvedTarget::File(path) => {
            let nodes = store.nodes_by_file(&path)?;
            match nodes.into_iter().next() {
                Some(first_node) => build_symbol_context(store, first_node, request),
                None => Ok(build_not_found_result(request, vec![])),
            }
        }
        ResolvedTarget::Ambiguous(meta) => Ok(build_ambiguous_result(request, meta)),
        ResolvedTarget::NotFound { suggestions } => {
            Ok(build_not_found_result(request, suggestions))
        }
    }
}

pub(super) fn build_review_context(
    store: &Store,
    request: &ContextRequest,
) -> Result<ContextResult> {
    let changed_paths = extract_changed_paths(request);
    let path_refs: Vec<&str> = changed_paths.iter().map(String::as_str).collect();

    let max_nodes = request.max_nodes.unwrap_or(DEFAULT_MAX_NODES);
    let max_depth = request.depth.unwrap_or(2);
    let traversal_max_nodes = max_nodes.saturating_add(max_nodes.min(16));

    let impact = store.impact_radius(&path_refs, max_depth, traversal_max_nodes)?;
    let advanced = atlas_impact::analyze(impact.clone());
    let impact_scores: HashMap<String, f64> = advanced
        .scored_nodes
        .iter()
        .map(|scored| (scored.node.qualified_name.clone(), scored.impact_score))
        .collect();

    let changed_qns: HashSet<String> = impact
        .changed_nodes
        .iter()
        .map(|n| n.qualified_name.clone())
        .collect();

    let mut nodes: Vec<SelectedNode> = Vec::new();
    let mut seen_qnames: HashSet<String> = HashSet::new();

    for node in impact.changed_nodes {
        seen_qnames.insert(node.qualified_name.clone());
        nodes.push(SelectedNode {
            node,
            selection_reason: SelectionReason::DirectTarget,
            distance: 0,
            relevance_score: 0.0,
        });
    }

    for node in impact.impacted_nodes {
        let qn = node.qualified_name.clone();
        if seen_qnames.insert(qn) {
            nodes.push(SelectedNode {
                node,
                selection_reason: SelectionReason::ImpactNeighbor,
                distance: 1,
                relevance_score: 0.0,
            });
        }
    }

    let edges: Vec<SelectedEdge> = impact
        .relevant_edges
        .into_iter()
        .filter(|e| {
            changed_qns.contains(e.source_qn.as_str())
                || changed_qns.contains(e.target_qn.as_str())
                || seen_qnames.contains(e.source_qn.as_str())
                || seen_qnames.contains(e.target_qn.as_str())
        })
        .map(|edge| SelectedEdge {
            edge,
            selection_reason: SelectionReason::ImpactNeighbor,
            depth: None,
            relevance_score: 0.0,
        })
        .collect();

    let files = collect_files(&nodes);

    let mut result = ContextResult {
        request: request.clone(),
        nodes,
        edges,
        files,
        truncation: TruncationMeta::none(),
        ambiguity: None,
        workflow: None,
        saved_context_sources: vec![],
    };

    rank_context(&mut result);
    apply_impact_focus_scores(&mut result, &impact_scores);
    trim_context(&mut result);
    update_file_node_counts(&mut result);

    if request.include_code_spans {
        apply_code_spans(&mut result);
    }

    result.workflow = Some(build_workflow_summary(&result));

    Ok(result)
}

pub(super) fn build_impact_context(
    store: &Store,
    request: &ContextRequest,
) -> Result<ContextResult> {
    let seed_paths: Vec<String> = match &request.target {
        ContextTarget::ChangedFiles { paths } => paths.clone(),
        ContextTarget::FilePath { path } => vec![path.clone()],
        ContextTarget::QualifiedName { .. } | ContextTarget::SymbolName { .. } => {
            match resolve_target(store, &request.target)? {
                ResolvedTarget::Node(node) => vec![node.file_path.clone()],
                ResolvedTarget::File(path) => vec![path],
                ResolvedTarget::Ambiguous(meta) => {
                    return Ok(build_ambiguous_result(request, meta));
                }
                ResolvedTarget::NotFound { suggestions } => {
                    return Ok(build_not_found_result(request, suggestions));
                }
            }
        }
        ContextTarget::ChangedSymbols { qnames } => {
            let mut paths: Vec<String> = Vec::new();
            for qn in qnames {
                if let Some(node) = store.node_by_qname(qn)? {
                    paths.push(node.file_path);
                }
            }
            paths.dedup();
            paths
        }
        ContextTarget::EdgeQuerySeed { source_qname, .. } => {
            match store.node_by_qname(source_qname)? {
                Some(node) => vec![node.file_path],
                None => return Ok(build_not_found_result(request, vec![])),
            }
        }
    };

    let adapted = ContextRequest {
        intent: request.intent,
        target: ContextTarget::ChangedFiles { paths: seed_paths },
        ..request.clone()
    };
    build_review_context(store, &adapted)
}

fn extract_changed_paths(request: &ContextRequest) -> Vec<String> {
    if let ContextTarget::ChangedFiles { paths } = &request.target {
        paths.clone()
    } else {
        vec![]
    }
}

fn apply_impact_focus_scores(result: &mut ContextResult, impact_scores: &HashMap<String, f64>) {
    for node in &mut result.nodes {
        if let Some(score) = impact_scores.get(&node.node.qualified_name) {
            node.relevance_score += (*score as f32) * 20.0;
        }
    }

    for edge in &mut result.edges {
        let source = impact_scores
            .get(&edge.edge.source_qn)
            .copied()
            .unwrap_or(0.0);
        let target = impact_scores
            .get(&edge.edge.target_qn)
            .copied()
            .unwrap_or(0.0);
        edge.relevance_score += ((source + target) as f32) * 5.0;
    }

    result.nodes.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.qualified_name.cmp(&b.node.qualified_name))
    });

    result.edges.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.edge.source_qn.cmp(&b.edge.source_qn))
            .then_with(|| a.edge.target_qn.cmp(&b.edge.target_qn))
    });
}
