//! Evidence-sufficiency model for bounded context escalation (VR3).
//!
//! These types are deterministic structural facts. They never describe an LLM
//! judgement and are computed from graph/context signals only.

use serde::{Deserialize, Serialize};

/// One unsatisfied structural constraint behind an escalation decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGap {
    /// The target did not resolve to a single node.
    TargetUnresolved,
    /// Caller retrieval hit its bucket cap.
    CallersTruncated,
    /// Callee retrieval hit its bucket cap.
    CalleesTruncated,
    /// No test neighbor or test file was attached.
    TestsMissing,
    /// An implementation/registration edge target was not resolved.
    DispatchUnresolved,
    /// Matched overviews are stale, orphaned, or unverified.
    OverviewNotFresh,
}

impl EvidenceGap {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TargetUnresolved => "target_unresolved",
            Self::CallersTruncated => "callers_truncated",
            Self::CalleesTruncated => "callees_truncated",
            Self::TestsMissing => "tests_missing",
            Self::DispatchUnresolved => "dispatch_unresolved",
            Self::OverviewNotFresh => "overview_not_fresh",
        }
    }
}

impl std::fmt::Display for EvidenceGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Deterministic sufficiency report emitted with every context result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSufficiencyReport {
    pub target_resolved: bool,
    pub callers_complete: bool,
    pub callees_complete: bool,
    pub tests_covered: bool,
    pub dispatch_complete: bool,
    pub overview_fresh: bool,
    /// Unsatisfied required constraints in deterministic order.
    pub evidence_gaps: Vec<EvidenceGap>,
    /// Fraction of required constraints satisfied, clamped to `0..=1`.
    pub sufficiency_score: f64,
    /// Why Round 2 ran (or why it was skipped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation_reason: Option<String>,
    /// Suggested follow-up tools for the remaining gaps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_tools: Vec<String>,
    /// Escalation rounds actually executed.
    pub rounds_used: u32,
    /// Set when escalation stopped early, e.g. `budget_exhausted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

impl EvidenceSufficiencyReport {
    /// Weighted fraction of satisfied constraints in `required`.
    pub fn compute_score(&mut self, required: &[EvidenceGap]) {
        if required.is_empty() {
            self.sufficiency_score = 1.0;
            return;
        }
        let satisfied = required
            .iter()
            .filter(|gap| !self.evidence_gaps.contains(gap))
            .count();
        self.sufficiency_score = (satisfied as f64 / required.len() as f64).clamp(0.0, 1.0);
    }

    /// True when at least one required constraint is unsatisfied.
    pub fn is_unsatisfied(&self) -> bool {
        !self.evidence_gaps.is_empty()
    }
}
