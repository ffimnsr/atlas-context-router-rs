use super::*;

pub fn build_context(
    store: &Store,
    request: &ContextRequest,
    policy: &BudgetPolicy,
) -> Result<ContextResult> {
    use atlas_core::model::ContextIntent;
    match request.intent {
        ContextIntent::Review => return build_review_context(store, request, policy),
        ContextIntent::Impact
        | ContextIntent::ImpactAnalysis
        | ContextIntent::RefactorSafety
        | ContextIntent::DependencyRemoval => return build_impact_context(store, request, policy),
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
            return build_impact_context(store, &derived, policy);
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
    policy: &BudgetPolicy,
) -> Result<ContextResult> {
    let changed_paths = extract_changed_paths(request);
    let accepted_paths: Vec<String> = changed_paths
        .iter()
        .take(policy.graph_traversal.seed_files.default_limit)
        .cloned()
        .collect();

    let max_depth = request.depth.unwrap_or(2);
    let traversal_max_nodes = policy.graph_traversal.nodes.default_limit;
    let traversal_max_edges = policy.graph_traversal.edges.default_limit;

    let impact = store.impact_radius(
        &accepted_paths
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        max_depth,
        traversal_max_nodes,
        traversal_max_edges,
    )?;
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
        seed_budgets: vec![SeedBudgetMeta::new(
            "changed_files",
            changed_paths.len(),
            accepted_paths.len(),
            true,
            (changed_paths.len() > accepted_paths.len()).then(|| {
                format!(
                    "narrow changed file list to {} file(s) or query one package/path",
                    accepted_paths.len()
                )
            }),
        )],
        traversal_budget: impact.traversal_budget.clone(),
        ambiguity: None,
        workflow: None,
        saved_context_sources: vec![],
        budget: BudgetReport::not_applicable(),
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
    policy: &BudgetPolicy,
) -> Result<ContextResult> {
    let (seed_paths, seed_budgets): (Vec<String>, Vec<SeedBudgetMeta>) = match &request.target {
        ContextTarget::ChangedFiles { paths } => (paths.clone(), vec![]),
        ContextTarget::FilePath { path } => (vec![path.clone()], vec![]),
        ContextTarget::QualifiedName { .. } | ContextTarget::SymbolName { .. } => {
            match resolve_target(store, &request.target)? {
                ResolvedTarget::Node(node) => (vec![node.file_path.clone()], vec![]),
                ResolvedTarget::File(path) => (vec![path], vec![]),
                ResolvedTarget::Ambiguous(meta) => {
                    let mut result = build_ambiguous_result(request, meta);
                    result.seed_budgets.push(SeedBudgetMeta::new(
                        "symbol_resolution",
                        0,
                        0,
                        false,
                        Some("narrow symbol query to exact qualified name or file path".to_owned()),
                    ));
                    return Ok(result);
                }
                ResolvedTarget::NotFound { suggestions } => {
                    return Ok(build_not_found_result(request, suggestions));
                }
            }
        }
        ContextTarget::ChangedSymbols { qnames } => {
            let accepted_qnames: Vec<String> = qnames
                .iter()
                .take(policy.graph_traversal.seed_nodes.default_limit)
                .cloned()
                .collect();
            let mut paths: Vec<String> = Vec::new();
            for qn in &accepted_qnames {
                if let Some(node) = store.node_by_qname(qn)? {
                    paths.push(node.file_path);
                }
            }
            paths.dedup();
            (
                paths,
                vec![SeedBudgetMeta::new(
                    "changed_symbols",
                    qnames.len(),
                    accepted_qnames.len(),
                    true,
                    (qnames.len() > accepted_qnames.len()).then(|| {
                        format!(
                            "narrow changed symbol list to {} symbol(s) or split request",
                            accepted_qnames.len()
                        )
                    }),
                )],
            )
        }
        ContextTarget::EdgeQuerySeed { source_qname, .. } => {
            match store.node_by_qname(source_qname)? {
                Some(node) => (vec![node.file_path], vec![]),
                None => return Ok(build_not_found_result(request, vec![])),
            }
        }
    };

    let adapted = ContextRequest {
        intent: request.intent,
        target: ContextTarget::ChangedFiles { paths: seed_paths },
        ..request.clone()
    };
    let mut result = build_review_context(store, &adapted, policy)?;
    result.seed_budgets.extend(seed_budgets);
    Ok(result)
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
