use super::*;

/// Build a symbol-centred [`ContextResult`] from a resolved seed node.
pub(super) fn build_symbol_context(
    store: &Store,
    seed: atlas_core::model::Node,
    request: &ContextRequest,
) -> Result<ContextResult> {
    let qname = seed.qualified_name.clone();

    let mut nodes: Vec<SelectedNode> = Vec::new();
    let mut edges: Vec<SelectedEdge> = Vec::new();
    let mut seen_qnames: HashSet<String> = HashSet::new();

    seen_qnames.insert(qname.clone());
    nodes.push(SelectedNode {
        node: seed.clone(),
        selection_reason: SelectionReason::DirectTarget,
        distance: 0,
        relevance_score: 0.0,
    });

    let depth = request.depth.unwrap_or(1).max(1);
    let mut frontier_qnames: Vec<String> = vec![qname.clone()];

    for hop in 1..=depth {
        let mut next_frontier: Vec<String> = Vec::new();

        for fqname in &frontier_qnames {
            if request.include_callers {
                for (caller, edge) in store.direct_callers(fqname, BUCKET_CALLERS)? {
                    let cqn = caller.qualified_name.clone();
                    if seen_qnames.insert(cqn.clone()) {
                        next_frontier.push(cqn);
                        edges.push(SelectedEdge {
                            edge,
                            selection_reason: SelectionReason::Caller,
                            depth: Some(hop),
                            relevance_score: 0.0,
                        });
                        nodes.push(SelectedNode {
                            node: caller,
                            selection_reason: SelectionReason::Caller,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }

            if request.include_callees {
                for (callee, edge) in store.direct_callees(fqname, BUCKET_CALLEES)? {
                    let cqn = callee.qualified_name.clone();
                    if seen_qnames.insert(cqn.clone()) {
                        next_frontier.push(cqn);
                        edges.push(SelectedEdge {
                            edge,
                            selection_reason: SelectionReason::Callee,
                            depth: Some(hop),
                            relevance_score: 0.0,
                        });
                        nodes.push(SelectedNode {
                            node: callee,
                            selection_reason: SelectionReason::Callee,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }

            if request.include_imports {
                for (import_node, edge) in store.import_neighbors(fqname, BUCKET_IMPORTS)? {
                    let iqn = import_node.qualified_name.clone();
                    if seen_qnames.insert(iqn) {
                        edges.push(SelectedEdge {
                            edge,
                            selection_reason: SelectionReason::Importee,
                            depth: Some(hop),
                            relevance_score: 0.0,
                        });
                        nodes.push(SelectedNode {
                            node: import_node,
                            selection_reason: SelectionReason::Importee,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }

            if hop == 1 && request.include_neighbors {
                for sibling in store.containment_siblings(fqname, BUCKET_SIBLINGS)? {
                    let sqn = sibling.qualified_name.clone();
                    if seen_qnames.insert(sqn) {
                        nodes.push(SelectedNode {
                            node: sibling,
                            selection_reason: SelectionReason::ContainmentSibling,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }

            if hop == 1 && request.include_tests {
                for (test_node, edge) in store.test_neighbors(fqname, BUCKET_TESTS)? {
                    let tqn = test_node.qualified_name.clone();
                    if seen_qnames.insert(tqn) {
                        edges.push(SelectedEdge {
                            edge,
                            selection_reason: SelectionReason::TestAdjacent,
                            depth: Some(hop),
                            relevance_score: 0.0,
                        });
                        nodes.push(SelectedNode {
                            node: test_node,
                            selection_reason: SelectionReason::TestAdjacent,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }
        }

        frontier_qnames = next_frontier;
        if frontier_qnames.is_empty() {
            break;
        }
    }

    let files = collect_files(&nodes);

    let mut result = ContextResult {
        request: request.clone(),
        nodes,
        edges,
        files,
        truncation: TruncationMeta::none(),
        seed_budgets: vec![],
        traversal_budget: None,
        ambiguity: None,
        workflow: None,
        saved_context_sources: vec![],
        budget: BudgetReport::not_applicable(),
    };

    rank_context(&mut result);
    trim_context(&mut result);
    update_file_node_counts(&mut result);

    if request.include_code_spans {
        apply_code_spans(&mut result);
    }

    result.workflow = Some(build_workflow_summary(&result));

    Ok(result)
}

pub(super) fn build_ambiguous_result(
    request: &ContextRequest,
    meta: AmbiguityMeta,
) -> ContextResult {
    ContextResult {
        request: request.clone(),
        nodes: vec![],
        edges: vec![],
        files: vec![],
        truncation: TruncationMeta::none(),
        seed_budgets: vec![],
        traversal_budget: None,
        ambiguity: Some(meta),
        workflow: None,
        saved_context_sources: vec![],
        budget: BudgetReport::not_applicable(),
    }
}

pub(super) fn build_not_found_result(
    request: &ContextRequest,
    suggestions: Vec<String>,
) -> ContextResult {
    let ambiguity = if suggestions.is_empty() {
        None
    } else {
        Some(AmbiguityMeta {
            query: format!("{:?}", request.target),
            candidates: suggestions,
            resolved: false,
        })
    };
    ContextResult {
        request: request.clone(),
        nodes: vec![],
        edges: vec![],
        files: vec![],
        truncation: TruncationMeta::none(),
        seed_budgets: vec![],
        traversal_budget: None,
        ambiguity,
        workflow: None,
        saved_context_sources: vec![],
        budget: BudgetReport::not_applicable(),
    }
}

pub(super) fn collect_files(nodes: &[SelectedNode]) -> Vec<SelectedFile> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut files: Vec<SelectedFile> = Vec::new();
    for sn in nodes {
        let path = sn.node.file_path.clone();
        if seen.insert(path.clone()) {
            let reason = if sn.selection_reason == SelectionReason::DirectTarget {
                SelectionReason::DirectTarget
            } else {
                sn.selection_reason
            };
            files.push(SelectedFile {
                path,
                selection_reason: reason,
                line_ranges: vec![],
                language: Some(sn.node.language.clone()),
                node_count_included: 0,
            });
        }
    }
    files
}

pub(super) fn update_file_node_counts(result: &mut ContextResult) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for sn in &result.nodes {
        *counts.entry(sn.node.file_path.clone()).or_insert(0) += 1;
    }
    for sf in &mut result.files {
        sf.node_count_included = counts.get(&sf.path).copied().unwrap_or(0);
    }
}
