use super::*;

pub(super) fn build_workflow_summary(result: &ContextResult) -> WorkflowSummary {
    let high_impact_nodes = result
        .nodes
        .iter()
        .take(5)
        .map(|node| WorkflowFocusNode {
            qualified_name: node.node.qualified_name.clone(),
            kind: node.node.kind.as_str().to_string(),
            file_path: node.node.file_path.clone(),
            relevance_score: node.relevance_score,
            selection_reason: node.selection_reason.as_str().to_string(),
        })
        .collect();

    let impacted_components = build_impacted_components(&result.nodes);
    let call_chains = build_call_chains(result);
    let ripple_effects = build_ripple_effects(result, &impacted_components, &call_chains);
    let headline = build_workflow_headline(result, &impacted_components, &call_chains);

    WorkflowSummary {
        headline,
        high_impact_nodes,
        impacted_components,
        call_chains,
        ripple_effects,
        noise_reduction: build_noise_reduction_summary(result),
    }
}

fn build_workflow_headline(
    result: &ContextResult,
    components: &[WorkflowComponent],
    call_chains: &[WorkflowCallChain],
) -> Option<String> {
    let direct_targets = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .count();
    let component_count = components.len();
    let chain_count = call_chains.len();

    Some(match result.request.intent {
        atlas_core::model::ContextIntent::Review => format!(
            "{} changed node(s), {} component(s), {} call chain(s)",
            direct_targets, component_count, chain_count
        ),
        atlas_core::model::ContextIntent::Impact
        | atlas_core::model::ContextIntent::ImpactAnalysis => format!(
            "Impact reaches {} node(s) across {} component(s)",
            result.nodes.len(),
            component_count
        ),
        atlas_core::model::ContextIntent::UsageLookup => format!(
            "{} usage node(s) surfaced, {} chain(s)",
            result.nodes.len().saturating_sub(direct_targets),
            chain_count
        ),
        _ => format!(
            "{} focused node(s) across {} component(s)",
            result.nodes.len(),
            component_count
        ),
    })
}

fn build_impacted_components(nodes: &[SelectedNode]) -> Vec<WorkflowComponent> {
    let mut component_map: HashMap<(String, String), (usize, usize, HashSet<String>)> =
        HashMap::new();

    for node in nodes {
        let mut identities = vec![component_identity(&node.node)];
        identities.extend(
            component_labels_for_node(&node.node)
                .into_iter()
                .map(|label| ("component_label".to_string(), label)),
        );

        for (kind, label) in identities {
            let entry = component_map
                .entry((kind, label))
                .or_insert_with(|| (0, 0, HashSet::new()));

            if node.selection_reason == SelectionReason::DirectTarget {
                entry.0 += 1;
            } else {
                entry.1 += 1;
            }
            entry.2.insert(node.node.file_path.clone());
        }
    }

    let mut components: Vec<WorkflowComponent> = component_map
        .into_iter()
        .map(
            |((kind, label), (changed, impacted, files))| WorkflowComponent {
                summary: format!(
                    "{} changed, {} impacted across {} file(s)",
                    changed,
                    impacted,
                    files.len()
                ),
                label,
                kind,
                changed_node_count: changed,
                impacted_node_count: impacted,
                file_count: files.len(),
            },
        )
        .collect();

    components.sort_by(|a, b| {
        (b.changed_node_count + b.impacted_node_count)
            .cmp(&(a.changed_node_count + a.impacted_node_count))
            .then_with(|| a.label.cmp(&b.label))
            .then_with(|| a.kind.cmp(&b.kind))
    });
    components.truncate(6);
    components
}

fn component_identity(node: &atlas_core::Node) -> (String, String) {
    let extra = node.extra_json.as_object();
    if let Some(path) = extra
        .and_then(|extra| {
            extra
                .get("owner_manifest_path")
                .or_else(|| extra.get("workspace_manifest_path"))
                .and_then(|value| value.as_str())
        })
        .filter(|path| !path.is_empty())
    {
        return ("package".to_string(), path.to_string());
    }

    let dir = parent_dir(&node.file_path);
    if dir.is_empty() {
        ("file".to_string(), node.file_path.clone())
    } else {
        ("directory".to_string(), dir.to_string())
    }
}

fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(index) => &path[..index],
        None => "",
    }
}

fn component_labels_for_node(node: &atlas_core::Node) -> Vec<String> {
    let path = node.file_path.as_str();
    let name = node.name.as_str();
    let mut labels = Vec::new();
    add_component_label(&mut labels, path.starts_with("packages/atlas-cli/"), "cli");
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-mcp/") || path == "MCP_TOOLS.md",
        "mcp",
    );
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-repo/"),
        "repo_scan",
    );
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-parser/"),
        "parse",
    );
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-store-sqlite/")
            || path.starts_with("packages/atlas-db-utils/"),
        "persist_graph",
    );
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-engine/")
            || path.contains("/update.rs")
            || path.contains("/watch.rs")
            || path.contains("postprocess"),
        "incremental_update",
    );
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-search/")
            || path.contains("query")
            || path.contains("traverse"),
        "search_traverse",
    );
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-review/")
            || path.contains("review")
            || path.ends_with("changes.rs")
            || path.ends_with("context_cmd.rs"),
        "review_context",
    );
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-contextsave/")
            || path.starts_with("packages/atlas-contentstore/"),
        "context_memory",
    );
    add_component_label(
        &mut labels,
        path.starts_with("packages/atlas-session/")
            || path.starts_with("packages/atlas-agent-events/")
            || path.contains("session")
            || path.contains("wake_up"),
        "session_continuity",
    );
    add_component_label(
        &mut labels,
        path.starts_with("schemas/")
            || path.starts_with(".atlas/")
            || path.ends_with(".toml")
            || path.ends_with(".yaml")
            || path.ends_with(".yml")
            || path.ends_with(".json"),
        "config",
    );
    add_component_label(
        &mut labels,
        path.contains("health")
            || path.contains("doctor")
            || path.contains("db_check")
            || path.contains("debug_graph")
            || path.contains("status")
            || name.contains("doctor")
            || name.contains("status"),
        "diagnostics",
    );
    add_component_label(
        &mut labels,
        path.starts_with("tests/")
            || path.contains("/tests/")
            || path.ends_with("_test.rs")
            || path.ends_with("tests.rs"),
        "tests",
    );
    add_component_label(
        &mut labels,
        path.starts_with("docs/") || path.starts_with("wiki/") || path.ends_with(".md"),
        "docs",
    );
    labels.sort();
    labels.dedup();
    labels
}

