//! ICM-G service tests: export boundaries, ingest freshness, refresh planning.

use atlas_adapters::RedactionRules;
use atlas_contentstore::ContentStore;
use atlas_core::model::{Edge, Node, NodeId, ParsedFile};
use atlas_core::{EdgeKind, NodeKind};
use atlas_session::{
    OverviewAnalysisInput, OverviewExportRequest, OverviewExportScope, OverviewFreshness,
    OverviewSourceKind, OverviewSubject, OverviewSubjectInput, OverviewSubjectKind, SessionStore,
};
use atlas_store_sqlite::Store;
use tempfile::TempDir;

use super::{export_overview_packets, ingest_overview_analysis, refresh_overview_plan, util};
use crate::overview::types::OverviewRefreshPlan;

struct Fixture {
    _dir: TempDir,
    repo_root: String,
    graph_db: String,
    store: Store,
    session: SessionStore,
    content: ContentStore,
}

fn node(qname: &str, file: &str, kind: NodeKind, start: u32, end: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: qname.rsplit("::").next().unwrap_or(qname).to_owned(),
        qualified_name: qname.to_owned(),
        file_path: file.to_owned(),
        line_start: start,
        line_end: end,
        language: "rust".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: format!("hash-{file}"),
        extra_json: serde_json::json!({"owner_name": "src"}),
        repo_provenance: None,
    }
}

fn edge(source: &str, target: &str, file: &str) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: source.to_owned(),
        target_qn: target.to_owned(),
        file_path: file.to_owned(),
        line: None,
        confidence: 1.0,
        confidence_tier: None,
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn setup() -> Fixture {
    let dir = TempDir::new().unwrap();
    let repo_root = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let source = (1..=40)
        .map(|line| format!("// line {line}\npub fn f{line}() {{}}\n"))
        .collect::<String>();
    std::fs::write(dir.path().join("src/a.rs"), source).unwrap();
    std::fs::write(dir.path().join("src/b.rs"), "pub fn g() {}\n").unwrap();

    let graph_db = dir
        .path()
        .join(".atlas")
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();
    let mut store = Store::open(&graph_db).unwrap();
    store.migrate().unwrap();
    let nodes = [
        node(
            "src/a.rs::fn::compute",
            "src/a.rs",
            NodeKind::Function,
            3,
            4,
        ),
        node("src/a.rs::fn::helper", "src/a.rs", NodeKind::Function, 5, 6),
        node("src/b.rs::fn::caller", "src/b.rs", NodeKind::Function, 1, 1),
    ];
    let edges = [
        edge("src/a.rs::fn::compute", "src/a.rs::fn::helper", "src/a.rs"),
        edge("src/b.rs::fn::caller", "src/a.rs::fn::compute", "src/b.rs"),
    ];
    let files = vec![
        ParsedFile {
            path: "src/a.rs".to_owned(),
            language: Some("rust".to_owned()),
            hash: "hash-src/a.rs".to_owned(),
            size: None,
            nodes: nodes[0..2].to_vec(),
            edges: edges[0..1].to_vec(),
        },
        ParsedFile {
            path: "src/b.rs".to_owned(),
            language: Some("rust".to_owned()),
            hash: "hash-src/b.rs".to_owned(),
            size: None,
            nodes: nodes[2..3].to_vec(),
            edges: edges[1..2].to_vec(),
        },
    ];
    store
        .replace_files_transactional_for_repo("repo-demo", &files)
        .unwrap();

    let session = SessionStore::open(
        dir.path()
            .join(".atlas")
            .join("session.db")
            .to_str()
            .unwrap(),
    )
    .unwrap();
    let content = ContentStore::open(
        dir.path()
            .join(".atlas")
            .join("context.db")
            .to_str()
            .unwrap(),
    )
    .unwrap();

    Fixture {
        _dir: dir,
        repo_root,
        graph_db,
        store,
        session,
        content,
    }
}

fn export_request(scope: OverviewExportScope) -> OverviewExportRequest {
    OverviewExportRequest {
        scope,
        limit: Some(10),
        include_callers: true,
        include_callees: true,
        include_tests: true,
        ..OverviewExportRequest::default()
    }
}

#[test]
fn project_export_emits_subject_identity_and_stale_metadata() {
    let mut fixture = setup();
    let result = export_overview_packets(
        &fixture.repo_root,
        &fixture.graph_db,
        &fixture.store,
        Some(&mut fixture.session),
        &export_request(OverviewExportScope::Project),
    )
    .unwrap();
    assert_eq!(result.packets.len(), 1);
    let packet = &result.packets[0];
    assert_eq!(packet.subject.kind, OverviewSubjectKind::Project);
    assert_eq!(
        packet.subject_id,
        atlas_session::subject_id(&packet.subject)
    );
    assert_eq!(packet.commit_sha, result.commit_sha);
    assert!(!packet.packet_id.is_empty());
    // No graph build status in the fixture: freshness must be explicit, never
    // reported as silently fresh.
    assert_eq!(result.freshness.state, "unverified");
    assert!(!result.freshness.safe_to_answer);
}

