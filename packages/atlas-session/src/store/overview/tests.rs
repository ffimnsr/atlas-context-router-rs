use tempfile::TempDir;

use super::*;
use crate::store::{DEFAULT_SESSION_DB, SessionStore, SessionStoreConfig};

fn open_store() -> (TempDir, SessionStore) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".atlas").join(DEFAULT_SESSION_DB);
    let store =
        SessionStore::open_with_config(path.to_str().unwrap(), SessionStoreConfig::default())
            .unwrap();
    (dir, store)
}

fn repo_root(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_owned()
}

fn file_subject(repo: &str, path: &str) -> OverviewSubject {
    OverviewSubject {
        kind: OverviewSubjectKind::File,
        repo_root: repo.to_owned(),
        commit_sha: "abc123".to_owned(),
        package_name: None,
        module_path: None,
        canonical_file_path: Some(path.to_owned()),
        qualified_name: None,
        node_kind: None,
        line_start: None,
        line_end: None,
        content_hash: None,
    }
}

fn function_subject(repo: &str, qname: &str, path: &str) -> OverviewSubject {
    OverviewSubject {
        kind: OverviewSubjectKind::Function,
        repo_root: repo.to_owned(),
        commit_sha: "abc123".to_owned(),
        package_name: None,
        module_path: None,
        canonical_file_path: Some(path.to_owned()),
        qualified_name: Some(qname.to_owned()),
        node_kind: Some("function".to_owned()),
        line_start: Some(10),
        line_end: Some(20),
        content_hash: None,
    }
}

fn new_record(subject: OverviewSubject, summary: &str) -> NewOverviewRecord {
    NewOverviewRecord {
        subject,
        title: "Parsing pipeline".to_owned(),
        summary: summary.to_owned(),
        description_preview: "bounded preview".to_owned(),
        responsibilities: vec!["parse".to_owned()],
        flow: vec![],
        inputs: vec![],
        outputs: vec![],
        invariants: vec!["deterministic".to_owned()],
        gotchas: vec![],
        examples: vec![],
        tags: vec!["parser".to_owned()],
        source_kind: OverviewSourceKind::Manual,
        analyzer_name: None,
        analyzer_version: None,
        model_name: None,
        graph_last_indexed_at: None,
        freshness: Some(OverviewFreshness::Fresh),
        confidence: 0.9,
        source_id: None,
        supersedes_id: None,
        metadata: serde_json::json!({}),
    }
}

#[test]
fn overview_enums_parse_exact_values_and_reject_unknown() {
    for value in [
        "project", "package", "module", "file", "symbol", "function", "method",
    ] {
        assert_eq!(
            value.parse::<OverviewSubjectKind>().unwrap().as_str(),
            value
        );
    }
    for value in ["fresh", "possibly_stale", "stale", "orphaned", "unverified"] {
        assert_eq!(value.parse::<OverviewFreshness>().unwrap().as_str(), value);
    }
    for value in ["external_llm", "manual", "imported"] {
        assert_eq!(value.parse::<OverviewSourceKind>().unwrap().as_str(), value);
    }
    assert!(("widget".parse::<OverviewSubjectKind>()).is_err());
    assert!(("unknown".parse::<OverviewFreshness>()).is_err());
    assert!(("widget".parse::<OverviewSourceKind>()).is_err());
}

#[test]
fn overview_subject_validation_enforces_required_fields() {
    let project = OverviewSubject {
        kind: OverviewSubjectKind::Project,
        repo_root: "/repo".to_owned(),
        commit_sha: "abc".to_owned(),
        package_name: None,
        module_path: None,
        canonical_file_path: None,
        qualified_name: None,
        node_kind: None,
        line_start: None,
        line_end: None,
        content_hash: None,
    };
    assert!(project.clone().validated().is_ok());

    let package = OverviewSubject {
        kind: OverviewSubjectKind::Package,
        package_name: None,
        ..project.clone()
    };
    assert!(package.validated().is_err());

    let module = OverviewSubject {
        kind: OverviewSubjectKind::Module,
        module_path: None,
        ..project.clone()
    };
    assert!(module.validated().is_err());

    let file = OverviewSubject {
        kind: OverviewSubjectKind::File,
        canonical_file_path: None,
        ..project.clone()
    };
    assert!(file.validated().is_err());

    let symbol = OverviewSubject {
        kind: OverviewSubjectKind::Symbol,
        qualified_name: None,
        canonical_file_path: Some("src/lib.rs".to_owned()),
        line_start: Some(1),
        line_end: Some(2),
        ..project.clone()
    };
    assert!(symbol.validated().is_err());

    let bad_span = OverviewSubject {
        kind: OverviewSubjectKind::Function,
        qualified_name: Some("src/lib.rs::fn::compute".to_owned()),
        canonical_file_path: Some("src/lib.rs".to_owned()),
        line_start: Some(10),
        line_end: Some(5),
        ..project
    };
    assert!(bad_span.validated().is_err());
}

