use super::distribution::callable_loc;
use super::distribution::metric_distribution;
use super::findings::build_dependency_depths;
use super::findings::build_direct_test_neighbors;
use super::findings::compute_test_adjacency;
use super::types::*;
use super::*;

pub(super) fn is_ignored_node(
    engine: &InsightsEngine<'_>,
    node: &Node,
    owner_id: Option<&str>,
) -> bool {
    let module_id = module_id_for_file(&node.file_path, owner_id);
    path_matches_ignore_list(
        &node.file_path,
        &engine.config().ignore_files,
        engine.ignore_files_glob.as_ref(),
    ) || module_matches_any(&node.qualified_name, &engine.config().ignore_modules)
        || module_matches_any(&module_id, &engine.config().ignore_modules)
        || owner_id.is_some_and(|owner| module_matches_any(owner, &engine.config().ignore_modules))
        || engine
            .config()
            .ignore_node_kinds
            .iter()
            .any(|kind| kind.eq_ignore_ascii_case(node.kind.as_str()))
}

pub(crate) fn build_node_metrics(
    engine: &InsightsEngine<'_>,
    snapshot: &GraphSnapshot,
    rust_complexity: &HashMap<String, RustComplexityMetrics>,
) -> Vec<NodeMetric> {
    let qname_to_node = snapshot
        .nodes
        .iter()
        .map(|node| (node.qualified_name.clone(), node))
        .collect::<HashMap<_, _>>();

    let mut inbound = HashMap::<String, Vec<&Edge>>::new();
    let mut outbound = HashMap::<String, Vec<&Edge>>::new();
    for edge in &snapshot.edges {
        outbound
            .entry(edge.source_qn.clone())
            .or_default()
            .push(edge);
        inbound
            .entry(edge.target_qn.clone())
            .or_default()
            .push(edge);
    }

    let test_nodes_by_file = snapshot
        .nodes
        .iter()
        .filter(|node| node.is_test || node.kind == NodeKind::Test)
        .fold(BTreeMap::<String, Vec<&Node>>::new(), |mut acc, node| {
            acc.entry(node.file_path.clone()).or_default().push(node);
            acc
        });

    let test_neighbors = build_direct_test_neighbors(snapshot, &qname_to_node);
    let dependency_depths = build_dependency_depths(snapshot);

    snapshot
        .nodes
        .iter()
        .map(|node| {
            let module_id = module_id_for_file(
                &node.file_path,
                snapshot
                    .owner_by_file
                    .get(&node.file_path)
                    .and_then(|v| v.as_deref()),
            );
            let fan_in = inbound.get(&node.qualified_name).map_or(0, Vec::len);
            let fan_out = outbound.get(&node.qualified_name).map_or(0, Vec::len);
            let reference_count = inbound
                .get(&node.qualified_name)
                .into_iter()
                .flat_map(|edges| edges.iter())
                .chain(
                    outbound
                        .get(&node.qualified_name)
                        .into_iter()
                        .flat_map(|edges| edges.iter()),
                )
                .filter(|edge| is_reference_edge(edge.kind))
                .count();
            let (linked_test_count, coverage_strength) = compute_test_adjacency(
                node,
                &inbound,
                &test_neighbors,
                test_nodes_by_file.get(&node.file_path),
            );
            let loc = callable_loc(node);
            let large_function_candidate =
                loc.is_some_and(|loc| loc >= engine.config().large_function_loc);
            let complexity = if is_callable(node) {
                rust_complexity
                    .get(&node.qualified_name)
                    .map(|metrics| {
                        (
                            MetricValue::Available(metrics.cyclomatic_complexity),
                            MetricValue::Available(metrics.cognitive_complexity),
                            MetricValue::Available(metrics.branch_count),
                            MetricValue::Available(metrics.max_nesting_depth),
                        )
                    })
                    .unwrap_or((
                        MetricValue::NotAvailable,
                        MetricValue::NotAvailable,
                        MetricValue::NotAvailable,
                        MetricValue::NotAvailable,
                    ))
            } else {
                (
                    MetricValue::NotAvailable,
                    MetricValue::NotAvailable,
                    MetricValue::NotAvailable,
                    MetricValue::NotAvailable,
                )
            };

            let high_complexity_candidate = complexity
                .0
                .copied()
                .is_some_and(|value| value >= engine.config().high_cyclomatic_complexity)
                || complexity
                    .1
                    .copied()
                    .is_some_and(|value| value >= engine.config().high_cognitive_complexity)
                || complexity
                    .2
                    .copied()
                    .is_some_and(|value| value >= engine.config().branch_count)
                || complexity
                    .3
                    .copied()
                    .is_some_and(|value| value >= engine.config().max_nesting_depth);

            NodeMetric {
                node: node.clone(),
                module_id,
                fan_in,
                fan_out,
                dependency_depth: dependency_depths
                    .get(&node.qualified_name)
                    .copied()
                    .unwrap_or_default(),
                reference_count,
                linked_test_count,
                coverage_strength,
                loc,
                large_function_candidate,
                cyclomatic_complexity: complexity.0,
                cognitive_complexity: complexity.1,
                branch_count: complexity.2,
                max_nesting_depth: complexity.3,
                high_complexity_candidate,
            }
        })
        .collect()
}

