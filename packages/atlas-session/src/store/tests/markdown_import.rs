use std::path::{Path, PathBuf};

use atlas_store_markdown::MarkdownRecord;

use super::*;

fn db_path(dir: &TempDir) -> String {
    dir.path()
        .join(".atlas")
        .join(DEFAULT_SESSION_DB)
        .to_string_lossy()
        .into_owned()
}

fn decision_event(session: &SessionId, summary: &str, conclusion: &str) -> NewSessionEvent {
    NewSessionEvent {
        session_id: session.clone(),
        event_type: SessionEventType::Decision,
        priority: 50,
        payload: serde_json::json!({
            "decision": summary,
            "summary": summary,
            "rationale": "version control",
            "conclusion": conclusion,
        }),
        created_at: None,
    }
}

#[test]
fn hand_edited_decision_body_is_imported_on_reopen() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let session = session_id();

    {
        let mut store = SessionStore::open(&path).unwrap();
        seed_session(&mut store, &session);
        store
            .append_event(decision_event(&session, "markdown truth", "adopt"))
            .unwrap();

        let decision_id: String = store
            .conn
            .query_row(
                "SELECT decision_id FROM decision_memory LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let relative = PathBuf::from(format!("decisions/{decision_id}.md"));
        let record = store.markdown.read_record(&relative).unwrap();
        assert!(record.body.contains("adopt"));
        let edited = record.body.replace("adopt", "revised");
        store
            .markdown
            .write_record(&relative, &MarkdownRecord::new(record.frontmatter, edited))
            .unwrap();
    }

    let store = SessionStore::open(&path).unwrap();
    let hits = store
        .search_decisions("/repo", "markdown truth", None, 10)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].decision.conclusion.as_deref(), Some("revised"));
}

#[test]
fn deleted_session_db_rebuilds_durable_families_from_markdown() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let session = session_id();

    {
        let mut store = SessionStore::open(&path).unwrap();
        seed_session(&mut store, &session);
        store
            .append_event(decision_event(&session, "durable rebuild", "restored"))
            .unwrap();
        store
            .store_feedback(&NewFeedback {
                repo_root: "/repo".to_owned(),
                session_id: Some(session.as_str().to_owned()),
                tool_name: "cli".to_owned(),
                analysis_kind: "dead_code".to_owned(),
                predicted: "helper is dead".to_owned(),
                actual: "helper is used".to_owned(),
                correction: "keep the helper".to_owned(),
                related_symbol: None,
                related_file: None,
                source_id: None,
                metadata: serde_json::json!({}),
            })
            .unwrap();
        store
            .create_durable_task(&NewDurableTask {
                task_id: "task-rebuild".to_owned(),
                originating_method: "tools/call".to_owned(),
                request_id: None,
                tool_name: Some("doctor".to_owned()),
                transport_kind: Some("stdio".to_owned()),
                session_id: None,
                status: DurableTaskStatus::Working,
                status_message: Some("running".to_owned()),
                ttl_ms: None,
            })
            .unwrap();
        store
            .record_experience_edge(&NewExperienceEdge {
                repo_root: "/repo".to_owned(),
                source_uri: "atlas://symbol/a".to_owned(),
                target_uri: "atlas://file/b.rs".to_owned(),
                query_text: "find a".to_owned(),
                trace_summary: "a depends on b".to_owned(),
                commit_sha: "abc".to_owned(),
                source_file_hash: None,
                target_file_hash: None,
                confidence: 0.9,
                query_embedding: None,
            })
            .unwrap();
        store
            .create_memoir_graph(&NewMemoirGraph {
                repo_root: "/repo".to_owned(),
                scope: MemoryScope::Project,
                name: "rebuild-graph".to_owned(),
                description: "graph".to_owned(),
                metadata: serde_json::json!({}),
            })
            .unwrap();
        for name in ["alpha", "beta"] {
            store
                .add_memoir_concept(&NewMemoirConcept {
                    repo_root: "/repo".to_owned(),
                    scope: MemoryScope::Project,
                    graph: "rebuild-graph".to_owned(),
                    name: name.to_owned(),
                    description: format!("{name} description"),
                    kind: "concept".to_owned(),
                    source_id: None,
                    metadata: serde_json::json!({}),
                })
                .unwrap();
        }
        store
            .link_memoir_concepts(&NewMemoirRelation {
                repo_root: "/repo".to_owned(),
                scope: MemoryScope::Project,
                graph: "rebuild-graph".to_owned(),
                source: "alpha".to_owned(),
                target: "beta".to_owned(),
                relation: "depends_on".to_owned(),
                confidence: 0.8,
                source_id: None,
                metadata: serde_json::json!({}),
                create_missing: false,
            })
            .unwrap();
        store
            .insert_overview_record(&NewOverviewRecord {
                subject: OverviewSubject {
                    kind: OverviewSubjectKind::File,
                    repo_root: "/repo".to_owned(),
                    commit_sha: "abc".to_owned(),
                    package_name: None,
                    module_path: None,
                    canonical_file_path: Some("src/lib.rs".to_owned()),
                    qualified_name: None,
                    node_kind: None,
                    line_start: None,
                    line_end: None,
                    content_hash: None,
                },
                title: "Rebuild plan".to_owned(),
                summary: "rebuild overview".to_owned(),
                description_preview: "overview body".to_owned(),
                responsibilities: vec!["parse".to_owned()],
                flow: Vec::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
                invariants: Vec::new(),
                gotchas: Vec::new(),
                examples: Vec::new(),
                tags: Vec::new(),
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
            })
            .unwrap();
        store
            .put_resume_snapshot(&session, "{\"summary\":\"resume\"}", 3, false)
            .unwrap();

        let alpha = store
            .markdown
            .read_record(Path::new("memoir/rebuild-graph/alpha.md"))
            .unwrap();
        assert_eq!(
            alpha.frontmatter.get_list("relations").unwrap().len(),
            1,
            "relation must be written before db deletion: {:?}",
            alpha.frontmatter
        );
    }

    std::fs::remove_file(&path).unwrap();
    let store = SessionStore::open(&path).unwrap();

    let decisions = store
        .search_decisions("/repo", "durable rebuild", None, 10)
        .unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        decisions[0].decision.conclusion.as_deref(),
        Some("restored")
    );

    let feedback = store
        .search_feedback("/repo", "helper", &Default::default(), 10)
        .unwrap();
    assert_eq!(feedback.len(), 1);

    let tasks = store.list_durable_tasks(None, 10).unwrap();
    assert!(
        tasks
            .tasks
            .iter()
            .any(|task| task.task_id == "task-rebuild"),
        "tasks rebuild from markdown"
    );

    let edges = store
        .inspect_experience_edges("/repo", "atlas://symbol/a", ExperienceDirection::Both, 10)
        .unwrap();
    assert_eq!(edges.len(), 1);

    let overview = store
        .search_overview_records("/repo", "rebuild", &Default::default())
        .unwrap();
    assert_eq!(overview.len(), 1);

    let inspection = store
        .inspect_memoir_concept(&MemoirInspectQuery {
            repo_root: "/repo".to_owned(),
            graph: Some("rebuild-graph".to_owned()),
            concept: "alpha".to_owned(),
            scope: None,
            depth: 2,
            relation: None,
        })
        .unwrap();
    assert_eq!(inspection.edges.len(), 1);

    let events = store.list_events(&session).unwrap();
    assert_eq!(events.len(), 1);
    let resume = store.get_resume_snapshot(&session).unwrap().unwrap();
    assert_eq!(resume.event_count, 3);
}

