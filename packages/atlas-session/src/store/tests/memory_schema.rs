use super::*;

// ── M1 — markdown truth, derived index schema ────────────────────────────────

#[test]
fn memory_index_columns_match_golden() {
    let (_dir, store) = open_store(16, 1024);

    let columns = table_columns(&store.conn, "memory_index");
    assert_eq!(
        columns,
        vec![
            "id",
            "repo_root",
            "session_id",
            "frontend",
            "scope",
            "topic",
            "title",
            "body",
            "importance",
            "created_at",
            "updated_at",
            "source_id",
            "metadata_json",
            "superseded_by",
            "tags_json",
            "links_json",
            "op_history_json",
            "numeric_value",
            "path",
        ]
    );

    let sql: String = store
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'view' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .expect("memories compatibility view");
    assert!(sql.contains("memory_access"));
}

#[test]
fn memory_index_indexes_match_golden() {
    let (_dir, store) = open_store(16, 1024);

    let indexes = {
        let mut stmt = store
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index'")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    for expected in [
        "idx_memory_index_repo_topic",
        "idx_memory_index_repo_importance",
        "idx_memory_index_repo_scope",
        "idx_memory_index_repo_session",
        "idx_memory_index_repo_updated",
        "idx_memory_index_repo_source",
        "idx_memory_index_repo_active",
        "idx_memory_index_superseded",
        "idx_memory_access_repo",
    ] {
        assert!(
            indexes.iter().any(|name| name == expected),
            "missing index {expected}"
        );
    }
}

#[test]
fn memory_schema_issues_are_empty_when_healthy() {
    let (_dir, store) = open_store(16, 1024);
    assert!(store.memory_schema_issues().is_empty());
}

#[test]
fn memory_schema_issues_detect_missing_fts_and_triggers() {
    let (_dir, store) = open_store(16, 1024);
    store
        .conn
        .execute_batch(
            "DROP TRIGGER memory_index_ai;
             DROP TRIGGER memory_index_ad;
             DROP TRIGGER memory_index_au;
             DROP TABLE memory_index_fts;",
        )
        .unwrap();
    let issues = store.memory_schema_issues();
    assert!(
        issues
            .iter()
            .any(|i| i == "missing table: memory_index_fts")
    );
    for trigger in ["memory_index_ai", "memory_index_ad", "memory_index_au"] {
        assert!(
            issues
                .iter()
                .any(|issue| issue == &format!("missing trigger: {trigger}")),
            "issues: {issues:?}"
        );
    }
}

#[test]
fn memory_schema_issues_detect_missing_index() {
    let (_dir, store) = open_store(16, 1024);
    store
        .conn
        .execute_batch("DROP INDEX idx_memory_index_repo_topic")
        .unwrap();
    let issues = store.memory_schema_issues();
    assert!(
        issues
            .iter()
            .any(|issue| issue == "missing index: idx_memory_index_repo_topic"),
        "issues: {issues:?}"
    );
}

#[test]
fn memory_schema_issues_report_malformed_frontmatter_with_path() {
    let (_dir, store) = open_store(16, 1024);
    let path = store
        .markdown
        .absolute(std::path::Path::new("memories/broken/bad.md"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "---\nid: bad\nscope: project\n---\n").unwrap();

    let issues = store.memory_schema_issues();
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("memories/broken/bad.md")),
        "issues: {issues:?}"
    );
}

#[test]
fn memory_storage_boundary_rejects_unknown_importance_and_scope() {
    let (_dir, store) = open_store(16, 1024);
    let root = store
        .markdown
        .root()
        .join("memories")
        .join("bad")
        .join("record");
    std::fs::create_dir_all(&root).unwrap();
    let base = "---\nid: bad-1\ntitle: bad\ntopic: bad\nscope: project\nimportance: normal\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\n---\nbody\n";
    let cases = [
        (
            "bad-1.md",
            base.replace("importance: normal", "importance: urgent"),
        ),
        (
            "bad-2.md",
            base.replace("scope: project", "scope: workspace"),
        ),
    ];
    for (name, content) in cases {
        std::fs::write(root.join(name), content).unwrap();
    }

    let error = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid importance"), "got: {error}");
    assert!(error.contains("invalid scope"), "got: {error}");
}

