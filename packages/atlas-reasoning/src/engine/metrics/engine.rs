use super::distribution::average;
use super::distribution::build_metric_graph_summary;
use super::distribution::distribution_value;
use super::findings::build_metric_findings;
use super::nodes::build_distributions;
use super::nodes::build_module_metrics;
use super::nodes::build_node_metrics;
use super::nodes::is_ignored_node;
use super::types::*;
use super::*;

impl<'s> InsightsEngine<'s> {
    pub fn analyze_metrics(&self, repo_root: impl AsRef<Path>) -> Result<MetricsAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other("metrics analysis requires a store-backed insights engine".to_owned())
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
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
        let graph_stats = store.stats()?;
        let graph_summary = build_metric_graph_summary(
            &snapshot,
            &node_metrics,
            graph_stats.node_count.max(0) as usize,
            graph_stats.edge_count.max(0) as usize,
        );

        Ok(MetricsAnalysis {
            metrics: CodeHealthMetrics {
                node_metrics,
                file_metrics,
                module_metrics,
                distributions,
                graph_summary,
            },
            report: self.metrics_report(findings),
        })
    }

    pub(crate) fn load_graph_snapshot(
        &self,
        store: &'s atlas_store_sqlite::Store,
        repo_root: impl AsRef<Path>,
    ) -> Result<GraphSnapshot> {
        // Build state is keyed by the git-resolved repo root, which resolves
        // symlinks (for example macOS `/var` -> `/private/var`). Canonicalize
        // caller input so both keys agree; fall back to the raw path when it
        // cannot be resolved (for example a not-yet-existing root in tests).
        let repo_root = Utf8Path::from_path(repo_root.as_ref()).ok_or_else(|| {
            AtlasError::Other("reasoning repo root is not valid UTF-8".to_owned())
        })?;
        let repo_root =
            canonical_filesystem_path(repo_root).unwrap_or_else(|_| repo_root.to_owned());
        let source_repo_id = match store.get_build_status(repo_root.as_str())? {
            Some(status) => status.source_repo_id,
            None => match store.single_source_repo_id()? {
                Some(source_repo_id) => source_repo_id,
                None => stable_repo_id(repo_root.as_path()),
            },
        };
        let file_hash_by_file = store
            .file_hashes_for_repo(&source_repo_id)?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let mut file_paths = file_hash_by_file.keys().cloned().collect::<Vec<_>>();
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

        let filtered_file_hash_by_file = file_hash_by_file
            .into_iter()
            .filter(|(file_path, _)| allowed_files.contains(file_path))
            .collect();

        Ok(GraphSnapshot {
            nodes: allowed_nodes,
            edges: filtered_edges,
            owner_by_file: filtered_owner_by_file,
            file_hash_by_file: filtered_file_hash_by_file,
        })
    }
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
