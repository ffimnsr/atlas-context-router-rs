use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use atlas_core::{
    AtlasError, InsightEvidence, InsightFinding, InsightLineRange, InsightSeverity,
    LargeFunctionReport, Result,
};
use atlas_repo::CanonicalRepoPath;
use serde::{Deserialize, Serialize};

use super::InsightsEngine;
use super::metrics::{GraphSnapshot, build_node_metrics, load_rust_complexity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LargeFunctionMode {
    Large,
    Complex,
    #[default]
    LargeOrComplex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeFunctionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity_threshold: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cognitive_threshold: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nesting_threshold: Option<usize>,
    #[serde(default)]
    pub mode: LargeFunctionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_tests: bool,
}

impl Default for LargeFunctionRequest {
    fn default() -> Self {
        Self {
            files: None,
            changed_files: None,
            threshold: None,
            complexity_threshold: None,
            cognitive_threshold: None,
            nesting_threshold: None,
            mode: LargeFunctionMode::LargeOrComplex,
            limit: None,
            include_tests: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LargeFunctionCandidate {
    pub file_path: String,
    pub qualified_name: String,
    pub display_name: String,
    pub node_kind: String,
    pub line_start: u32,
    pub line_end: u32,
    pub loc: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cyclomatic_complexity: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cognitive_complexity: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nesting_depth: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_count: Option<usize>,
    pub threshold: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity_threshold: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cognitive_threshold: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nesting_threshold: Option<usize>,
    pub fan_in: usize,
    pub fan_out: usize,
    pub module_boundary_crossings: usize,
    pub changed_file_boost: bool,
    pub large_match: bool,
    pub complex_match: bool,
    pub ranking_reason: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LargeFunctionReportResult {
    pub mode: LargeFunctionMode,
    #[serde(flatten)]
    pub report: LargeFunctionReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LargeFunctionAnalysis {
    pub request: LargeFunctionRequest,
    pub report: LargeFunctionReport,
    pub candidates: Vec<LargeFunctionCandidate>,
}

impl LargeFunctionAnalysis {
    pub fn report_result(&self) -> LargeFunctionReportResult {
        LargeFunctionReportResult {
            mode: self.request.mode,
            report: self.report.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RankingReasonInput {
    loc: usize,
    threshold: usize,
    cyclomatic: Option<usize>,
    complexity_threshold: usize,
    cognitive: Option<usize>,
    cognitive_threshold: usize,
    nesting: Option<usize>,
    nesting_threshold: usize,
    fan_in: usize,
    fan_out: usize,
    module_boundary_crossings: usize,
    changed_file_boost: bool,
}

impl<'s> InsightsEngine<'s> {
    pub fn find_large_functions(
        &self,
        repo_root: impl AsRef<Path>,
        request: LargeFunctionRequest,
    ) -> Result<LargeFunctionAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other(
                "large-function analysis requires a store-backed insights engine".to_owned(),
            )
        })?;
        let snapshot = self.load_graph_snapshot(store)?;
        let rust_complexity = load_rust_complexity(repo_root.as_ref(), &snapshot.nodes)?;
        let node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);
        let candidates = rank_large_function_candidates(self, &snapshot, &node_metrics, &request)?;
        let findings = candidates
            .iter()
            .map(candidate_to_finding)
            .collect::<Vec<_>>();
        let report = self.large_function_report(findings);
        let report_qnames = report
            .findings
            .iter()
            .filter_map(|finding| {
                finding
                    .evidence
                    .first()
                    .and_then(|evidence| evidence.qualified_name.clone())
            })
            .collect::<BTreeSet<_>>();
        let candidates = candidates
            .into_iter()
            .filter(|candidate| report_qnames.contains(&candidate.qualified_name))
            .collect();

        Ok(LargeFunctionAnalysis {
            request,
            report,
            candidates,
        })
    }
}

fn rank_large_function_candidates(
    engine: &InsightsEngine<'_>,
    snapshot: &GraphSnapshot,
    node_metrics: &[super::metrics::NodeMetric],
    request: &LargeFunctionRequest,
) -> Result<Vec<LargeFunctionCandidate>> {
    let normalized_files = normalize_paths(request.files.as_deref())?;
    let normalized_changed_files = normalize_paths(request.changed_files.as_deref())?;
    let file_scope = normalized_files.iter().cloned().collect::<BTreeSet<_>>();
    let changed_scope = normalized_changed_files
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    let threshold = request
        .threshold
        .unwrap_or(engine.config().large_function_loc);
    let complexity_threshold = request
        .complexity_threshold
        .unwrap_or(engine.config().high_cyclomatic_complexity);
    let cognitive_threshold = request
        .cognitive_threshold
        .unwrap_or(engine.config().high_cognitive_complexity);
    let nesting_threshold = request
        .nesting_threshold
        .unwrap_or(engine.config().max_nesting_depth);
    let requested_limit = request.limit.unwrap_or(engine.config().max_findings);

    let module_by_qname = node_metrics
        .iter()
        .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
        .collect::<HashMap<_, _>>();
    let mut module_boundary_crossings = HashMap::<String, usize>::new();
    for edge in &snapshot.edges {
        let Some(source_module) = module_by_qname.get(&edge.source_qn) else {
            continue;
        };
        let Some(target_module) = module_by_qname.get(&edge.target_qn) else {
            continue;
        };
        if source_module == target_module {
            continue;
        }
        *module_boundary_crossings
            .entry(edge.source_qn.clone())
            .or_default() += 1;
        *module_boundary_crossings
            .entry(edge.target_qn.clone())
            .or_default() += 1;
    }

    let mut candidates = node_metrics
        .iter()
        .filter(|metric| {
            request.include_tests
                || (!metric.node.is_test && metric.node.kind != atlas_core::NodeKind::Test)
        })
        .filter(|metric| file_scope.is_empty() || file_scope.contains(&metric.node.file_path))
        .filter_map(|metric| {
            let loc = metric.loc?;
            let cyclomatic = metric.cyclomatic_complexity.copied();
            let cognitive = metric.cognitive_complexity.copied();
            let nesting = metric.max_nesting_depth.copied();
            let branch_count = metric.branch_count.copied();

            let large_match = loc >= threshold;
            let complex_match = cyclomatic.is_some_and(|value| value >= complexity_threshold)
                || cognitive.is_some_and(|value| value >= cognitive_threshold)
                || nesting.is_some_and(|value| value >= nesting_threshold);

            let include = match request.mode {
                LargeFunctionMode::Large => large_match,
                LargeFunctionMode::Complex => complex_match,
                LargeFunctionMode::LargeOrComplex => large_match || complex_match,
            };
            if !include {
                return None;
            }

            let changed_file_boost =
                !changed_scope.is_empty() && changed_scope.contains(&metric.node.file_path);
            let boundary_crossings = module_boundary_crossings
                .get(&metric.node.qualified_name)
                .copied()
                .unwrap_or_default();
            let fan_in_boost = metric
                .fan_in
                .saturating_sub(engine.config().high_fan_in.saturating_sub(1));
            let fan_out_boost = metric
                .fan_out
                .saturating_sub(engine.config().high_fan_out.saturating_sub(1));
            let complexity_excess = cyclomatic
                .unwrap_or_default()
                .saturating_sub(complexity_threshold)
                + cognitive
                    .unwrap_or_default()
                    .saturating_sub(cognitive_threshold)
                + nesting
                    .unwrap_or_default()
                    .saturating_sub(nesting_threshold);
            let score = (loc as f64 * 1000.0)
                + if changed_file_boost { 500.0 } else { 0.0 }
                + (fan_in_boost as f64 * 10.0)
                + (fan_out_boost as f64 * 10.0)
                + (boundary_crossings as f64 * 5.0)
                + complexity_excess as f64;

            let reason_input = RankingReasonInput {
                loc,
                threshold,
                cyclomatic,
                complexity_threshold,
                cognitive,
                cognitive_threshold,
                nesting,
                nesting_threshold,
                fan_in: metric.fan_in,
                fan_out: metric.fan_out,
                module_boundary_crossings: boundary_crossings,
                changed_file_boost,
            };

            Some(LargeFunctionCandidate {
                file_path: metric.node.file_path.clone(),
                qualified_name: metric.node.qualified_name.clone(),
                display_name: metric.node.name.clone(),
                node_kind: metric.node.kind.as_str().to_owned(),
                line_start: metric.node.line_start,
                line_end: metric.node.line_end,
                loc,
                cyclomatic_complexity: cyclomatic,
                cognitive_complexity: cognitive,
                max_nesting_depth: nesting,
                branch_count,
                threshold,
                complexity_threshold: Some(complexity_threshold),
                cognitive_threshold: Some(cognitive_threshold),
                nesting_threshold: Some(nesting_threshold),
                fan_in: metric.fan_in,
                fan_out: metric.fan_out,
                module_boundary_crossings: boundary_crossings,
                changed_file_boost,
                large_match,
                complex_match,
                ranking_reason: ranking_reason(reason_input),
                score,
            })
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .loc
            .cmp(&left.loc)
            .then_with(|| right.changed_file_boost.cmp(&left.changed_file_boost))
            .then_with(|| right.fan_in.cmp(&left.fan_in))
            .then_with(|| right.fan_out.cmp(&left.fan_out))
            .then_with(|| {
                right
                    .module_boundary_crossings
                    .cmp(&left.module_boundary_crossings)
            })
            .then_with(|| right.score.total_cmp(&left.score))
            .then_with(|| left.file_path.cmp(&right.file_path))
            .then_with(|| left.line_start.cmp(&right.line_start))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
    });
    candidates.truncate(requested_limit);
    Ok(candidates)
}

fn candidate_to_finding(candidate: &LargeFunctionCandidate) -> InsightFinding {
    let severity = if candidate.loc >= candidate.threshold.saturating_mul(2)
        || candidate.cyclomatic_complexity.is_some_and(|value| {
            candidate
                .complexity_threshold
                .is_some_and(|threshold| value >= threshold.saturating_mul(2))
        })
        || candidate.cognitive_complexity.is_some_and(|value| {
            candidate
                .cognitive_threshold
                .is_some_and(|threshold| value >= threshold.saturating_mul(2))
        }) {
        InsightSeverity::High
    } else if candidate.large_match || candidate.complex_match {
        InsightSeverity::Medium
    } else {
        InsightSeverity::Low
    };

    InsightFinding {
        id: format!("large_function:{}", candidate.qualified_name),
        title: format!("large or complex function {}", candidate.display_name),
        severity,
        category: "large_functions".to_owned(),
        message: format!(
            "{} spans {} lines at {}:{}-{}",
            candidate.qualified_name,
            candidate.loc,
            candidate.file_path,
            candidate.line_start,
            candidate.line_end,
        ),
        evidence: vec![InsightEvidence {
            file_path: Some(candidate.file_path.clone()),
            qualified_name: Some(candidate.qualified_name.clone()),
            node_kind: Some(candidate.node_kind.clone()),
            edge_kind: None,
            line_range: Some(InsightLineRange {
                start_line: candidate.line_start,
                end_line: candidate.line_end,
            }),
            confidence_tier: None,
        }],
        ranking_reason: candidate.ranking_reason.clone(),
        details: Some(serde_json::to_value(candidate).unwrap_or(serde_json::Value::Null)),
        score: candidate.score,
    }
}

fn ranking_reason(input: RankingReasonInput) -> String {
    format!(
        "loc {loc}/{threshold}; cyclomatic {}/{complexity_threshold}; cognitive {}/{cognitive_threshold}; nesting {}/{nesting_threshold}; fan_in {fan_in}; fan_out {fan_out}; boundary_crossings {module_boundary_crossings}; changed_file_boost {}",
        input
            .cyclomatic
            .map(|value| value.to_string())
            .unwrap_or_else(|| "not_available".to_owned()),
        input
            .cognitive
            .map(|value| value.to_string())
            .unwrap_or_else(|| "not_available".to_owned()),
        input
            .nesting
            .map(|value| value.to_string())
            .unwrap_or_else(|| "not_available".to_owned()),
        if input.changed_file_boost {
            "yes"
        } else {
            "no"
        },
        loc = input.loc,
        threshold = input.threshold,
        complexity_threshold = input.complexity_threshold,
        cognitive_threshold = input.cognitive_threshold,
        nesting_threshold = input.nesting_threshold,
        fan_in = input.fan_in,
        fan_out = input.fan_out,
        module_boundary_crossings = input.module_boundary_crossings,
    )
}

fn normalize_paths(paths: Option<&[String]>) -> Result<Vec<String>> {
    paths
        .map(|paths| {
            paths
                .iter()
                .map(|path| {
                    CanonicalRepoPath::from_repo_relative(path)
                        .map(|canonical| canonical.as_str().to_owned())
                        .map_err(|error| AtlasError::Other(error.to_string()))
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()
        .map(|paths| paths.unwrap_or_default())
}