#[test]
fn memory_record_round_trips_through_frontmatter() {
    let (_dir, store) = open_store(16, 1024);
    let record = seed_memory(
        &store,
        "round-trip",
        "2026-01-02T00:00:00Z",
        "body: with colon\nand second line",
        "architecture",
        MemoryImportance::High,
        MemoryScope::Project,
    );

    let relative = super::markdown_memory::relative_path(&record.topic, &record.id);
    let raw = store.markdown.read_record(&relative).unwrap();
    let decoded =
        super::markdown_memory::decode_record(&relative, &raw, &record.repo_root).unwrap();
    assert_eq!(decoded.id, record.id);
    assert_eq!(decoded.body, record.body);
    assert_eq!(decoded.tags, record.tags);
    assert_eq!(decoded.links, record.links);

    // Canonical serialization is byte-stable: parse then serialize unchanged.
    let encoded = super::markdown_memory::encode_record(&decoded).unwrap();
    let original = std::fs::read(store.markdown.absolute(&relative)).unwrap();
    assert_eq!(encoded.to_bytes(&relative).unwrap(), original);
}

#[test]
fn memory_directory_tree_matches_golden() {
    let (dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "tree-1",
        "2026-01-01T00:00:00Z",
        "first",
        "Deploy Process",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "tree-2",
        "2026-01-02T00:00:00Z",
        "second",
        "deploy-process",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    let memories = dir.path().join(".atlas").join("memories");
    let golden = memories.join("deploy-process");
    assert!(golden.is_dir(), "slug directory must exist");
    for id in ["tree-1", "tree-2"] {
        assert!(golden.join(format!("{id}.md")).is_file(), "missing {id}.md");
    }
    assert!(!memories.join("locks").exists());
    assert!(!memories.join("tmp").exists());
}

#[test]
fn duplicate_memory_ids_are_reported_across_topic_directories() {
    let (_dir, store) = open_store(16, 1024);
    let record = seed_memory(
        &store,
        "dup-id",
        "2026-01-01T00:00:00Z",
        "body",
        "topic-one",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    let other = super::markdown_memory::relative_path("topic-two", &record.id);
    let encoded = super::markdown_memory::encode_record(&record).unwrap();
    store.markdown.write_record(&other, &encoded).unwrap();

    let duplicates = store
        .markdown
        .detect_duplicate_ids(std::path::Path::new("memories"), "id")
        .unwrap();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0].0, "dup-id");
}

#[test]
fn memory_links_validate_targets_and_relations() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_memory(
        &store,
        "link-a",
        "2026-01-01T00:00:00Z",
        "a",
        "links",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "link-b",
        "2026-01-02T00:00:00Z",
        "b",
        "links",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    let linked = store
        .link_memories("/repo", "link-a", "link-b", "refines")
        .unwrap();
    assert_eq!(linked.links.len(), 1);

    // Duplicate links collapse.
    let again = store
        .link_memories("/repo", "link-a", "link-b", "refines")
        .unwrap();
    assert_eq!(again.links.len(), 1);

    assert!(
        store
            .link_memories("/repo", "link-a", "link-a", "refines")
            .is_err()
    );
    assert!(
        store
            .link_memories("/repo", "link-a", "missing", "refines")
            .is_err()
    );
    assert!(
        store
            .link_memories("/repo", "link-a", "link-b", "bogus")
            .is_err()
    );
}

#[test]
fn supersedes_chain_leaves_only_newest_active() {
    let (_dir, mut store) = open_store(16, 1024);
    for (id, body) in [("chain-a", "a"), ("chain-b", "b"), ("chain-c", "c")] {
        seed_memory(
            &store,
            id,
            "2026-01-01T00:00:00Z",
            body,
            "chain",
            MemoryImportance::Normal,
            MemoryScope::Project,
        );
    }
    store
        .link_memories("/repo", "chain-b", "chain-a", "supersedes")
        .unwrap();
    store
        .link_memories("/repo", "chain-c", "chain-b", "supersedes")
        .unwrap();

    let visible = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "chain-c");

    let all = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                include_superseded: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(
        all.iter()
            .find(|record| record.id == "chain-a")
            .unwrap()
            .superseded_by
            .as_deref(),
        Some("chain-b")
    );
}