#[test]
fn migrate_dry_run_keeps_pending_rows_out_of_markdown() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let config = SessionStoreConfig {
        markdown_reconcile_on_open: false,
        ..Default::default()
    };
    {
        let store = SessionStore::open_with_config(&path, config.clone()).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO memory_index
                    (id, repo_root, scope, topic, title, body, importance,
                     created_at, updated_at, metadata_json, op_history_json)
                 VALUES ('pending-md', '/repo', 'project', 'migration', 'pending',
                         'pending body', 'normal',
                         '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '{}', '[]')",
                [],
            )
            .unwrap();
    }

    let mut store = SessionStore::open_with_config(&path, config).unwrap();
    assert!(
        !store
            .markdown
            .absolute(Path::new("memories/migration/pending-md.md"))
            .exists(),
        "opening for migration must not export pending rows"
    );
    let dry = store.migrate_markdown(true, false).unwrap();
    assert_eq!(dry.written.len(), 1);
    assert!(
        !store
            .markdown
            .absolute(Path::new("memories/migration/pending-md.md"))
            .exists(),
        "dry run must not write markdown"
    );
}

#[test]
fn memoir_delete_removes_markdown_directory() {
    let dir = TempDir::new().unwrap();
    let path = db_path(&dir);
    let mut store = SessionStore::open(&path).unwrap();
    store
        .create_memoir_graph(&NewMemoirGraph {
            repo_root: "/repo".to_owned(),
            scope: MemoryScope::Project,
            name: "delete-me".to_owned(),
            description: "graph".to_owned(),
            metadata: serde_json::json!({}),
        })
        .unwrap();
    let graph_dir = store.markdown.absolute(Path::new("memoir/delete-me"));
    assert!(graph_dir.exists());

    let result = store
        .delete_memoir_graph("/repo", MemoryScope::Project, "delete-me", false)
        .unwrap();
    assert!(result.deleted);
    assert!(
        !graph_dir.exists(),
        "memoir delete must remove the committed markdown directory"
    );
}
