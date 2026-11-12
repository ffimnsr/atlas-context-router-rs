//! VQ1 — RAG adaptation evaluation report and release gates.
//!
//! This module turns the four RAG measurement families (L0/L1/L2 token
//! efficiency, DSQ candidate pruning, two-path memory recall lift, and
//! experience-edge escalation-round reduction) into one deterministic,
//! JSON-serializable report.
//!
//! Measurements are counts and ratios, never wall-clock timings, so the same
//! corpus produces byte-identical JSON across runs and hosts. A committed
//! baseline under `benches/baselines/` makes regressions visible: any metric
//! that worsens by more than [`RAG_BASELINE_REGRESSION_TOLERANCE`] fails the
//! bench gate.

use serde::{Deserialize, Serialize};

pub use crate::eval::TokenEfficiency;

/// Schema version for the emitted report and stored baselines.
pub const RAG_EVAL_SCHEMA_VERSION: u32 = 1;

/// Relative regression tolerated before the baseline gate fails.
pub const RAG_BASELINE_REGRESSION_TOLERANCE: f64 = 0.10;

/// Recall cut-off used by the auxiliary-memoir measurement.
pub const RAG_AUX_RECALL_K: usize = 10;

/// Acceptance thresholds for the RAG adaptation release gate.
///
/// These are deterministic counts/ratios, not timings, so they can be checked
/// on any host without a warm cache or embedding backend.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RagEvalThresholds {
    /// L0 tokens may not exceed this fraction of L2 tokens.
    pub max_l0_l2_ratio: f64,
    /// DSQ scoped queries must remove at least this fraction of candidates.
    pub min_dsq_reduction: f64,
    /// Two-path recall must beat primary-only recall by more than this.
    pub min_auxiliary_lift: f64,
    /// Experience edges must keep escalation at or below this round count.
    pub max_escalation_rounds: usize,
}

/// Default RAG adaptation acceptance thresholds.
pub const RAG_EVAL_THRESHOLDS: RagEvalThresholds = RagEvalThresholds {
    max_l0_l2_ratio: 0.10,
    min_dsq_reduction: 0.50,
    min_auxiliary_lift: 0.0,
    max_escalation_rounds: 1,
};

/// DSQ scope predicate measured by one pruning row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DsqScopeKind {
    /// Recursive path prefix (`subpath_prefix`).
    Recursive,
    /// Non-recursive exact parent directory (`exact_dir`).
    ExactDir,
    /// Recursive prefix with excluded subtrees.
    Exclusion,
}

impl DsqScopeKind {
    /// Stable label used in JSON and gate names.
    pub fn label(self) -> &'static str {
        match self {
            Self::Recursive => "recursive",
            Self::ExactDir => "exact_dir",
            Self::Exclusion => "exclusion",
        }
    }
}

/// Candidate counts for one DSQ predicate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DsqPruning {
    /// Predicate that produced the counts.
    pub scope: DsqScopeKind,
    /// Candidates left after scope pre-filtering.
    pub scoped_candidate_count: usize,
    /// Candidates for the same query without a path scope.
    pub unscoped_candidate_count: usize,
}

impl DsqPruning {
    /// Fraction of unscoped candidates removed by the scope predicate.
    pub fn pruning_ratio(&self) -> f64 {
        if self.unscoped_candidate_count == 0 {
            return 0.0;
        }
        let retained = self.scoped_candidate_count as f64 / self.unscoped_candidate_count as f64;
        (1.0 - retained).clamp(0.0, 1.0)
    }
}

/// Outcome of one acceptance gate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagGateResult {
    /// Stable gate name.
    pub gate: String,
    /// Whether the observed value satisfies the threshold.
    pub passed: bool,
    /// Observed value rendered deterministically.
    pub observed: String,
    /// Threshold rendered deterministically.
    pub threshold: String,
}

/// One metric that regressed beyond the allowed tolerance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagRegression {
    /// Metric name (stable across runs).
    pub metric: String,
    /// Committed baseline value.
    pub baseline: f64,
    /// Current measured value.
    pub current: f64,
    /// Relative tolerance applied to the comparison.
    pub tolerance: f64,
}