pub(super) fn build_module_metrics(
    snapshot: &GraphSnapshot,
    node_metrics: &[NodeMetric],
    file_metrics: &[FileMetric],
) -> Vec<ModuleMetric> {
    let node_module_by_qname = node_metrics
        .iter()
        .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
        .collect::<HashMap<_, _>>();
    let mut file_paths_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut node_count_by_module = BTreeMap::<String, usize>::new();
    for metric in node_metrics {
        *node_count_by_module
            .entry(metric.module_id.clone())
            .or_default() += 1;
    }
    for metric in file_metrics {
        file_paths_by_module
            .entry(metric.module_id.clone())
            .or_default()
            .insert(metric.file_path.clone());
    }

    let mut internal = BTreeMap::<String, usize>::new();
    let mut outbound = BTreeMap::<String, usize>::new();
    let mut inbound = BTreeMap::<String, usize>::new();
    for edge in &snapshot.edges {
        let Some(source_module) = node_module_by_qname.get(&edge.source_qn) else {
            continue;
        };
        let Some(target_module) = node_module_by_qname.get(&edge.target_qn) else {
            continue;
        };
        if source_module == target_module {
            *internal.entry(source_module.clone()).or_default() += 1;
        } else {
            *outbound.entry(source_module.clone()).or_default() += 1;
            *inbound.entry(target_module.clone()).or_default() += 1;
        }
    }

    file_paths_by_module
        .into_iter()
        .map(|(module_id, file_paths)| {
            let node_count = node_count_by_module
                .get(&module_id)
                .copied()
                .unwrap_or_default();
            let internal_edge_count = internal.get(&module_id).copied().unwrap_or_default();
            let external_dependency_edge_count =
                outbound.get(&module_id).copied().unwrap_or_default();
            let inbound_dependency_edge_count =
                inbound.get(&module_id).copied().unwrap_or_default();
            let possible_relationships = if node_count <= 1 {
                1.0
            } else {
                (node_count * (node_count - 1)) as f64
            };
            let cohesion = (internal_edge_count as f64 / possible_relationships).min(1.0);
            let coupling_score =
                (external_dependency_edge_count + inbound_dependency_edge_count) as f64;

            ModuleMetric {
                module_id: module_id.clone(),
                file_paths: file_paths.into_iter().collect(),
                node_count,
                internal_edge_count,
                external_dependency_edge_count,
                inbound_dependency_edge_count,
                coupling_score,
                cohesion,
                high_coupling_candidate: false,
            }
        })
        .collect()
}

