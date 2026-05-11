use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use atlas_core::{
    AtlasError, CoverageStrength, Edge, EdgeKind, InsightEvidence, InsightFinding,
    InsightLineRange, InsightSeverity, MetricsReport, Node, NodeKind, Result,
};
use tree_sitter::Node as TsNode;

use super::InsightsEngine;
use super::insights::{module_matches_any, path_matches_any};

#[derive(Debug, Clone, PartialEq)]
pub enum MetricValue<T> {
    Available(T),
    NotAvailable,
}

impl<T> MetricValue<T> {
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Available(value) => Some(value),
            Self::NotAvailable => None,
        }
    }
}

impl<T: Copy> MetricValue<T> {
    pub fn copied(&self) -> Option<T> {
        self.as_ref().copied()
    }
}

#[derive(Debug, Clone)]
pub struct NodeMetric {
    pub node: Node,
    pub module_id: String,
    pub fan_in: usize,
    pub fan_out: usize,
    pub dependency_depth: usize,
    pub reference_count: usize,
    pub linked_test_count: usize,
    pub coverage_strength: CoverageStrength,
    pub loc: Option<usize>,
    pub large_function_candidate: bool,
    pub cyclomatic_complexity: MetricValue<usize>,
    pub cognitive_complexity: MetricValue<usize>,
    pub branch_count: MetricValue<usize>,
    pub max_nesting_depth: MetricValue<usize>,
    pub high_complexity_candidate: bool,
}

#[derive(Debug, Clone)]
pub struct FileMetric {
    pub file_path: String,
    pub module_id: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub average_fan_in: f64,
    pub average_fan_out: f64,
    pub import_count: usize,
    pub test_coverage_ratio: MetricValue<f64>,
    pub highly_connected: bool,
}

#[derive(Debug, Clone)]
pub struct ModuleMetric {
    pub module_id: String,
    pub file_paths: Vec<String>,
    pub node_count: usize,
    pub internal_edge_count: usize,
    pub external_dependency_edge_count: usize,
    pub inbound_dependency_edge_count: usize,
    pub coupling_score: f64,
    pub cohesion: f64,
    pub high_coupling_candidate: bool,
}

#[derive(Debug, Clone)]
pub struct MetricOutlier {
    pub subject_id: String,
    pub file_path: Option<String>,
    pub qualified_name: Option<String>,
    pub value: f64,
}

#[derive(Debug, Clone)]
pub struct MetricDistribution {
    pub metric_name: String,
    pub min: f64,
    pub max: f64,
    pub average: f64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub threshold_value: Option<f64>,
    pub outlier_cutoff: f64,
    pub outliers: Vec<MetricOutlier>,
}

#[derive(Debug, Clone)]
pub struct CodeHealthMetrics {
    pub node_metrics: Vec<NodeMetric>,
    pub file_metrics: Vec<FileMetric>,
    pub module_metrics: Vec<ModuleMetric>,
    pub distributions: Vec<MetricDistribution>,
}

#[derive(Debug, Clone)]
pub struct MetricsAnalysis {
    pub metrics: CodeHealthMetrics,
    pub report: MetricsReport,
}