/// Full four-family RAG evaluation report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RagEvaluationReport {
    /// Report schema version.
    pub schema_version: u32,
    /// Corpus identifier the report was produced from.
    pub corpus: String,
    /// Recall cut-off used by the auxiliary-memoir measurement.
    pub recall_k: usize,
    /// L0/L1/L2 token ratios against the L2 baseline.
    pub token_efficiency: TokenEfficiency,
    /// Approximate L0 tokens across the overview fixture.
    pub l0_tokens: usize,
    /// Approximate L1 tokens across the overview fixture.
    pub l1_tokens: usize,
    /// Approximate L2 tokens across the overview fixture.
    pub l2_tokens: usize,
    /// DSQ candidate counts, one row per predicate.
    pub dsq_pruning: Vec<DsqPruning>,
    /// Primary-only memory recall@k.
    pub primary_recall_at_k: f64,
    /// Two-path memory recall@k.
    pub aux_recall_at_k: f64,
    /// `aux_recall_at_k - primary_recall_at_k`.
    pub auxiliary_lift: f64,
    /// Escalation rounds without experience edges.
    pub rounds_without_experience: usize,
    /// Escalation rounds with experience edges.
    pub rounds_used: usize,
    /// `rounds_without_experience - rounds_used`.
    pub escalation_round_reduction: usize,
    /// Thresholds this report is judged against.
    pub thresholds: RagEvalThresholds,
}

impl RagEvaluationReport {
    /// Assemble a report, deriving the lift and round-reduction fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        corpus: impl Into<String>,
        recall_k: usize,
        token_efficiency: TokenEfficiency,
        l0_tokens: usize,
        l1_tokens: usize,
        l2_tokens: usize,
        dsq_pruning: Vec<DsqPruning>,
        primary_recall_at_k: f64,
        aux_recall_at_k: f64,
        rounds_without_experience: usize,
        rounds_used: usize,
    ) -> Self {
        Self {
            schema_version: RAG_EVAL_SCHEMA_VERSION,
            corpus: corpus.into(),
            recall_k,
            token_efficiency,
            l0_tokens,
            l1_tokens,
            l2_tokens,
            dsq_pruning,
            primary_recall_at_k,
            aux_recall_at_k,
            auxiliary_lift: aux_recall_at_k - primary_recall_at_k,
            rounds_without_experience,
            rounds_used,
            escalation_round_reduction: rounds_without_experience.saturating_sub(rounds_used),
            thresholds: RAG_EVAL_THRESHOLDS,
        }
    }

    /// Evaluate every acceptance gate in deterministic report order.
    pub fn gates(&self) -> Vec<RagGateResult> {
        let thresholds = &self.thresholds;
        let mut gates = Vec::new();

        let l0_ratio = self.token_efficiency.l0_ratio;
        gates.push(gate(
            "l0_token_ratio",
            l0_ratio <= thresholds.max_l0_l2_ratio,
            format!("{l0_ratio:.4}"),
            format!("<= {:.4}", thresholds.max_l0_l2_ratio),
        ));

        for row in &self.dsq_pruning {
            let ratio = row.pruning_ratio();
            gates.push(gate(
                &format!("dsq_reduction/{}", row.scope.label()),
                ratio >= thresholds.min_dsq_reduction,
                format!("{ratio:.4}"),
                format!(">= {:.4}", thresholds.min_dsq_reduction),
            ));
        }

        gates.push(gate(
            "auxiliary_lift",
            self.auxiliary_lift > thresholds.min_auxiliary_lift,
            format!("{:.4}", self.auxiliary_lift),
            format!("> {:.4}", thresholds.min_auxiliary_lift),
        ));
        gates.push(gate(
            "escalation_rounds",
            self.rounds_used <= thresholds.max_escalation_rounds,
            self.rounds_used.to_string(),
            format!("<= {}", thresholds.max_escalation_rounds),
        ));

        gates
    }

    /// `true` when every acceptance gate passes.
    pub fn passes(&self) -> bool {
        self.gates().iter().all(|gate| gate.passed)
    }

    /// Compare this report against a committed baseline.
    ///
    /// Returns one [`RagRegression`] per metric that worsened by more than
    /// [`RAG_BASELINE_REGRESSION_TOLERANCE`]. Higher-is-better metrics are
    /// compared against `baseline * (1 - tolerance)`; lower-is-better metrics
    /// against `baseline * (1 + tolerance)`.
    pub fn compare_to_baseline(&self, baseline: &Self) -> Vec<RagRegression> {
        let mut regressions = Vec::new();

        push_regression(
            &mut regressions,
            "l0_l2_token_ratio",
            baseline.token_efficiency.l0_ratio,
            self.token_efficiency.l0_ratio,
            false,
        );
        push_regression(
            &mut regressions,
            "l1_l2_token_ratio",
            baseline.token_efficiency.l1_ratio,
            self.token_efficiency.l1_ratio,
            false,
        );

        for row in &self.dsq_pruning {
            let label = form_dsq_metric(row.scope);
            if let Some(baseline_row) = baseline
                .dsq_pruning
                .iter()
                .find(|candidate| candidate.scope == row.scope)
            {
                push_regression(
                    &mut regressions,
                    &label,
                    baseline_row.pruning_ratio(),
                    row.pruning_ratio(),
                    true,
                );
            }
        }

        push_regression(
            &mut regressions,
            "auxiliary_lift",
            baseline.auxiliary_lift,
            self.auxiliary_lift,
            true,
        );
        push_regression(
            &mut regressions,
            "rounds_used",
            baseline.rounds_used as f64,
            self.rounds_used as f64,
            false,
        );

        regressions
    }

    /// Serialize this report as pretty-printed JSON.
    pub fn to_json_pretty(&self) -> std::result::Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Parse a report from JSON.
    pub fn from_json(value: &str) -> std::result::Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}