pub(super) fn build_distributions(
    engine: &InsightsEngine<'_>,
    node_metrics: &[NodeMetric],
    file_metrics: &[FileMetric],
    module_metrics: &[ModuleMetric],
) -> Vec<MetricDistribution> {
    let mut distributions = Vec::new();

    distributions.push(metric_distribution(
        "fan_in",
        node_metrics
            .iter()
            .map(|metric| distribution_sample_for_node(metric, metric.fan_in as f64))
            .collect(),
        Some(engine.config().high_fan_in as f64),
        engine.config().outlier_percentile_cutoff as f64,
    ));
    distributions.push(metric_distribution(
        "fan_out",
        node_metrics
            .iter()
            .map(|metric| distribution_sample_for_node(metric, metric.fan_out as f64))
            .collect(),
        Some(engine.config().high_fan_out as f64),
        engine.config().outlier_percentile_cutoff as f64,
    ));
    distributions.push(metric_distribution(
        "loc",
        node_metrics
            .iter()
            .filter_map(|metric| {
                metric
                    .loc
                    .map(|loc| distribution_sample_for_node(metric, loc as f64))
            })
            .collect(),
        Some(engine.config().large_function_loc as f64),
        engine.config().outlier_percentile_cutoff as f64,
    ));
    distributions.push(metric_distribution(
        "cyclomatic_complexity",
        node_metrics
            .iter()
            .filter_map(|metric| {
                metric
                    .cyclomatic_complexity
                    .copied()
                    .map(|value| distribution_sample_for_node(metric, value as f64))
            })
            .collect(),
        Some(engine.config().high_cyclomatic_complexity as f64),
        engine.config().outlier_percentile_cutoff as f64,
    ));
    distributions.push(metric_distribution(
        "cognitive_complexity",
        node_metrics
            .iter()
            .filter_map(|metric| {
                metric
                    .cognitive_complexity
                    .copied()
                    .map(|value| distribution_sample_for_node(metric, value as f64))
            })
            .collect(),
        Some(engine.config().high_cognitive_complexity as f64),
        engine.config().outlier_percentile_cutoff as f64,
    ));
    distributions.push(metric_distribution(
        "max_nesting_depth",
        node_metrics
            .iter()
            .filter_map(|metric| {
                metric
                    .max_nesting_depth
                    .copied()
                    .map(|value| distribution_sample_for_node(metric, value as f64))
            })
            .collect(),
        Some(engine.config().max_nesting_depth as f64),
        engine.config().outlier_percentile_cutoff as f64,
    ));
    distributions.push(metric_distribution(
        "branch_count",
        node_metrics
            .iter()
            .filter_map(|metric| {
                metric
                    .branch_count
                    .copied()
                    .map(|value| distribution_sample_for_node(metric, value as f64))
            })
            .collect(),
        Some(engine.config().branch_count as f64),
        engine.config().outlier_percentile_cutoff as f64,
    ));
    distributions.push(metric_distribution(
        "file_node_count",
        file_metrics
            .iter()
            .map(|metric| DistributionSample {
                subject_id: metric.file_path.clone(),
                file_path: Some(metric.file_path.clone()),
                qualified_name: None,
                value: metric.node_count as f64,
            })
            .collect(),
        None,
        engine.config().outlier_percentile_cutoff as f64,
    ));
    distributions.push(metric_distribution(
        "coupling",
        module_metrics
            .iter()
            .map(|metric| DistributionSample {
                subject_id: metric.module_id.clone(),
                file_path: metric.file_paths.first().cloned(),
                qualified_name: None,
                value: metric.coupling_score,
            })
            .collect(),
        Some(engine.config().high_coupling as f64),
        engine.config().outlier_percentile_cutoff as f64,
    ));

    distributions.sort_by(|left, right| left.metric_name.cmp(&right.metric_name));
    distributions
}

pub(super) fn distribution_sample_for_node(metric: &NodeMetric, value: f64) -> DistributionSample {
    DistributionSample {
        subject_id: metric.node.qualified_name.clone(),
        file_path: Some(metric.node.file_path.clone()),
        qualified_name: Some(metric.node.qualified_name.clone()),
        value,
    }
}