#[derive(Debug, Clone)]
pub(super) struct GraphSnapshot {
    pub(super) nodes: Vec<Node>,
    pub(super) edges: Vec<Edge>,
    pub(super) owner_by_file: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RustComplexityMetrics {
    cyclomatic_complexity: usize,
    cognitive_complexity: usize,
    branch_count: usize,
    max_nesting_depth: usize,
}

#[derive(Debug, Clone)]
struct DistributionSample {
    subject_id: String,
    file_path: Option<String>,
    qualified_name: Option<String>,
    value: f64,
}

impl<'s> InsightsEngine<'s> {
    pub fn analyze_metrics(&self, repo_root: impl AsRef<Path>) -> Result<MetricsAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other("metrics analysis requires a store-backed insights engine".to_owned())
        })?;
        let snapshot = self.load_graph_snapshot(store)?;
        let rust_complexity = load_rust_complexity(repo_root.as_ref(), &snapshot.nodes)?;
        let mut node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);
        let mut file_metrics = build_file_metrics(&snapshot, &node_metrics);
        let mut module_metrics = build_module_metrics(&snapshot, &node_metrics, &file_metrics);
        let distributions =
            build_distributions(self, &node_metrics, &file_metrics, &module_metrics);

        let file_node_count_p90 = distribution_value(&distributions, "file_node_count", |d| d.p90);
        for metric in &mut file_metrics {
            let average_connectivity = metric.average_fan_in + metric.average_fan_out;
            metric.highly_connected = average_connectivity >= self.config().high_coupling as f64
                || file_node_count_p90
                    .is_some_and(|p90| p90 > 0.0 && metric.node_count as f64 >= p90);
        }

        for metric in &mut module_metrics {
            metric.high_coupling_candidate =
                metric.coupling_score >= self.config().high_coupling as f64;
        }

        node_metrics.sort_by(|left, right| {
            left.node
                .file_path
                .cmp(&right.node.file_path)
                .then_with(|| left.node.line_start.cmp(&right.node.line_start))
                .then_with(|| left.node.qualified_name.cmp(&right.node.qualified_name))
        });
        file_metrics.sort_by(|left, right| left.file_path.cmp(&right.file_path));
        module_metrics.sort_by(|left, right| left.module_id.cmp(&right.module_id));

        let findings = build_metric_findings(
            self,
            &node_metrics,
            &file_metrics,
            &module_metrics,
            &distributions,
        );

        Ok(MetricsAnalysis {
            metrics: CodeHealthMetrics {
                node_metrics,
                file_metrics,
                module_metrics,
                distributions,
            },
            report: self.metrics_report(findings),
        })
    }

    pub(super) fn load_graph_snapshot(
        &self,
        store: &'s atlas_store_sqlite::Store,
    ) -> Result<GraphSnapshot> {
        let mut file_paths = store.file_hashes()?.into_keys().collect::<Vec<_>>();
        file_paths.sort();

        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut owner_by_file = BTreeMap::new();
        for file_path in file_paths {
            let owner_id = store.file_owner_id(&file_path)?;
            owner_by_file.insert(file_path.clone(), owner_id);
            nodes.extend(store.nodes_by_file(&file_path)?);
            edges.extend(store.edges_by_file(&file_path)?);
        }

        let allowed_nodes = nodes
            .into_iter()
            .filter(|node| {
                !is_ignored_node(
                    self,
                    node,
                    owner_by_file
                        .get(&node.file_path)
                        .and_then(|v| v.as_deref()),
                )
            })
            .collect::<Vec<_>>();
        let allowed_qnames = allowed_nodes
            .iter()
            .map(|node| node.qualified_name.clone())
            .collect::<HashSet<_>>();
        let allowed_files = allowed_nodes
            .iter()
            .map(|node| node.file_path.clone())
            .collect::<HashSet<_>>();
        let filtered_edges = edges
            .into_iter()
            .filter(|edge| {
                allowed_files.contains(&edge.file_path)
                    && allowed_qnames.contains(&edge.source_qn)
                    && allowed_qnames.contains(&edge.target_qn)
            })
            .collect::<Vec<_>>();
        let filtered_owner_by_file = owner_by_file
            .into_iter()
            .filter(|(file_path, _)| allowed_files.contains(file_path))
            .collect();

        Ok(GraphSnapshot {
            nodes: allowed_nodes,
            edges: filtered_edges,
            owner_by_file: filtered_owner_by_file,
        })
    }
}

pub(super) fn is_ignored_node(
    engine: &InsightsEngine<'_>,
    node: &Node,
    owner_id: Option<&str>,
) -> bool {
    let module_id = module_id_for_file(&node.file_path, owner_id);
    path_matches_any(&node.file_path, &engine.config().ignore_files)
        || module_matches_any(&node.qualified_name, &engine.config().ignore_modules)
        || module_matches_any(&module_id, &engine.config().ignore_modules)
        || owner_id.is_some_and(|owner| module_matches_any(owner, &engine.config().ignore_modules))
        || engine
            .config()
            .ignore_node_kinds
            .iter()
            .any(|kind| kind.eq_ignore_ascii_case(node.kind.as_str()))
}