fn gate(name: &str, passed: bool, observed: String, threshold: String) -> RagGateResult {
    RagGateResult {
        gate: name.to_owned(),
        passed,
        observed,
        threshold,
    }
}

fn form_dsq_metric(scope: DsqScopeKind) -> String {
    format!("dsq_reduction/{}", scope.label())
}

fn push_regression(
    regressions: &mut Vec<RagRegression>,
    metric: &str,
    baseline: f64,
    current: f64,
    higher_is_better: bool,
) {
    let regressed = if higher_is_better {
        if baseline > 0.0 {
            current < baseline * (1.0 - RAG_BASELINE_REGRESSION_TOLERANCE)
        } else {
            current < baseline
        }
    } else if baseline > 0.0 {
        current > baseline * (1.0 + RAG_BASELINE_REGRESSION_TOLERANCE)
    } else {
        current > baseline
    };

    if regressed {
        regressions.push(RagRegression {
            metric: metric.to_owned(),
            baseline,
            current,
            tolerance: RAG_BASELINE_REGRESSION_TOLERANCE,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(l0_ratio: f64, dsq_ratio: f64, lift: f64, rounds: usize) -> RagEvaluationReport {
        let l0_tokens = (l0_ratio * 1000.0).round() as usize;
        RagEvaluationReport::new(
            "unit",
            RAG_AUX_RECALL_K,
            TokenEfficiency {
                l0_ratio,
                l1_ratio: 0.4,
                l2_ratio: 1.0,
            },
            l0_tokens,
            400,
            1000,
            vec![DsqPruning {
                scope: DsqScopeKind::Recursive,
                scoped_candidate_count: ((1.0 - dsq_ratio) * 100.0).round() as usize,
                unscoped_candidate_count: 100,
            }],
            0.8,
            0.8 + lift,
            2,
            rounds,
        )
    }

    #[test]
    fn passing_report_meets_every_gate() {
        let report = report(0.05, 0.9, 0.2, 1);
        assert!(report.passes(), "{:?}", report.gates());
        assert!((report.auxiliary_lift - 0.2).abs() < 1e-9);
        assert_eq!(report.escalation_round_reduction, 1);
    }

    #[test]
    fn l0_budget_failure_is_reported() {
        let report = report(0.25, 0.9, 0.2, 1);
        let gate = report
            .gates()
            .into_iter()
            .find(|gate| gate.gate == "l0_token_ratio")
            .expect("l0 gate");
        assert!(!gate.passed);
    }

    #[test]
    fn dsq_gate_fails_below_half_reduction() {
        let report = report(0.05, 0.4, 0.2, 1);
        assert!(!report.passes());
    }

    #[test]
    fn auxiliary_gate_requires_positive_lift() {
        let report = report(0.05, 0.9, 0.0, 1);
        let gate = report
            .gates()
            .into_iter()
            .find(|gate| gate.gate == "auxiliary_lift")
            .expect("aux gate");
        assert!(!gate.passed);
    }

    #[test]
    fn escalation_gate_rejects_round_two() {
        let report = report(0.05, 0.9, 0.2, 2);
        let gate = report
            .gates()
            .into_iter()
            .find(|gate| gate.gate == "escalation_rounds")
            .expect("round gate");
        assert!(!gate.passed);
    }

    #[test]
    fn baseline_regression_uses_ten_percent_tolerance() {
        let baseline = report(0.05, 0.9, 0.2, 1);
        let within_tolerance = report(0.054, 0.86, 0.19, 1);
        assert!(within_tolerance.compare_to_baseline(&baseline).is_empty());

        let regressed = report(0.08, 0.7, 0.1, 2);
        let names: Vec<String> = regressed
            .compare_to_baseline(&baseline)
            .into_iter()
            .map(|regression| regression.metric)
            .collect();
        assert!(names.contains(&"l0_l2_token_ratio".to_owned()));
        assert!(names.contains(&"dsq_reduction/recursive".to_owned()));
        assert!(names.contains(&"auxiliary_lift".to_owned()));
        assert!(names.contains(&"rounds_used".to_owned()));
    }

    #[test]
    fn report_round_trips_through_json() {
        let report = report(0.05, 0.9, 0.2, 1);
        let json = report.to_json_pretty().expect("serialize");
        let parsed = RagEvaluationReport::from_json(&json).expect("deserialize");
        assert_eq!(report, parsed);
    }
}
