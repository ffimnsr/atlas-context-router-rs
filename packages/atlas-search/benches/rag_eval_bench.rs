//! VQ1 RAG evaluation gate (deterministic, non-criterion harness).
//!
//! Runs the fixed fixture corpus, writes the JSON report artifact, checks the
//! acceptance thresholds, and compares against the committed baseline.
//!
//! Run with:
//!   cargo bench -p atlas-search --bench rag_eval_bench
//!
//! Artifact:
//!   target/bench-ci/rag-eval.json (override with `RAG_EVAL_JSON_OUT`)
//!
//! Gate can be disabled with `RAG_EVAL_GATE=0` while regenerating a baseline.

use atlas_search::rag_eval::RagEvaluationReport;

#[path = "../tests/support/rag_corpus.rs"]
mod rag_corpus;

fn baseline_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("baselines")
        .join("rag_eval_baseline.json")
}

fn output_path() -> std::path::PathBuf {
    std::env::var_os("RAG_EVAL_JSON_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
                .join("bench-ci")
                .join("rag-eval.json")
        })
}

fn main() {
    let report = rag_corpus::evaluate_rag_corpus().expect("build deterministic rag eval corpus");
    let json = report.to_json_pretty().expect("serialize rag eval report");

    let out = output_path();
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create rag eval artifact directory");
    }
    std::fs::write(&out, format!("{json}\n")).expect("write rag eval json artifact");
    println!("{json}");
    eprintln!("rag eval JSON written to {}", out.display());

    let mut failures = Vec::new();
    for gate in report.gates() {
        eprintln!(
            "rag-eval gate {}: {} (observed {})",
            gate.gate,
            if gate.passed { "pass" } else { "FAIL" },
            gate.observed
        );
        if !gate.passed {
            failures.push(format!(
                "{} observed {} (threshold {})",
                gate.gate, gate.observed, gate.threshold
            ));
        }
    }

    let baseline_path = baseline_path();
    match std::fs::read_to_string(&baseline_path) {
        Ok(raw) => match RagEvaluationReport::from_json(&raw) {
            Ok(baseline) => {
                let regressions = report.compare_to_baseline(&baseline);
                if regressions.is_empty() {
                    eprintln!("rag-eval baseline comparison: no regressions");
                }
                for regression in regressions {
                    failures.push(format!(
                        "{} regressed {:.4} -> {:.4}",
                        regression.metric, regression.baseline, regression.current
                    ));
                }
            }
            Err(error) => failures.push(format!("baseline parse error: {error}")),
        },
        Err(error) => {
            eprintln!(
                "warning: no rag eval baseline at {} ({error}); skipping regression gate",
                baseline_path.display()
            );
        }
    }

    if !failures.is_empty() && std::env::var("RAG_EVAL_GATE").as_deref() != Ok("0") {
        eprintln!("rag eval gate failed:");
        for failure in &failures {
            eprintln!("  - {failure}");
        }
        std::process::exit(1);
    }
}