pub(super) fn build_node_metrics(
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

fn build_file_metrics(snapshot: &GraphSnapshot, node_metrics: &[NodeMetric]) -> Vec<FileMetric> {
    let nodes_by_file = node_metrics.iter().fold(
        BTreeMap::<String, Vec<&NodeMetric>>::new(),
        |mut acc, metric| {
            acc.entry(metric.node.file_path.clone())
                .or_default()
                .push(metric);
            acc
        },
    );
    let edges_by_file =
        snapshot
            .edges
            .iter()
            .fold(BTreeMap::<String, Vec<&Edge>>::new(), |mut acc, edge| {
                acc.entry(edge.file_path.clone()).or_default().push(edge);
                acc
            });

    nodes_by_file
        .into_iter()
        .map(|(file_path, metrics)| {
            let edge_count = edges_by_file.get(&file_path).map_or(0, |edges| edges.len());
            let import_count = edges_by_file.get(&file_path).map_or(0, |edges| {
                edges
                    .iter()
                    .filter(|edge| edge.kind == EdgeKind::Imports)
                    .count()
            });
            let average_fan_in = average(metrics.iter().map(|metric| metric.fan_in as f64));
            let average_fan_out = average(metrics.iter().map(|metric| metric.fan_out as f64));
            let test_nodes = metrics
                .iter()
                .filter(|metric| metric.node.is_test || metric.node.kind == NodeKind::Test)
                .count();
            let non_test_callable_nodes = metrics
                .iter()
                .filter(|metric| {
                    is_callable(&metric.node)
                        && !metric.node.is_test
                        && metric.node.kind != NodeKind::Test
                })
                .count();
            let test_coverage_ratio = if non_test_callable_nodes == 0 {
                MetricValue::NotAvailable
            } else {
                MetricValue::Available(test_nodes as f64 / non_test_callable_nodes as f64)
            };

            FileMetric {
                module_id: metrics[0].module_id.clone(),
                file_path,
                node_count: metrics.len(),
                edge_count,
                average_fan_in,
                average_fan_out,
                import_count,
                test_coverage_ratio,
                highly_connected: false,
            }
        })
        .collect()
}

fn build_module_metrics(
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

fn build_distributions(
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

fn distribution_sample_for_node(metric: &NodeMetric, value: f64) -> DistributionSample {
    DistributionSample {
        subject_id: metric.node.qualified_name.clone(),
        file_path: Some(metric.node.file_path.clone()),
        qualified_name: Some(metric.node.qualified_name.clone()),
        value,
    }
}

fn build_metric_findings(
    engine: &InsightsEngine<'_>,
    node_metrics: &[NodeMetric],
    file_metrics: &[FileMetric],
    module_metrics: &[ModuleMetric],
    distributions: &[MetricDistribution],
) -> Vec<InsightFinding> {
    let mut findings = Vec::new();

    for metric in node_metrics {
        if metric.fan_in >= engine.config().high_fan_in {
            findings.push(node_metric_finding(
                metric,
                "fan_in",
                metric.fan_in as f64,
                engine.config().high_fan_in as f64,
                "high fan-in",
            ));
        }
        if metric.fan_out >= engine.config().high_fan_out {
            findings.push(node_metric_finding(
                metric,
                "fan_out",
                metric.fan_out as f64,
                engine.config().high_fan_out as f64,
                "high fan-out",
            ));
        }
        if metric.large_function_candidate {
            findings.push(node_metric_finding(
                metric,
                "loc",
                metric.loc.unwrap_or_default() as f64,
                engine.config().large_function_loc as f64,
                "large function",
            ));
        }
        for (metric_name, raw_value, threshold, title) in [
            (
                "cyclomatic_complexity",
                metric.cyclomatic_complexity.copied(),
                engine.config().high_cyclomatic_complexity as f64,
                "high cyclomatic complexity",
            ),
            (
                "cognitive_complexity",
                metric.cognitive_complexity.copied(),
                engine.config().high_cognitive_complexity as f64,
                "high cognitive complexity",
            ),
            (
                "branch_count",
                metric.branch_count.copied(),
                engine.config().branch_count as f64,
                "high branch count",
            ),
            (
                "max_nesting_depth",
                metric.max_nesting_depth.copied(),
                engine.config().max_nesting_depth as f64,
                "deep nesting",
            ),
        ] {
            if let Some(raw_value) = raw_value
                && raw_value as f64 >= threshold
            {
                findings.push(node_metric_finding(
                    metric,
                    metric_name,
                    raw_value as f64,
                    threshold,
                    title,
                ));
            }
        }
    }

    for metric in file_metrics.iter().filter(|metric| metric.highly_connected) {
        let raw_value = metric.average_fan_in + metric.average_fan_out;
        let threshold = engine.config().high_coupling as f64;
        findings.push(InsightFinding {
            id: format!("metrics:file:{}", metric.file_path),
            title: format!("highly connected file {}", metric.file_path),
            severity: severity_for_ratio(raw_value, threshold.max(1.0)),
            category: "file_metrics".to_owned(),
            message: format!(
                "file {} has {} nodes, {} edges, connectivity {:.2}, threshold {:.2}",
                metric.file_path, metric.node_count, metric.edge_count, raw_value, threshold
            ),
            evidence: vec![InsightEvidence {
                file_path: Some(metric.file_path.clone()),
                qualified_name: None,
                node_kind: None,
                edge_kind: None,
                line_range: None,
                confidence_tier: None,
            }],
            ranking_reason: format!(
                "file metric connectivity {:.2} exceeded threshold {:.2}",
                raw_value, threshold
            ),
            details: None,
            score: raw_value,
        });
    }

    for metric in module_metrics
        .iter()
        .filter(|metric| metric.high_coupling_candidate)
    {
        findings.push(InsightFinding {
            id: format!("metrics:module:{}", metric.module_id),
            title: format!("high coupling module {}", metric.module_id),
            severity: severity_for_ratio(
                metric.coupling_score,
                engine.config().high_coupling.max(1) as f64,
            ),
            category: "module_metrics".to_owned(),
            message: format!(
                "module {} has coupling {:.2} (outbound {}, inbound {}), threshold {:.2}",
                metric.module_id,
                metric.coupling_score,
                metric.external_dependency_edge_count,
                metric.inbound_dependency_edge_count,
                engine.config().high_coupling as f64,
            ),
            evidence: metric
                .file_paths
                .first()
                .cloned()
                .into_iter()
                .map(|file_path| InsightEvidence {
                    file_path: Some(file_path),
                    qualified_name: None,
                    node_kind: None,
                    edge_kind: None,
                    line_range: None,
                    confidence_tier: None,
                })
                .collect(),
            ranking_reason: format!(
                "module coupling {:.2} exceeded threshold {:.2}",
                metric.coupling_score,
                engine.config().high_coupling as f64,
            ),
            details: None,
            score: metric.coupling_score,
        });
    }

    for distribution in distributions {
        for outlier in &distribution.outliers {
            findings.push(InsightFinding {
                id: format!(
                    "metrics:distribution:{}:{}",
                    distribution.metric_name, outlier.subject_id
                ),
                title: format!("{} outlier", distribution.metric_name),
                severity: severity_for_ratio(outlier.value, distribution.outlier_cutoff.max(1.0)),
                category: "distribution".to_owned(),
                message: format!(
                    "metric {} value {:.2} exceeded outlier cutoff {:.2}",
                    distribution.metric_name, outlier.value, distribution.outlier_cutoff
                ),
                evidence: vec![InsightEvidence {
                    file_path: outlier.file_path.clone(),
                    qualified_name: outlier.qualified_name.clone(),
                    node_kind: None,
                    edge_kind: None,
                    line_range: None,
                    confidence_tier: None,
                }],
                ranking_reason: format!(
                    "distribution {} cutoff {:.2}, raw {:.2}",
                    distribution.metric_name, distribution.outlier_cutoff, outlier.value
                ),
                details: None,
                score: outlier.value,
            });
        }
    }

    findings
}

fn node_metric_finding(
    metric: &NodeMetric,
    metric_name: &str,
    raw_value: f64,
    threshold: f64,
    title: &str,
) -> InsightFinding {
    InsightFinding {
        id: format!("metrics:{metric_name}:{}", metric.node.qualified_name),
        title: format!("{title} {}", metric.node.name),
        severity: severity_for_ratio(raw_value, threshold.max(1.0)),
        category: "node_metrics".to_owned(),
        message: format!(
            "metric {} for {} is {:.2}; threshold {:.2}",
            metric_name, metric.node.qualified_name, raw_value, threshold
        ),
        evidence: vec![InsightEvidence {
            file_path: Some(metric.node.file_path.clone()),
            qualified_name: Some(metric.node.qualified_name.clone()),
            node_kind: Some(metric.node.kind.as_str().to_owned()),
            edge_kind: None,
            line_range: Some(InsightLineRange {
                start_line: metric.node.line_start,
                end_line: metric.node.line_end,
            }),
            confidence_tier: None,
        }],
        ranking_reason: format!(
            "raw value {:.2} exceeded threshold {:.2} for {}",
            raw_value, threshold, metric_name
        ),
        details: None,
        score: raw_value,
    }
}

fn severity_for_ratio(raw_value: f64, threshold: f64) -> InsightSeverity {
    if raw_value >= threshold * 2.0 {
        InsightSeverity::High
    } else if raw_value >= threshold {
        InsightSeverity::Medium
    } else {
        InsightSeverity::Low
    }
}

fn compute_test_adjacency(
    node: &Node,
    inbound: &HashMap<String, Vec<&Edge>>,
    test_neighbors: &HashMap<String, BTreeSet<String>>,
    file_tests: Option<&Vec<&Node>>,
) -> (usize, CoverageStrength) {
    if let Some(linked) = test_neighbors.get(&node.qualified_name)
        && !linked.is_empty()
    {
        return (linked.len(), CoverageStrength::Direct);
    }

    let caller_tests = inbound
        .get(&node.qualified_name)
        .into_iter()
        .flat_map(|edges| edges.iter())
        .filter_map(|edge| test_neighbors.get(&edge.source_qn))
        .flat_map(|tests| tests.iter().cloned())
        .collect::<BTreeSet<_>>();
    if !caller_tests.is_empty() {
        return (caller_tests.len(), CoverageStrength::IndirectThroughCallers);
    }

    if let Some(file_tests) = file_tests
        && !file_tests.is_empty()
    {
        return (file_tests.len(), CoverageStrength::SameFile);
    }

    (0, CoverageStrength::None)
}

fn build_direct_test_neighbors(
    snapshot: &GraphSnapshot,
    qname_to_node: &HashMap<String, &Node>,
) -> HashMap<String, BTreeSet<String>> {
    let mut neighbors = HashMap::<String, BTreeSet<String>>::new();
    for edge in &snapshot.edges {
        if !matches!(edge.kind, EdgeKind::Tests | EdgeKind::TestedBy) {
            continue;
        }
        let Some(source) = qname_to_node.get(&edge.source_qn) else {
            continue;
        };
        let Some(target) = qname_to_node.get(&edge.target_qn) else {
            continue;
        };
        if source.is_test || source.kind == NodeKind::Test {
            neighbors
                .entry(target.qualified_name.clone())
                .or_default()
                .insert(source.qualified_name.clone());
        }
        if target.is_test || target.kind == NodeKind::Test {
            neighbors
                .entry(source.qualified_name.clone())
                .or_default()
                .insert(target.qualified_name.clone());
        }
    }
    neighbors
}

fn build_dependency_depths(snapshot: &GraphSnapshot) -> HashMap<String, usize> {
    let adjacency =
        snapshot
            .edges
            .iter()
            .fold(HashMap::<String, Vec<String>>::new(), |mut acc, edge| {
                acc.entry(edge.source_qn.clone())
                    .or_default()
                    .push(edge.target_qn.clone());
                acc
            });

    let mut memo = HashMap::<String, usize>::new();
    let max_depth = snapshot.nodes.len();
    snapshot
        .nodes
        .iter()
        .map(|node| {
            let depth = dependency_depth(
                &node.qualified_name,
                &adjacency,
                &mut HashSet::new(),
                &mut memo,
                max_depth,
            );
            (node.qualified_name.clone(), depth)
        })
        .collect()
}

fn dependency_depth(
    qname: &str,
    adjacency: &HashMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
    memo: &mut HashMap<String, usize>,
    remaining_budget: usize,
) -> usize {
    if remaining_budget == 0 {
        return 0;
    }
    if let Some(value) = memo.get(qname) {
        return *value;
    }
    if !visiting.insert(qname.to_owned()) {
        return 0;
    }

    let mut depth = 0;
    if let Some(targets) = adjacency.get(qname) {
        for target in targets {
            if target == qname || visiting.contains(target.as_str()) {
                continue;
            }
            depth = depth
                .max(1 + dependency_depth(target, adjacency, visiting, memo, remaining_budget - 1));
        }
    }

    visiting.remove(qname);
    memo.insert(qname.to_owned(), depth);
    depth
}

pub(super) fn load_rust_complexity(
    repo_root: &Path,
    nodes: &[Node],
) -> Result<HashMap<String, RustComplexityMetrics>> {
    let rust_callables = nodes
        .iter()
        .filter(|node| is_callable(node) && node.language.eq_ignore_ascii_case("rust"))
        .fold(BTreeMap::<String, Vec<&Node>>::new(), |mut acc, node| {
            acc.entry(node.file_path.clone()).or_default().push(node);
            acc
        });

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|err| AtlasError::Other(format!("failed to load rust grammar: {err}")))?;

    let mut metrics = HashMap::new();
    for (file_path, file_nodes) in rust_callables {
        let source_path = repo_root.join(&file_path);
        let Ok(source) = fs::read(&source_path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut function_metrics = HashMap::<(String, u32), RustComplexityMetrics>::new();
        collect_rust_function_metrics(tree.root_node(), &source, &mut function_metrics);

        for node in file_nodes {
            if let Some(metric) = function_metrics.get(&(node.name.clone(), node.line_start)) {
                metrics.insert(node.qualified_name.clone(), *metric);
            }
        }
    }

    Ok(metrics)
}

fn collect_rust_function_metrics(
    node: TsNode<'_>,
    source: &[u8],
    out: &mut HashMap<(String, u32), RustComplexityMetrics>,
) {
    if node.kind() == "function_item"
        && let Some(name) = child_field_text(node, "name", source)
    {
        out.insert(
            (name.to_owned(), start_line(node)),
            analyze_rust_function(node, source),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_function_metrics(child, source, out);
    }
}

fn analyze_rust_function(node: TsNode<'_>, source: &[u8]) -> RustComplexityMetrics {
    let mut metrics = RustComplexityMetrics {
        cyclomatic_complexity: 1,
        cognitive_complexity: 0,
        branch_count: 0,
        max_nesting_depth: 0,
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rust_complexity(child, source, 0, &mut metrics);
    }

    metrics
}

fn walk_rust_complexity(
    node: TsNode<'_>,
    source: &[u8],
    nesting_depth: usize,
    metrics: &mut RustComplexityMetrics,
) {
    let kind = node.kind();
    let next_nesting = if is_nesting_node(node) {
        let depth = nesting_depth + 1;
        metrics.max_nesting_depth = metrics.max_nesting_depth.max(depth);
        depth
    } else {
        nesting_depth
    };

    match kind {
        "if_expression" | "while_expression" | "for_expression" | "loop_expression" => {
            metrics.cyclomatic_complexity += 1;
            metrics.cognitive_complexity += 1 + nesting_depth;
            metrics.branch_count += 1;
        }
        "match_expression" => {
            let arms = count_children_of_kind(node, "match_arm").max(1);
            metrics.cyclomatic_complexity += arms;
            metrics.cognitive_complexity += 1 + nesting_depth;
            metrics.branch_count += arms;
        }
        "return_expression" => {
            metrics.cognitive_complexity += 1 + nesting_depth;
            metrics.branch_count += 1;
        }
        "binary_expression" => {
            let boolean_branches = count_boolean_operators(node);
            metrics.cyclomatic_complexity += boolean_branches;
            metrics.cognitive_complexity += boolean_branches;
            metrics.branch_count += boolean_branches;
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rust_complexity(child, source, next_nesting, metrics);
    }

    let _ = source;
}

fn count_boolean_operators(node: TsNode<'_>) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "&&" | "||") {
            count += 1;
        }
    }
    count
}

fn count_children_of_kind(node: TsNode<'_>, kind: &str) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            count += 1;
        }
    }
    count
}

fn is_nesting_node(node: TsNode<'_>) -> bool {
    match node.kind() {
        "if_expression" | "while_expression" | "for_expression" | "loop_expression"
        | "match_expression" | "closure_expression" => true,
        "block" => !node.parent().is_some_and(|parent| {
            matches!(
                parent.kind(),
                "function_item"
                    | "if_expression"
                    | "else_clause"
                    | "while_expression"
                    | "for_expression"
                    | "loop_expression"
                    | "match_expression"
                    | "match_arm"
                    | "closure_expression"
            )
        }),
        _ => false,
    }
}

fn child_field_text<'a>(node: TsNode<'_>, field: &str, source: &'a [u8]) -> Option<&'a str> {
    node.child_by_field_name(field)
        .and_then(|child| std::str::from_utf8(&source[child.start_byte()..child.end_byte()]).ok())
}

fn start_line(node: TsNode<'_>) -> u32 {
    node.start_position().row as u32 + 1
}

fn metric_distribution(
    metric_name: &str,
    samples: Vec<DistributionSample>,
    threshold_value: Option<f64>,
    outlier_percentile_cutoff: f64,
) -> MetricDistribution {
    if samples.is_empty() {
        return MetricDistribution {
            metric_name: metric_name.to_owned(),
            min: 0.0,
            max: 0.0,
            average: 0.0,
            p50: 0.0,
            p90: 0.0,
            p95: 0.0,
            threshold_value,
            outlier_cutoff: threshold_value.unwrap_or(0.0),
            outliers: Vec::new(),
        };
    }

    let mut sorted_values = samples
        .iter()
        .map(|sample| sample.value)
        .collect::<Vec<_>>();
    sorted_values.sort_by(|left, right| left.total_cmp(right));

    let min = *sorted_values.first().unwrap_or(&0.0);
    let max = *sorted_values.last().unwrap_or(&0.0);
    let average = average(sorted_values.iter().copied());
    let p50 = percentile(&sorted_values, 50.0);
    let p90 = percentile(&sorted_values, 90.0);
    let p95 = percentile(&sorted_values, 95.0);
    let percentile_cutoff = percentile(&sorted_values, outlier_percentile_cutoff);
    let outlier_cutoff = threshold_value.map_or(percentile_cutoff, |threshold| {
        threshold.max(percentile_cutoff)
    });
    let mut outliers = samples
        .into_iter()
        .filter(|sample| sample.value >= outlier_cutoff)
        .map(|sample| MetricOutlier {
            subject_id: sample.subject_id,
            file_path: sample.file_path,
            qualified_name: sample.qualified_name,
            value: sample.value,
        })
        .collect::<Vec<_>>();
    outliers.sort_by(|left, right| {
        right
            .value
            .total_cmp(&left.value)
            .then_with(|| left.subject_id.cmp(&right.subject_id))
    });

    MetricDistribution {
        metric_name: metric_name.to_owned(),
        min,
        max,
        average,
        p50,
        p90,
        p95,
        threshold_value,
        outlier_cutoff,
        outliers,
    }
}

fn distribution_value(
    distributions: &[MetricDistribution],
    metric_name: &str,
    selector: impl Fn(&MetricDistribution) -> f64,
) -> Option<f64> {
    distributions
        .iter()
        .find(|distribution| distribution.metric_name == metric_name)
        .map(selector)
}

pub(super) fn module_id_for_file(file_path: &str, owner_id: Option<&str>) -> String {
    if let Some(owner_id) = owner_id {
        return owner_id.to_owned();
    }

    match file_path.rsplit_once('/') {
        Some((prefix, _)) if !prefix.is_empty() => format!("module:{prefix}"),
        _ => "module:<root>".to_owned(),
    }
}

pub(super) fn callable_loc(node: &Node) -> Option<usize> {
    if !is_callable(node) {
        return None;
    }
    Some(node.line_end.saturating_sub(node.line_start) as usize + 1)
}

pub(super) fn is_callable(node: &Node) -> bool {
    matches!(
        node.kind,
        NodeKind::Function | NodeKind::Method | NodeKind::Test
    )
}

pub(super) fn is_reference_edge(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Calls
            | EdgeKind::Imports
            | EdgeKind::References
            | EdgeKind::Extends
            | EdgeKind::Implements
            | EdgeKind::Tests
            | EdgeKind::TestedBy
    )
}

fn average(values: impl Iterator<Item = f64>) -> f64 {
    let mut total = 0.0;
    let mut count = 0usize;
    for value in values {
        total += value;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

fn percentile(sorted_values: &[f64], percentile: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    if sorted_values.len() == 1 {
        return sorted_values[0];
    }

    let rank = (percentile / 100.0) * (sorted_values.len() - 1) as f64;
    let lower_index = rank.floor() as usize;
    let upper_index = rank.ceil() as usize;
    if lower_index == upper_index {
        sorted_values[lower_index]
    } else {
        let weight = rank - lower_index as f64;
        sorted_values[lower_index] * (1.0 - weight) + sorted_values[upper_index] * weight
    }
}