#[test]
fn overview_subject_path_canonicalization_matches_identity() {
    let (dir, _store) = open_store();
    let repo = repo_root(&dir);

    let dotted = file_subject(&repo, "./src/lib.rs").validated().unwrap();
    let plain = file_subject(&repo, "src/lib.rs").validated().unwrap();

    assert_eq!(dotted.canonical_file_path, plain.canonical_file_path);
    assert_eq!(subject_id(&dotted), subject_id(&plain));
}

#[test]
fn overview_budget_truncates_fields_and_reports_metadata() {
    let mut record = new_record(
        file_subject("/repo", "src/lib.rs"),
        &"s".repeat(OVERVIEW_SUMMARY_MAX_CHARS + 10),
    );
    record.title = "t".repeat(OVERVIEW_TITLE_MAX_CHARS + 5);
    record.responsibilities = (0..OVERVIEW_LIST_MAX_ITEMS + 3)
        .map(|index| format!("item-{index}"))
        .collect();
    let meta = bound_new_overview_record(&mut record).unwrap();

    assert!(meta.title_truncated);
    assert!(meta.summary_truncated);
    assert_eq!(record.responsibilities.len(), OVERVIEW_LIST_MAX_ITEMS);
    assert_eq!(meta.omitted_list_items, 3);
}

#[test]
fn overview_budget_rejects_bad_confidence_and_missing_summary() {
    let mut record = new_record(file_subject("/repo", "src/lib.rs"), "");
    assert!(bound_new_overview_record(&mut record).is_err());

    let mut record = new_record(file_subject("/repo", "src/lib.rs"), "summary");
    record.confidence = 1.5;
    assert!(bound_new_overview_record(&mut record).is_err());
}

#[test]
fn overview_tables_match_required_schema() {
    let (_dir, store) = open_store();
    let columns = table_columns(&store, "overview_records");
    for required in [
        "id",
        "repo_root",
        "subject_id",
        "subject_kind",
        "package_name",
        "module_path",
        "canonical_file_path",
        "qualified_name",
        "node_kind",
        "line_start",
        "line_end",
        "content_hash",
        "title",
        "summary",
        "description_preview",
        "responsibilities_json",
        "flow_json",
        "inputs_json",
        "outputs_json",
        "invariants_json",
        "gotchas_json",
        "examples_json",
        "tags_json",
        "source_kind",
        "analyzer_name",
        "analyzer_version",
        "model_name",
        "commit_sha",
        "graph_last_indexed_at",
        "freshness",
        "confidence",
        "source_id",
        "supersedes_id",
        "created_at",
        "updated_at",
        "metadata_json",
    ] {
        assert!(columns.contains(&required.to_owned()), "missing {required}");
    }
    assert!(table_columns(&store, "overview_record_fts").contains(&"title".to_owned()));
    assert!(table_columns(&store, "overview_packets").contains(&"packet_id".to_owned()));
}

#[test]
fn overview_insert_supersedes_previous_active_record() {
    let (dir, mut store) = open_store();
    let repo = repo_root(&dir);
    let subject = file_subject(&repo, "src/lib.rs");

    let first = store
        .insert_overview_record(&new_record(subject.clone(), "first"))
        .unwrap();
    let second = store
        .insert_overview_record(&new_record(subject.clone(), "second"))
        .unwrap();

    assert_eq!(second.supersedes_id.as_deref(), Some(first.id.as_str()));
    let active = store
        .list_overview_records(&repo, &OverviewListFilter::default())
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, second.id);
    assert!(store.overview_schema_issues().is_empty());
}

