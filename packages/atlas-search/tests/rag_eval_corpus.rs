//! VQ1 — deterministic fixture corpus and baseline-stability checks.

#[path = "support/rag_corpus.rs"]
mod rag_corpus;

use atlas_search::rag_eval::RagEvaluationReport;

fn baseline() -> RagEvaluationReport {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("baselines")
        .join("rag_eval_baseline.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read baseline {}: {error}", path.display()));
    RagEvaluationReport::from_json(&raw)
        .unwrap_or_else(|error| panic!("parse baseline {}: {error}", path.display()))
}

#[test]
fn corpus_is_deterministic_across_two_runs() {
    let first = rag_corpus::evaluate_rag_corpus().expect("first run");
    let second = rag_corpus::evaluate_rag_corpus().expect("second run");

    assert_eq!(
        first, second,
        "the fixture corpus must produce identical reports"
    );
    assert!(
        first.passes(),
        "acceptance gates failed: {:?}",
        first.gates()
    );
    assert!(
        first.token_efficiency.l0_ratio <= first.thresholds.max_l0_l2_ratio,
        "L0 must stay within 10% of L2 tokens"
    );
    assert!(
        first.auxiliary_lift > 0.0,
        "auxiliary recall must lift recall"
    );
    assert!(
        first.rounds_used <= first.thresholds.max_escalation_rounds,
        "experience edges must keep escalation at or below the cap"
    );
}

#[test]
fn report_round_trips_through_json() {
    let report = rag_corpus::evaluate_rag_corpus().expect("report");
    let json = report.to_json_pretty().expect("serialize report");
    let parsed = RagEvaluationReport::from_json(&json).expect("parse report");

    // Integer and enum fields must round-trip exactly. Float fields are
    // compared through the same tolerance the baseline gate uses, because
    // serde_json float parsing may differ by one ULP without the
    // `float_roundtrip` feature.
    assert_eq!(report.schema_version, parsed.schema_version);
    assert_eq!(report.corpus, parsed.corpus);
    assert_eq!(report.recall_k, parsed.recall_k);
    assert_eq!(report.l0_tokens, parsed.l0_tokens);
    assert_eq!(report.l1_tokens, parsed.l1_tokens);
    assert_eq!(report.l2_tokens, parsed.l2_tokens);
    assert_eq!(report.dsq_pruning, parsed.dsq_pruning);
    assert_eq!(report.rounds_used, parsed.rounds_used);
    assert_eq!(
        report.rounds_without_experience,
        parsed.rounds_without_experience
    );
    assert_eq!(report.gates(), parsed.gates());
    assert!(report.compare_to_baseline(&parsed).is_empty());
    assert!(parsed.compare_to_baseline(&report).is_empty());
}

#[test]
fn baseline_comparison_is_stable_across_two_runs() {
    let baseline = baseline();
    let first = rag_corpus::evaluate_rag_corpus().expect("first run");
    let second = rag_corpus::evaluate_rag_corpus().expect("second run");

    let first_regressions = first.compare_to_baseline(&baseline);
    let second_regressions = second.compare_to_baseline(&baseline);

    assert!(
        first_regressions.is_empty(),
        "current corpus regressed against baseline: {first_regressions:?}"
    );
    assert_eq!(first_regressions, second_regressions);
}
