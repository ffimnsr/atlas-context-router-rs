use super::*;

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

#[derive(Debug, Clone, PartialEq)]
pub struct MetricGraphSummary {
    pub node_count: usize,
    pub edge_count: usize,
    pub density: f64,
    pub average_degree: f64,
    pub diameter: usize,
    pub diameter_scope: String,
    /// Scope qualifier for `node_count`/`edge_count`.
    ///
    /// Metrics apply the `insights.ignore_files` / `ignore_modules` /
    /// `ignore_node_kinds` filters, so these counts are post-filter. Raw graph
    /// counts are reported separately to keep the two scopes explicit.
    pub scope: String,
    pub raw_node_count: usize,
    pub raw_edge_count: usize,
}

#[derive(Debug, Clone)]
pub struct CodeHealthMetrics {
    pub node_metrics: Vec<NodeMetric>,
    pub file_metrics: Vec<FileMetric>,
    pub module_metrics: Vec<ModuleMetric>,
    pub distributions: Vec<MetricDistribution>,
    pub graph_summary: MetricGraphSummary,
}

#[derive(Debug, Clone)]
pub struct MetricsAnalysis {
    pub metrics: CodeHealthMetrics,
    pub report: MetricsReport,
}

#[derive(Debug, Clone)]
pub(crate) struct GraphSnapshot {
    pub(crate) nodes: Vec<Node>,
    pub(crate) edges: Vec<Edge>,
    pub(crate) owner_by_file: BTreeMap<String, Option<String>>,
    pub(crate) file_hash_by_file: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RustComplexityMetrics {
    pub(crate) cyclomatic_complexity: usize,
    pub(crate) cognitive_complexity: usize,
    pub(crate) branch_count: usize,
    pub(crate) max_nesting_depth: usize,
}

#[derive(Debug, Clone)]
pub(super) struct DistributionSample {
    pub(super) subject_id: String,
    pub(super) file_path: Option<String>,
    pub(super) qualified_name: Option<String>,
    pub(super) value: f64,
}