#[test]
fn overview_search_ranks_exact_subject_match_first() {
    let (dir, mut store) = open_store();
    let repo = repo_root(&dir);

    let mut file_record = new_record(file_subject(&repo, "src/parser.rs"), "parser summary");
    file_record.title = "Parser module".to_owned();
    store.insert_overview_record(&file_record).unwrap();

    let mut fn_record = new_record(
        function_subject(&repo, "src/parser.rs::fn::parse", "src/parser.rs"),
        "parse function summary",
    );
    fn_record.title = "parse".to_owned();
    fn_record.tags = vec!["parser".to_owned(), "parse".to_owned()];
    store.insert_overview_record(&fn_record).unwrap();

    let hits = store
        .search_overview_records(
            &repo,
            "src/parser.rs::fn::parse",
            &OverviewSearchFilter::default(),
        )
        .unwrap();
    assert!(!hits.is_empty());
    assert_eq!(
        hits[0].record.qualified_name.as_deref(),
        Some("src/parser.rs::fn::parse")
    );
    assert!(hits[0].ranking.exact_subject_match);

    let hits = store
        .search_overview_records(&repo, "summary", &OverviewSearchFilter::default())
        .unwrap();
    assert_eq!(hits.len(), 2);
}

#[test]
fn overview_context_selection_emits_reasons_and_stale_metadata() {
    let (dir, mut store) = open_store();
    let repo = repo_root(&dir);

    let file = store
        .insert_overview_record(&new_record(
            file_subject(&repo, "src/lib.rs"),
            "file summary",
        ))
        .unwrap();
    store
        .insert_overview_record(&new_record(
            function_subject(&repo, "src/lib.rs::fn::compute", "src/lib.rs"),
            "function summary",
        ))
        .unwrap();
    store
        .mark_overview_freshness(&file.id, OverviewFreshness::Stale)
        .unwrap();

    let fresh_only = store
        .select_overviews_for_context(&OverviewContextQuery {
            repo_root: repo.clone(),
            qualified_names: vec!["src/lib.rs::fn::compute".to_owned()],
            file_paths: vec!["src/lib.rs".to_owned()],
            include_stale: false,
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(fresh_only.len(), 1);
    assert_eq!(fresh_only[0].selection_reason, "same_symbol");

    let with_stale = store
        .select_overviews_for_context(&OverviewContextQuery {
            repo_root: repo,
            qualified_names: vec!["src/lib.rs::fn::compute".to_owned()],
            file_paths: vec!["src/lib.rs".to_owned()],
            include_stale: true,
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(with_stale.len(), 2);
    let stale = with_stale
        .iter()
        .find(|hit| hit.record.id == file.id)
        .unwrap();
    assert_eq!(stale.selection_reason, "same_file");
    assert!(stale.stale_reason.is_some());
}

fn table_columns(store: &SessionStore, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA table_info('{table}')");
    let mut stmt = store.conn.prepare(&sql).unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn overview_delete_dry_run_reports_without_deleting() {
    let (dir, mut store) = open_store();
    let repo = repo_root(&dir);
    let record = store
        .insert_overview_record(&new_record(file_subject(&repo, "src/lib.rs"), "summary"))
        .unwrap();

    let dry = store.delete_overview_record(&record.id, true).unwrap();
    assert!(dry.found);
    assert!(!dry.deleted);
    assert!(store.get_overview_record(&record.id).unwrap().is_some());

    let deleted = store.delete_overview_record(&record.id, false).unwrap();
    assert!(deleted.deleted);
    assert!(store.get_overview_record(&record.id).unwrap().is_none());
}

#[test]
fn overview_schema_issues_detects_noncanonical_paths() {
    let (dir, mut store) = open_store();
    let repo = repo_root(&dir);
    let record = store
        .insert_overview_record(&new_record(file_subject(&repo, "src/lib.rs"), "summary"))
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE overview_records SET canonical_file_path = './src/lib.rs' WHERE id = ?1",
            rusqlite::params![record.id],
        )
        .unwrap();
    let issues = store.overview_schema_issues();
    assert!(
        issues
            .iter()
            .any(|issue| issue.starts_with("noncanonical_path_rows")),
        "{issues:?}"
    );
}
