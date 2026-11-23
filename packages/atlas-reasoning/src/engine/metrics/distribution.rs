use super::types::*;
use super::*;

pub(super) fn metric_distribution(
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

pub(super) fn distribution_value(
    distributions: &[MetricDistribution],
    metric_name: &str,
    selector: impl Fn(&MetricDistribution) -> f64,
) -> Option<f64> {
    distributions
        .iter()
        .find(|distribution| distribution.metric_name == metric_name)
        .map(selector)
}

pub(crate) fn module_id_for_file(file_path: &str, owner_id: Option<&str>) -> String {
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

pub(crate) fn is_callable(node: &Node) -> bool {
    matches!(
        node.kind,
        NodeKind::Function | NodeKind::Method | NodeKind::Test
    )
}

pub(crate) fn is_reference_edge(kind: EdgeKind) -> bool {
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

pub(super) fn average(values: impl Iterator<Item = f64>) -> f64 {
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

pub(super) fn percentile(sorted_values: &[f64], percentile: f64) -> f64 {
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

pub(super) fn build_metric_graph_summary(
    snapshot: &GraphSnapshot,
    node_metrics: &[NodeMetric],
    raw_node_count: usize,
    raw_edge_count: usize,
) -> MetricGraphSummary {
    let node_count = snapshot.nodes.len();
    let edge_count = snapshot.edges.len();
    let density = if node_count > 1 {
        edge_count as f64 / (node_count as f64 * (node_count - 1) as f64)
    } else {
        0.0
    };
    let average_degree = if node_count == 0 {
        0.0
    } else {
        edge_count as f64 / node_count as f64
    };

    let module_by_qname = node_metrics
        .iter()
        .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
        .collect::<HashMap<_, _>>();
    let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
    for edge in &snapshot.edges {
        let (Some(source), Some(target)) = (
            module_by_qname.get(&edge.source_qn),
            module_by_qname.get(&edge.target_qn),
        ) else {
            continue;
        };
        if source != target {
            adjacency
                .entry(source.clone())
                .or_default()
                .insert(target.clone());
        }
    }

    MetricGraphSummary {
        node_count,
        edge_count,
        density,
        average_degree,
        diameter: module_graph_diameter(&adjacency),
        diameter_scope: "module_graph".to_owned(),
        scope: "post_filter".to_owned(),
        raw_node_count,
        raw_edge_count,
    }
}

/// Longest shortest path over the directed module graph. The module graph is
/// small (tens of nodes) whereas the symbol graph is tens of thousands, so this
/// bounded BFS is safe to run per metrics request.
fn module_graph_diameter(adjacency: &BTreeMap<String, BTreeSet<String>>) -> usize {
    let mut diameter = 0usize;
    for start in adjacency.keys() {
        let mut distances = HashMap::<&str, usize>::new();
        let mut queue = VecDeque::<&str>::new();
        distances.insert(start.as_str(), 0);
        queue.push_back(start.as_str());
        while let Some(current) = queue.pop_front() {
            let distance = distances[current];
            let Some(nexts) = adjacency.get(current) else {
                continue;
            };
            for next in nexts {
                if !distances.contains_key(next.as_str()) {
                    distances.insert(next.as_str(), distance + 1);
                    queue.push_back(next.as_str());
                    diameter = diameter.max(distance + 1);
                }
            }
        }
    }
    diameter
}
