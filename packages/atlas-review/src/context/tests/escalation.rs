//! VR3 evidence-sufficiency and VR4 experience-edge context tests.

use super::*;
use atlas_core::BudgetPolicy;
use atlas_core::model::{
    ContextOverview, EvidenceGap, EvidenceSufficiencyReport, ExperienceShortcutEvidence,
    TruncationMeta,
};
use atlas_session::{NewExperienceEdge, SessionStore};
use atlas_store_sqlite::Store;

fn symbol_node(qname: &str, file: &str, name: &str) -> Node {
    make_node(qname, name, file, NodeKind::Function, None)
}

/// Fixed fixture covering unresolved-target, caller-truncation, missing-test,
/// dispatch, and stale-overview signals.
fn seed_signal_fixture(store: &mut Store) {
    let target = symbol_node("src/target.rs::fn::target", "src/target.rs", "target");

    let mut caller_files = Vec::new();
    for index in 0..20 {
        let path = format!("src/callers/caller_{index}.rs");
        let qname = format!("{path}::fn::caller_{index}");
        let caller = symbol_node(&qname, &path, &format!("caller_{index}"));
        caller_files.push(ParsedFile {
            path: path.clone(),
            language: Some("rust".to_owned()),
            hash: format!("h{index}"),
            size: None,
            nodes: vec![caller],
            edges: vec![make_edge(&qname, &target_qname(), EdgeKind::Calls, &path)],
        });
    }

    let mut files = vec![ParsedFile {
        path: "src/target.rs".to_owned(),
        language: Some("rust".to_owned()),
        hash: "target".to_owned(),
        size: None,
        nodes: vec![target.clone()],
        edges: vec![make_edge(
            &target_qname(),
            "src/missing.rs::trait::MissingImpl",
            EdgeKind::Implements,
            "src/target.rs",
        )],
    }];
    files.extend(caller_files);
    store
        .replace_files_transactional_for_repo("repo_signals", &files)
        .unwrap();
}

fn target_qname() -> String {
    "src/target.rs::fn::target".to_owned()
}

fn request(intent: ContextIntent) -> ContextRequest {
    ContextRequest {
        intent,
        target: ContextTarget::QualifiedName {
            qname: target_qname(),
        },
        include_tests: false,
        include_imports: false,
        include_neighbors: false,
        ..ContextRequest::default()
    }
}

fn empty_result(request: ContextRequest) -> ContextResult {
    ContextResult {
        request,
        nodes: vec![],
        edges: vec![],
        files: vec![],
        truncation: TruncationMeta::none(),
        seed_budgets: vec![],
        traversal_budget: None,
        ambiguity: None,
        workflow: None,
        saved_context_sources: vec![],
        content_assets: vec![],
        overviews: vec![],
        sufficiency: None,
        repo_provenance: None,
        telemetry: None,
        budget: BudgetReport::not_applicable(),
    }
}

#[test]
fn fixed_fixture_reports_caller_truncation_and_missing_tests() {
    let mut store = open_store();
    seed_signal_fixture(&mut store);
    let engine = ContextEngine::new(&store).with_escalation(false, 1);
    let result = engine.build(&request(ContextIntent::UsageLookup)).unwrap();
    let report = result.sufficiency.clone().expect("sufficiency report");

    assert!(report.target_resolved);
    assert!(!report.callers_complete);
    assert!(
        report
            .evidence_gaps
            .contains(&EvidenceGap::CallersTruncated)
    );
    assert!(!report.tests_covered);

    // Tests are only a required constraint for refactor/removal intents; the
    // same result reports the gap when evaluated under that contract.
    let mut refactor = request(ContextIntent::RefactorSafety);
    refactor.target = ContextTarget::ChangedFiles {
        paths: vec!["src/target.rs".to_owned()],
    };
    let refactor_report = super::sufficiency::evaluate(&refactor, &result);
    assert!(
        refactor_report
            .evidence_gaps
            .contains(&EvidenceGap::TestsMissing)
    );
}

#[test]
fn dispatch_and_overview_signals_are_reported() {
    let mut store = open_store();
    seed_signal_fixture(&mut store);
    let engine = ContextEngine::new(&store).with_escalation(false, 1);
    let mut result = engine.build(&request(ContextIntent::UsageLookup)).unwrap();
    // The fixed graph fixture cannot produce a stale overview without a
    // session DB, so inject the two structural signals deterministically.
    result.edges.push(atlas_core::model::SelectedEdge {
        edge: make_edge(
            &target_qname(),
            "src/missing.rs::trait::MissingImpl",
            EdgeKind::Implements,
            "src/target.rs",
        ),
        selection_reason: SelectionReason::DirectTarget,
        depth: Some(0),
        relevance_score: 0.0,
        context_ranking_evidence: None,
    });
    let mut stale = request(ContextIntent::UsageLookup);
    stale.include_overviews = true;
    result.overviews.push(ContextOverview {
        overview_id: "ov_stale".to_owned(),
        subject_id: target_qname(),
        subject_kind: "symbol".to_owned(),
        title: "target".to_owned(),
        summary: "stale summary".to_owned(),
        preview: "stale preview".to_owned(),
        freshness: "stale".to_owned(),
        confidence: 0.5,
        selection_reason: "same_symbol".to_owned(),
        source_id: None,
        canonical_file_path: Some("src/target.rs".to_owned()),
        qualified_name: Some(target_qname()),
        stale_reason: Some("commit moved".to_owned()),
        level: None,
        uri: None,
    });

    let report = super::sufficiency::evaluate(&stale, &result);
    assert!(!report.dispatch_complete);
    assert!(!report.overview_fresh);
    assert!(
        report
            .evidence_gaps
            .contains(&EvidenceGap::OverviewNotFresh)
    );
}