#[test]
fn function_export_includes_callers_callees_and_bounded_excerpt() {
    let mut fixture = setup();
    let request = OverviewExportRequest {
        scope: OverviewExportScope::Functions,
        subjects: vec!["src/a.rs::fn::compute".to_owned()],
        include_code_spans: true,
        ..export_request(OverviewExportScope::Functions)
    };
    let result = export_overview_packets(
        &fixture.repo_root,
        &fixture.graph_db,
        &fixture.store,
        Some(&mut fixture.session),
        &request,
    )
    .unwrap();
    assert_eq!(result.packets.len(), 1);
    let packet = &result.packets[0];
    assert_eq!(packet.subject.kind, OverviewSubjectKind::Function);
    assert!(packet.callers.contains(&"src/b.rs::fn::caller".to_owned()));
    assert!(packet.callees.contains(&"src/a.rs::fn::helper".to_owned()));
    let excerpt = packet.code_excerpt.as_ref().expect("code excerpt");
    assert!(excerpt.text.contains("pub fn f2"));
}

#[test]
fn package_export_and_ambiguous_subject_failure() {
    let mut fixture = setup();
    let packages = export_overview_packets(
        &fixture.repo_root,
        &fixture.graph_db,
        &fixture.store,
        Some(&mut fixture.session),
        &export_request(OverviewExportScope::Packages),
    )
    .unwrap();
    assert_eq!(packages.packets.len(), 1);
    assert_eq!(
        packages.packets[0].subject.package_name.as_deref(),
        Some("src")
    );

    let mut request = export_request(OverviewExportScope::Project);
    request.subjects = vec!["src".to_owned()];
    let error = export_overview_packets(
        &fixture.repo_root,
        &fixture.graph_db,
        &fixture.store,
        Some(&mut fixture.session),
        &request,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("ambiguous"), "{error}");
}

#[test]
fn export_budget_truncation_sets_budget_metadata() {
    let mut fixture = setup();
    let request = OverviewExportRequest {
        scope: OverviewExportScope::Functions,
        include_code_spans: true,
        max_tokens: Some(256),
        ..export_request(OverviewExportScope::Functions)
    };
    let result = export_overview_packets(
        &fixture.repo_root,
        &fixture.graph_db,
        &fixture.store,
        Some(&mut fixture.session),
        &request,
    )
    .unwrap();
    assert!(!result.packets.is_empty());
    assert!(result.budget.max_tokens == 256);
    // Packet-level degradation must roll up into the export budget flag.
    assert!(
        result.budget.budget_hit,
        "packet budget hit must roll up into the export budget"
    );
    assert!(
        result.budget.omitted_bytes > 0,
        "packet omissions must roll up into omitted_bytes"
    );
}

#[test]
fn changed_export_is_empty_without_git_and_clear_with_bad_since() {
    let mut fixture = setup();
    let request = OverviewExportRequest {
        scope: OverviewExportScope::Changed,
        ..export_request(OverviewExportScope::Changed)
    };
    let result = export_overview_packets(
        &fixture.repo_root,
        &fixture.graph_db,
        &fixture.store,
        Some(&mut fixture.session),
        &request,
    )
    .unwrap();
    assert!(result.packets.is_empty());

    let request = OverviewExportRequest {
        since: Some("definitely-not-a-ref".to_owned()),
        ..request
    };
    assert!(
        export_overview_packets(
            &fixture.repo_root,
            &fixture.graph_db,
            &fixture.store,
            Some(&mut fixture.session),
            &request,
        )
        .is_err()
    );
}

fn analysis(repo_root: &str, subject: OverviewSubject, commit_sha: &str) -> OverviewAnalysisInput {
    OverviewAnalysisInput {
        schema_version: 1,
        packet_id: None,
        evidence_ids: vec![],
        subject: OverviewSubject {
            commit_sha: commit_sha.to_owned(),
            ..subject
        }
        .into(),
        title: "Function overview".to_owned(),
        summary: "bounded summary".to_owned(),
        description: "bounded description".to_owned(),
        responsibilities: vec!["computes".to_owned()],
        flow: vec![],
        inputs: vec![],
        outputs: vec![],
        invariants: vec!["deterministic".to_owned()],
        gotchas: vec![],
        examples: vec![],
        tags: vec!["function".to_owned()],
        confidence: 0.7,
        source_kind: OverviewSourceKind::ExternalLlm,
        analyzer_name: Some("fixture".to_owned()),
        analyzer_version: Some("1".to_owned()),
        model_name: None,
        source_ids: vec![],
        metadata: serde_json::json!({"repo": repo_root}),
    }
}

fn compute_subject(repo_root: &str) -> OverviewSubject {
    OverviewSubject {
        kind: OverviewSubjectKind::Function,
        repo_root: repo_root.to_owned(),
        commit_sha: "no-head".to_owned(),
        package_name: None,
        module_path: Some("src".to_owned()),
        canonical_file_path: Some("src/a.rs".to_owned()),
        qualified_name: Some("src/a.rs::fn::compute".to_owned()),
        node_kind: Some("function".to_owned()),
        line_start: Some(3),
        line_end: Some(4),
        content_hash: None,
    }
}

