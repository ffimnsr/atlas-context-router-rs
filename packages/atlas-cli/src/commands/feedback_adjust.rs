//! ICM-C3 — Feedback-driven confidence adjustment for analysis outputs.
//!
//! `FeedbackAdjuster` queries `feedback_records` (session store) before
//! analysis results are returned and lowers confidence by one deterministic
//! step per surface whenever prior feedback marks the same symbol, file, or
//! analysis kind as a false positive. Adjustment is best-effort: a missing or
//! unreadable session store, a disabled config flag, or an empty feedback
//! table all mean "no change".
//!
//! Rules honored (ISSUES.md ICM-C):
//! - never lower confidence without a matching false-positive record
//! - never mutate state, only reported results
//! - every applied change is surfaced as `feedback_evidence` in JSON output

use std::path::Path;

use atlas_core::{
    ConfidenceTier, DeadCodeCandidate, ImpactClass, RefactorPlan, RefactorSafetyResult,
    RemovalImpactResult, SafetyBand,
};
use atlas_session::{FeedbackRecord, SessionStore};
use serde_json::Value;

/// One applied confidence adjustment, serialized as `feedback_evidence`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackEvidence {
    pub record_id: String,
    pub analysis_kind: String,
    pub predicted: String,
    pub actual: String,
    pub correction: String,
    pub related_symbol: Option<String>,
    pub related_file: Option<String>,
    /// Why this record matched: `symbol`, `file`, or `kind`.
    pub matched_on: String,
}

/// One deterministic lowering step per analysis surface.
pub struct FeedbackAdjuster {
    repo_root: String,
    enabled: bool,
    store: Option<SessionStore>,
}

impl FeedbackAdjuster {
    /// Open the adjuster best-effort: `None` when the config flag is off or
    /// the session store cannot be opened. Never fails analysis on its own.
    pub fn new(repo_root: &str) -> Self {
        let enabled = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .map(|config| config.analysis.feedback_adjustment.enabled)
            .unwrap_or(false);
        let store = SessionStore::open_in_repo(Path::new(repo_root)).ok();
        Self {
            repo_root: repo_root.to_owned(),
            enabled,
            store,
        }
    }

    fn matching(
        &self,
        kind: &str,
        symbol: Option<&str>,
        file: Option<&str>,
    ) -> Vec<FeedbackRecord> {
        let Some(store) = &self.store else {
            return Vec::new();
        };
        if !self.enabled {
            return Vec::new();
        }
        store
            .feedback_matching(&self.repo_root, kind, symbol, file)
            .unwrap_or_default()
            .into_iter()
            .filter(FeedbackRecord::is_false_positive_evidence)
            .collect()
    }

    fn evidence_for(
        records: &[FeedbackRecord],
        symbol: Option<&str>,
        file: Option<&str>,
        _kind: &str,
    ) -> Vec<FeedbackEvidence> {
        records
            .iter()
            .map(|record| {
                let matched_on =
                    if symbol.is_some_and(|s| record.related_symbol.as_deref() == Some(s)) {
                        "symbol"
                    } else if file.is_some_and(|f| record.related_file.as_deref() == Some(f)) {
                        "file"
                    } else {
                        "kind"
                    };
                FeedbackEvidence {
                    record_id: record.id.clone(),
                    analysis_kind: record.analysis_kind.clone(),
                    predicted: record.predicted.clone(),
                    actual: record.actual.clone(),
                    correction: record.correction.clone(),
                    related_symbol: record.related_symbol.clone(),
                    related_file: record.related_file.clone(),
                    matched_on: matched_on.to_owned(),
                }
            })
            .collect()
    }

    /// Lower dead-code certainty one step per matching candidate.
    pub fn adjust_dead_code(&self, candidates: &mut [DeadCodeCandidate]) -> Vec<FeedbackEvidence> {
        let mut evidence = Vec::new();
        for candidate in candidates {
            let records = self.matching(
                "dead_code",
                Some(&candidate.node.qualified_name),
                Some(&candidate.node.file_path),
            );
            if records.is_empty() {
                continue;
            }
            candidate.certainty = lower_tier(candidate.certainty);
            evidence.extend(Self::evidence_for(
                &records,
                Some(&candidate.node.qualified_name),
                Some(&candidate.node.file_path),
                "dead_code",
            ));
        }
        dedup_evidence(evidence)
    }

    /// Lower impacted-node impact class one step per matching node.
    pub fn adjust_removal(&self, result: &mut RemovalImpactResult) -> Vec<FeedbackEvidence> {
        let mut evidence = Vec::new();
        for impacted in &mut result.impacted_symbols {
            let records = self.matching(
                "remove",
                Some(&impacted.node.qualified_name),
                Some(&impacted.node.file_path),
            );
            if records.is_empty() {
                continue;
            }
            impacted.impact_class = lower_impact_class(impacted.impact_class);
            evidence.extend(Self::evidence_for(
                &records,
                Some(&impacted.node.qualified_name),
                Some(&impacted.node.file_path),
                "remove",
            ));
        }
        dedup_evidence(evidence)
    }