#[test]
fn truncation_round_two_escalates() {
    let mut store = open_store();
    seed_signal_fixture(&mut store);
    let engine = ContextEngine::new(&store)
        .with_budget_policy(BudgetPolicy::default())
        .with_escalation(true, 1);
    let result = engine.build(&request(ContextIntent::UsageLookup)).unwrap();
    let report = result.sufficiency.expect("report");
    assert_eq!(report.rounds_used, 1);
    assert!(report.escalation_reason.is_some());
}

#[test]
fn escalation_stops_when_budget_is_exhausted() {
    let mut store = open_store();
    seed_signal_fixture(&mut store);
    let engine = ContextEngine::new(&store).with_escalation(true, 1);
    let mut req = request(ContextIntent::UsageLookup);
    req.token_budget = Some(1);
    let result = engine.build(&req).unwrap();
    let report = result.sufficiency.expect("report");
    assert_eq!(report.rounds_used, 0);
    assert_eq!(report.skipped_reason.as_deref(), Some("budget_exhausted"));
}

#[test]
fn escalation_respects_hard_round_cap() {
    let mut store = open_store();
    seed_signal_fixture(&mut store);
    let engine = ContextEngine::new(&store).with_escalation(true, 99);
    let result = engine.build(&request(ContextIntent::UsageLookup)).unwrap();
    let report = result.sufficiency.expect("report");
    assert_eq!(report.rounds_used, 3);
}

#[test]
fn escalation_output_is_deterministic() {
    let mut store = open_store();
    seed_signal_fixture(&mut store);
    let engine = ContextEngine::new(&store).with_escalation(true, 2);
    let first = engine.build(&request(ContextIntent::UsageLookup)).unwrap();
    let second = engine.build(&request(ContextIntent::UsageLookup)).unwrap();
    assert_eq!(
        serde_json::to_value(&first.sufficiency).unwrap(),
        serde_json::to_value(&second.sufficiency).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
}

// ── VR4 experience edges in context assembly ────────────────────────────────

fn seed_test_fixture(store: &mut Store) {
    let target = symbol_node("src/target.rs::fn::target", "src/target.rs", "target");
    let mut test_node = make_node(
        "tests/target_test.rs::fn::target_test",
        "target_test",
        "tests/target_test.rs",
        NodeKind::Test,
        None,
    );
    test_node.is_test = true;
    let files = vec![
        ParsedFile {
            path: "src/target.rs".to_owned(),
            language: Some("rust".to_owned()),
            hash: "target".to_owned(),
            size: None,
            nodes: vec![target],
            edges: vec![],
        },
        ParsedFile {
            path: "tests/target_test.rs".to_owned(),
            language: Some("rust".to_owned()),
            hash: "test".to_owned(),
            size: None,
            nodes: vec![test_node],
            edges: vec![],
        },
    ];
    store
        .replace_files_transactional_for_repo("repo_signals", &files)
        .unwrap();
}

fn repo_root_for(dir: &tempfile::TempDir) -> String {
    dir.path().to_string_lossy().into_owned()
}

fn test_edge(repo: &str, source_uri: &str, target_uri: &str) -> NewExperienceEdge {
    NewExperienceEdge {
        repo_root: repo.to_owned(),
        source_uri: source_uri.to_owned(),
        target_uri: target_uri.to_owned(),
        query_text: "src/target.rs".to_owned(),
        trace_summary: "query_graph -> target".to_owned(),
        commit_sha: "abc123".to_owned(),
        source_file_hash: Some("h-source".to_owned()),
        target_file_hash: Some("h-target".to_owned()),
        confidence: 0.9,
        query_embedding: None,
    }
}

fn experience_engine<'a>(
    store: &'a Store,
    session: &'a SessionStore,
    repo: &'a str,
) -> ContextEngine<'a> {
    ContextEngine::new(store)
        .with_escalation(true, 2)
        .with_experience_store(repo, session, 3)
}

fn refactor_request() -> ContextRequest {
    ContextRequest {
        intent: ContextIntent::RefactorSafety,
        target: ContextTarget::ChangedFiles {
            paths: vec!["src/target.rs".to_owned()],
        },
        include_tests: false,
        include_imports: false,
        include_neighbors: false,
        include_ranking_evidence: true,
        ..ContextRequest::default()
    }
}

