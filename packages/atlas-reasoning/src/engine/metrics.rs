use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use atlas_core::{
    AtlasError, CoverageStrength, Edge, EdgeKind, InsightEvidence, InsightFinding,
    InsightLineRange, InsightSeverity, MetricsReport, Node, NodeKind, Result,
};
use atlas_repo::{canonical_filesystem_path, stable_repo_id};
use camino::Utf8Path;
use tree_sitter::Node as TsNode;

use super::InsightsEngine;
use super::insights::{module_matches_any, path_matches_ignore_list};

mod distribution;
mod engine;
mod findings;
mod nodes;
mod rust;
mod types;

pub(crate) use self::distribution::is_callable;
pub(crate) use self::distribution::is_reference_edge;
pub(crate) use self::distribution::module_id_for_file;
pub(crate) use self::nodes::build_node_metrics;
pub(crate) use self::rust::load_rust_complexity;
pub use self::types::CodeHealthMetrics;
pub use self::types::FileMetric;
pub(crate) use self::types::GraphSnapshot;
pub use self::types::MetricDistribution;
pub use self::types::MetricGraphSummary;
pub use self::types::MetricOutlier;
pub use self::types::MetricValue;
pub use self::types::MetricsAnalysis;
pub use self::types::ModuleMetric;
pub use self::types::NodeMetric;
pub(crate) use self::types::RustComplexityMetrics;
