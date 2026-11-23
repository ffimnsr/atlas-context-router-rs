use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use atlas_core::{
    BudgetReport, ChangeRiskResult, ConfidenceTier, CoverageStrength, DependencyRemovalResult,
    Edge, EdgeKind, InsightEvidence, InsightFinding, InsightLineRange, InsightSeverity, Node,
    NodeKind, ReasoningEvidence, RefactorSafetyResult, Result, RiskReport, SafetyScore,
    TestAdjacencyResult,
};
use atlas_store_sqlite::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    InsightsEngine, ReasoningEngine,
    helpers::{
        EDGE_QUERY_LIMIT, RiskInputs, build_review_focus, compute_risk_level, compute_safety_score,
        file_paths_cross_package, is_public_node, normalize_qn_kind_tokens,
    },
    metrics::{
        GraphSnapshot, NodeMetric, build_node_metrics, load_rust_complexity, module_id_for_file,
    },
    scc::strongly_connected_components,
};

mod dependency;
mod factors;
mod insights;
mod model;
mod reasoning;

pub use self::model::RiskAssessmentAnalysis;
pub use self::model::RiskAssessmentTarget;
pub use self::model::RiskClassification;
pub use self::model::RiskFactorContribution;