#[test]
fn fresh_experience_edge_adds_shortcut_with_provenance() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = repo_root_for(&dir);
    let mut store = open_store();
    seed_test_fixture(&mut store);
    let mut session = SessionStore::open(":memory:").unwrap();
    let repo_id = atlas_repo::stable_repo_id(camino::Utf8Path::new(&repo));
    let target_uri = format!("atlas://repo/{repo_id}/file/src%2Ftarget.rs");
    let test_uri = format!("atlas://repo/{repo_id}/file/tests%2Ftarget_test.rs");
    session
        .record_experience_edge(&test_edge(&repo, &target_uri, &test_uri))
        .unwrap();

    let engine = experience_engine(&store, &session, &repo);
    let result = engine.build(&refactor_request()).unwrap();
    let shortcut = result
        .nodes
        .iter()
        .find(|node| node.selection_reason == SelectionReason::ExperienceShortcut)
        .expect("experience shortcut");
    let evidence = shortcut
        .context_ranking_evidence
        .as_ref()
        .and_then(|evidence| evidence.experience_shortcut.as_ref())
        .expect("shortcut provenance");
    assert_eq!(evidence.commit_sha, "abc123");
    assert_eq!(evidence.confidence, 0.9);
    assert!(!evidence.edge_id.is_empty());
    assert_eq!(evidence.trace_summary, "query_graph -> target");
}

#[test]
fn fresh_experience_edge_reduces_escalation_rounds() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = repo_root_for(&dir);
    let mut store = open_store();
    seed_test_fixture(&mut store);
    let mut session = SessionStore::open(":memory:").unwrap();
    let repo_id = atlas_repo::stable_repo_id(camino::Utf8Path::new(&repo));
    let target_uri = format!("atlas://repo/{repo_id}/file/src%2Ftarget.rs");
    let test_uri = format!("atlas://repo/{repo_id}/file/tests%2Ftarget_test.rs");
    session
        .record_experience_edge(&test_edge(&repo, &target_uri, &test_uri))
        .unwrap();

    let seeded = experience_engine(&store, &session, &repo)
        .build(&refactor_request())
        .unwrap();
    let seeded_report = seeded.sufficiency.expect("report");
    assert!(seeded_report.tests_covered);
    assert_eq!(seeded_report.rounds_used, 0);

    let plain = ContextEngine::new(&store)
        .with_escalation(true, 2)
        .build(&refactor_request())
        .unwrap();
    let plain_report = plain.sufficiency.expect("report");
    assert!(!plain_report.tests_covered);
    assert!(plain_report.rounds_used >= 1);
}

#[test]
fn stale_experience_edge_is_not_surfaced() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = repo_root_for(&dir);
    let mut store = open_store();
    seed_test_fixture(&mut store);
    let mut session = SessionStore::open(":memory:").unwrap();
    let repo_id = atlas_repo::stable_repo_id(camino::Utf8Path::new(&repo));
    let target_uri = format!("atlas://repo/{repo_id}/file/src%2Ftarget.rs");
    let test_uri = format!("atlas://repo/{repo_id}/file/tests%2Ftarget_test.rs");
    session
        .record_experience_edge(&test_edge(&repo, &target_uri, &test_uri))
        .unwrap();
    session
        .invalidate_experience_paths(&repo, &[("src/target.rs".to_owned(), None)])
        .unwrap();

    let engine = experience_engine(&store, &session, &repo);
    let result = engine.build(&refactor_request()).unwrap();
    assert!(
        result
            .nodes
            .iter()
            .all(|node| node.selection_reason != SelectionReason::ExperienceShortcut),
        "stale edge must not surface as a shortcut"
    );
}

#[test]
fn experience_shortcut_evidence_struct_matches_report_contract() {
    let evidence = ExperienceShortcutEvidence {
        edge_id: "edge-1".to_owned(),
        confidence: 0.5,
        trace_summary: "trace".to_owned(),
        commit_sha: "sha".to_owned(),
    };
    let value = serde_json::to_value(&evidence).unwrap();
    assert_eq!(value["edge_id"], "edge-1");
}

#[test]
fn sufficiency_report_exposes_required_fields() {
    let report = EvidenceSufficiencyReport {
        target_resolved: true,
        callers_complete: true,
        callees_complete: true,
        tests_covered: true,
        dispatch_complete: true,
        overview_fresh: true,
        evidence_gaps: vec![],
        sufficiency_score: 1.0,
        escalation_reason: None,
        next_tools: vec![],
        rounds_used: 0,
        skipped_reason: None,
    };
    let value = serde_json::to_value(&report).unwrap();
    for key in [
        "target_resolved",
        "callers_complete",
        "callees_complete",
        "tests_covered",
        "dispatch_complete",
        "overview_fresh",
    ] {
        assert!(value.get(key).is_some(), "missing {key}");
    }
}

#[test]
fn empty_result_helper_is_stable() {
    let result = empty_result(request(ContextIntent::Symbol));
    assert!(result.nodes.is_empty());
}