#[test]
fn ingest_accepts_string_subject_references() {
    let mut fixture = setup();
    let rules = RedactionRules::default();
    let mut input = analysis(
        &fixture.repo_root,
        compute_subject(&fixture.repo_root),
        "no-head",
    );
    input.subject = OverviewSubjectInput::Reference("src/a.rs".to_owned());

    let result = ingest_overview_analysis(
        &fixture.repo_root,
        &fixture.store,
        &mut fixture.session,
        &mut fixture.content,
        &rules,
        &input,
    )
    .unwrap();
    assert_eq!(result.record.subject_kind, OverviewSubjectKind::File);
    assert_eq!(
        result.record.canonical_file_path.as_deref(),
        Some("src/a.rs")
    );
    assert_eq!(result.record.commit_sha, "no-head");
}

#[test]
fn ingest_marks_fresh_stale_and_orphaned_states() {
    let mut fixture = setup();
    let rules = RedactionRules::default();

    let fresh = ingest_overview_analysis(
        &fixture.repo_root,
        &fixture.store,
        &mut fixture.session,
        &mut fixture.content,
        &rules,
        &analysis(
            &fixture.repo_root,
            compute_subject(&fixture.repo_root),
            "no-head",
        ),
    )
    .unwrap();
    assert_eq!(fresh.record.freshness, OverviewFreshness::Fresh);
    assert_eq!(fresh.status, "fresh");

    let stale = ingest_overview_analysis(
        &fixture.repo_root,
        &fixture.store,
        &mut fixture.session,
        &mut fixture.content,
        &rules,
        &analysis(
            &fixture.repo_root,
            compute_subject(&fixture.repo_root),
            "older-commit",
        ),
    )
    .unwrap();
    assert_eq!(stale.record.freshness, OverviewFreshness::PossiblyStale);

    let mut orphan_subject = compute_subject(&fixture.repo_root);
    orphan_subject.qualified_name = Some("src/a.rs::fn::missing".to_owned());
    let orphan = ingest_overview_analysis(
        &fixture.repo_root,
        &fixture.store,
        &mut fixture.session,
        &mut fixture.content,
        &rules,
        &analysis(&fixture.repo_root, orphan_subject, "no-head"),
    )
    .unwrap();
    assert_eq!(orphan.record.freshness, OverviewFreshness::Orphaned);
}

#[test]
fn ingest_rejects_bad_schema_and_missing_summary() {
    let mut fixture = setup();
    let rules = RedactionRules::default();
    let mut input = analysis(
        &fixture.repo_root,
        compute_subject(&fixture.repo_root),
        "no-head",
    );
    input.schema_version = 2;
    assert!(
        ingest_overview_analysis(
            &fixture.repo_root,
            &fixture.store,
            &mut fixture.session,
            &mut fixture.content,
            &rules,
            &input,
        )
        .is_err()
    );

    let mut input = analysis(
        &fixture.repo_root,
        compute_subject(&fixture.repo_root),
        "no-head",
    );
    input.summary.clear();
    assert!(
        ingest_overview_analysis(
            &fixture.repo_root,
            &fixture.store,
            &mut fixture.session,
            &mut fixture.content,
            &rules,
            &input,
        )
        .is_err()
    );
}

#[test]
fn refresh_plan_is_read_only_by_default() {
    let mut fixture = setup();
    let record = fixture
        .session
        .insert_overview_record(&atlas_session::NewOverviewRecord {
            subject: compute_subject(&fixture.repo_root),
            title: "compute".to_owned(),
            summary: "compute summary".to_owned(),
            description_preview: "preview".to_owned(),
            responsibilities: vec![],
            flow: vec![],
            inputs: vec![],
            outputs: vec![],
            invariants: vec![],
            gotchas: vec![],
            examples: vec![],
            tags: vec![],
            source_kind: OverviewSourceKind::Manual,
            analyzer_name: None,
            analyzer_version: None,
            model_name: None,
            graph_last_indexed_at: None,
            freshness: Some(OverviewFreshness::Fresh),
            confidence: 0.7,
            source_id: None,
            supersedes_id: None,
            metadata: serde_json::json!({}),
        })
        .unwrap();

    let plan: OverviewRefreshPlan = refresh_overview_plan(
        &fixture.repo_root,
        &fixture.store,
        &mut fixture.session,
        None,
        false,
    )
    .unwrap();
    assert!(!plan.apply);
    assert_eq!(plan.applied_count, 0);
    assert_eq!(
        fixture
            .session
            .get_overview_record(&record.id)
            .unwrap()
            .unwrap()
            .freshness,
        OverviewFreshness::Fresh
    );
}

#[test]
fn graph_provenance_reports_build_state_and_indexed_ref() {
    let fixture = setup();
    let graph =
        util::graph_provenance(&fixture.store, &fixture.graph_db, &fixture.repo_root).unwrap();
    assert_eq!(graph.graph_db_path, fixture.graph_db);
    assert!(graph.indexed_file_count >= 2);
}