    /// Lower the numeric safety score (and recompute the band) one step per
    /// matching record for the analysed symbol.
    pub fn adjust_safety(
        &self,
        symbol: &str,
        result: &mut RefactorSafetyResult,
    ) -> Vec<FeedbackEvidence> {
        let records = self.matching("safety", Some(symbol), Some(&result.node.file_path));
        if records.is_empty() {
            return Vec::new();
        }
        result.safety.score = (result.safety.score - 0.1).max(0.0);
        result.safety.band = safety_band(result.safety.score);
        result.safety.reasons.push(
            "confidence lowered by matching feedback evidence (false positive on record)"
                .to_owned(),
        );
        dedup_evidence(Self::evidence_for(
            &records,
            Some(symbol),
            Some(&result.node.file_path),
            "safety",
        ))
    }

    /// Lower the estimated safety band of a dead-code removal plan one step;
    /// only used for dry-run reporting.
    pub fn adjust_remove_dead_plan(
        &self,
        symbol: &str,
        plan: &mut RefactorPlan,
    ) -> Vec<FeedbackEvidence> {
        let records = self.matching("remove_dead", Some(symbol), None);
        if records.is_empty() {
            return Vec::new();
        }
        plan.estimated_safety = lower_band(plan.estimated_safety);
        plan.manual_review.push(format!(
            "removal confidence lowered by feedback evidence: {} matching correction(s)",
            records.len()
        ));
        dedup_evidence(Self::evidence_for(
            &records,
            Some(symbol),
            None,
            "remove_dead",
        ))
    }
}

fn lower_tier(tier: ConfidenceTier) -> ConfidenceTier {
    match tier {
        ConfidenceTier::High => ConfidenceTier::Medium,
        ConfidenceTier::Medium => ConfidenceTier::Low,
        ConfidenceTier::Low => ConfidenceTier::Low,
    }
}

fn lower_impact_class(class: ImpactClass) -> ImpactClass {
    match class {
        ImpactClass::Definite => ImpactClass::Probable,
        ImpactClass::Probable => ImpactClass::Weak,
        ImpactClass::Weak => ImpactClass::Weak,
    }
}

fn lower_band(band: SafetyBand) -> SafetyBand {
    match band {
        SafetyBand::Safe => SafetyBand::Caution,
        SafetyBand::Caution => SafetyBand::Risky,
        SafetyBand::Risky => SafetyBand::Risky,
    }
}

/// Mirrors the engine's band mapping (score >= 0.7 safe, >= 0.4 caution).
pub fn safety_band(score: f64) -> SafetyBand {
    if score >= 0.7 {
        SafetyBand::Safe
    } else if score >= 0.4 {
        SafetyBand::Caution
    } else {
        SafetyBand::Risky
    }
}

/// Deterministic evidence dedup: same record may match several nodes.
fn dedup_evidence(evidence: Vec<FeedbackEvidence>) -> Vec<FeedbackEvidence> {
    let mut seen = std::collections::BTreeSet::new();
    evidence
        .into_iter()
        .filter(|entry| seen.insert(entry.record_id.clone()))
        .collect()
}

/// Serialize evidence for JSON output; `null` when empty so callers can skip
/// the field entirely.
pub fn evidence_json(evidence: &[FeedbackEvidence]) -> Value {
    serde_json::to_value(evidence).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_and_band_lowering_is_one_deterministic_step() {
        assert_eq!(lower_tier(ConfidenceTier::High), ConfidenceTier::Medium);
        assert_eq!(lower_tier(ConfidenceTier::Medium), ConfidenceTier::Low);
        assert_eq!(lower_tier(ConfidenceTier::Low), ConfidenceTier::Low);

        assert_eq!(
            lower_impact_class(ImpactClass::Definite),
            ImpactClass::Probable
        );
        assert_eq!(lower_impact_class(ImpactClass::Probable), ImpactClass::Weak);
        assert_eq!(lower_impact_class(ImpactClass::Weak), ImpactClass::Weak);

        assert_eq!(lower_band(SafetyBand::Safe), SafetyBand::Caution);
        assert_eq!(lower_band(SafetyBand::Caution), SafetyBand::Risky);
        assert_eq!(lower_band(SafetyBand::Risky), SafetyBand::Risky);

        assert_eq!(safety_band(0.7), SafetyBand::Safe);
        assert_eq!(safety_band(0.69), SafetyBand::Caution);
        assert_eq!(safety_band(0.4), SafetyBand::Caution);
        assert_eq!(safety_band(0.39), SafetyBand::Risky);
    }

    #[test]
    fn evidence_marks_the_strongest_match_key() {
        let record = FeedbackRecord {
            id: "r1".to_owned(),
            repo_root: "/repo".to_owned(),
            session_id: None,
            tool_name: "cli".to_owned(),
            analysis_kind: "remove".to_owned(),
            predicted: "removable".to_owned(),
            actual: "blocked".to_owned(),
            correction: "dynamic dispatch".to_owned(),
            related_symbol: Some("src/a.rs::fn::f".to_owned()),
            related_file: Some("src/a.rs".to_owned()),
            source_id: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            metadata: serde_json::json!({}),
        };
        let evidence = FeedbackAdjuster::evidence_for(
            &[record],
            Some("src/a.rs::fn::f"),
            Some("src/a.rs"),
            "remove",
        );
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].matched_on, "symbol");
        assert_eq!(evidence[0].record_id, "r1");
    }
}