fn add_component_label(labels: &mut Vec<String>, predicate: bool, label: &str) {
    if predicate {
        labels.push(label.to_string());
    }
}

fn build_call_chains(result: &ContextResult) -> Vec<WorkflowCallChain> {
    use atlas_core::EdgeKind;

    let direct_targets: HashSet<&str> = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .map(|node| node.node.qualified_name.as_str())
        .collect();

    let call_edges: Vec<&SelectedEdge> = result
        .edges
        .iter()
        .filter(|edge| edge.edge.kind == EdgeKind::Calls)
        .collect();

    let mut incoming: HashMap<&str, Vec<&SelectedEdge>> = HashMap::new();
    let mut outgoing: HashMap<&str, Vec<&SelectedEdge>> = HashMap::new();
    for edge in &call_edges {
        incoming
            .entry(edge.edge.target_qn.as_str())
            .or_default()
            .push(*edge);
        outgoing
            .entry(edge.edge.source_qn.as_str())
            .or_default()
            .push(*edge);
    }

    let mut seen = HashSet::new();
    let mut chains: Vec<(f32, WorkflowCallChain)> = Vec::new();

    for target in &direct_targets {
        if let (Some(ins), Some(outs)) = (incoming.get(target), outgoing.get(target)) {
            for inbound in ins.iter().take(3) {
                for outbound in outs.iter().take(3) {
                    let steps = vec![
                        inbound.edge.source_qn.clone(),
                        (*target).to_string(),
                        outbound.edge.target_qn.clone(),
                    ];
                    let key = steps.join(" -> ");
                    if seen.insert(key.clone()) {
                        chains.push((
                            inbound.relevance_score + outbound.relevance_score,
                            WorkflowCallChain {
                                summary: key,
                                steps,
                                edge_kinds: vec![
                                    inbound.edge.kind.as_str().to_string(),
                                    outbound.edge.kind.as_str().to_string(),
                                ],
                            },
                        ));
                    }
                }
            }
        }

        for edge in call_edges
            .iter()
            .filter(|edge| edge.edge.source_qn == *target || edge.edge.target_qn == *target)
        {
            let steps = vec![edge.edge.source_qn.clone(), edge.edge.target_qn.clone()];
            let key = steps.join(" -> ");
            if seen.insert(key.clone()) {
                chains.push((
                    edge.relevance_score,
                    WorkflowCallChain {
                        summary: key,
                        steps,
                        edge_kinds: vec![edge.edge.kind.as_str().to_string()],
                    },
                ));
            }
        }
    }

    chains.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.summary.cmp(&b.1.summary))
    });
    chains.into_iter().map(|(_, chain)| chain).take(5).collect()
}

fn build_ripple_effects(
    result: &ContextResult,
    components: &[WorkflowComponent],
    call_chains: &[WorkflowCallChain],
) -> Vec<String> {
    let mut ripple_effects = Vec::new();

    if components.len() > 1 {
        ripple_effects.push(format!(
            "Impact spans {} components: {}",
            components.len(),
            components
                .iter()
                .take(3)
                .map(|component| component.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if let Some(chain) = call_chains.first() {
        ripple_effects.push(format!("Primary call chain: {}", chain.summary));
    }

    let neighboring_files: HashSet<&str> = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason != SelectionReason::DirectTarget)
        .map(|node| node.node.file_path.as_str())
        .collect();
    if !neighboring_files.is_empty() {
        ripple_effects.push(format!(
            "Change reaches {} neighboring file(s).",
            neighboring_files.len()
        ));
    }

    if ripple_effects.is_empty() {
        ripple_effects.push("Impact remains local to selected nodes.".to_string());
    }

    ripple_effects
}

fn build_noise_reduction_summary(result: &ContextResult) -> NoiseReductionSummary {
    let mut rules_applied = Vec::new();
    if !result.request.include_neighbors {
        rules_applied.push("omitted containment siblings".to_string());
    }
    if !result.request.include_tests {
        rules_applied.push("omitted test-only neighbors".to_string());
    }
    if !result.request.include_imports {
        rules_applied.push("omitted import neighbors".to_string());
    }
    if !result.request.include_callers {
        rules_applied.push("omitted caller expansion".to_string());
    }
    if !result.request.include_callees {
        rules_applied.push("omitted callee expansion".to_string());
    }
    if result.truncation.truncated {
        rules_applied.push("trimmed low-signal nodes and edges to requested caps".to_string());
    }

    NoiseReductionSummary {
        retained_nodes: result.nodes.len(),
        retained_edges: result.edges.len(),
        retained_files: result.files.len(),
        dropped_nodes: result.truncation.nodes_dropped,
        dropped_edges: result.truncation.edges_dropped,
        dropped_files: result.truncation.files_dropped,
        rules_applied,
    }
}
