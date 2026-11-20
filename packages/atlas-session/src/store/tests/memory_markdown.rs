use std::collections::BTreeMap;
use std::time::SystemTime;

use super::*;

fn seed(store: &SessionStore, id: &str, body: &str, topic: &str) -> MemoryRecord {
    seed_memory(
        store,
        id,
        "2026-01-01T00:00:00Z",
        body,
        topic,
        MemoryImportance::Normal,
        MemoryScope::Project,
    )
}

fn snapshot_tree(store: &SessionStore) -> BTreeMap<std::path::PathBuf, (Vec<u8>, SystemTime)> {
    let mut out = BTreeMap::new();
    for relative in store
        .markdown
        .list_markdown(std::path::Path::new("memories"))
        .unwrap()
    {
        let absolute = store.markdown.absolute(&relative);
        let bytes = std::fs::read(&absolute).unwrap();
        let modified = std::fs::metadata(&absolute).unwrap().modified().unwrap();
        out.insert(relative, (bytes, modified));
    }
    out
}

#[test]
fn recall_and_decay_leave_markdown_bytes_and_mtimes_untouched() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "pure-a", "alpha storage fact", "storage");
    seed(&store, "pure-b", "beta storage fact", "storage");
    let before = snapshot_tree(&store);

    let viewer = cli_viewer();
    let hits = store
        .recall_memories(
            "/repo",
            "storage",
            &MemoryListFilter::default(),
            false,
            &viewer,
            10,
        )
        .unwrap();
    assert_eq!(hits.len(), 2);

    let policy = MemoryDecayPolicy::default();
    store
        .decay_reports("/repo", &MemoryListFilter::default(), &policy, false)
        .unwrap();

    let after = snapshot_tree(&store);
    assert_eq!(before, after, "reads must not rewrite truth files");

    let access_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memory_access", [], |row| row.get(0))
        .unwrap();
    assert!(
        access_count >= 2,
        "access bookkeeping is derived-index only"
    );
}

#[test]
fn tags_filter_requires_every_tag() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "tag-a", "tagged body", "tags");
    let mut record = super::markdown_memory::load_by_id(&store.markdown, "/repo", "tag-a")
        .unwrap()
        .unwrap();
    record.tags = vec!["storage".to_owned(), "sqlite".to_owned()];
    super::markdown_memory::write_existing_record(&store.markdown, &store.conn, &record).unwrap();

    let hit = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                tags: vec!["storage".to_owned()],
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(hit.len(), 1);
    let miss = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                tags: vec!["storage".to_owned(), "missing".to_owned()],
                ..Default::default()
            },
        )
        .unwrap();
    assert!(miss.is_empty());
}

#[test]
fn duplicate_tags_collapse_on_write() {
    let (_dir, mut store) = open_store(16, 1024);
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: "tags".to_owned(),
        title: "tags".to_owned(),
        body: "duplicate tags".to_owned(),
        importance: MemoryImportance::Normal,
        source_id: None,
        tags: vec![
            "Storage".to_owned(),
            "storage".to_owned(),
            "STORAGE".to_owned(),
        ],
        links: Vec::new(),
        metadata: serde_json::json!({}),
    };
    let record = store.store_memory(&input).unwrap();
    assert_eq!(record.tags, vec!["Storage".to_owned()]);
}

#[test]
fn malformed_file_does_not_block_other_records() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "good", "valid fact", "health");
    let broken = store
        .markdown
        .absolute(std::path::Path::new("memories/health/broken.md"));
    std::fs::write(&broken, "---\nid: broken\nscope: nope\n---\nbody\n").unwrap();

    let records = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "good");

    let report = store
        .memory_health(
            "/repo",
            &MemoryListFilter::default(),
            &Default::default(),
            &|_| true,
        )
        .unwrap();
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.kind == "malformed_frontmatter")
    );
}

#[test]
fn dangling_link_is_tolerated_and_reported() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "dangling", "links to a deleted memory", "links");
    // Bypass validation to simulate a hand-edited file whose target vanished.
    let mut record = super::markdown_memory::load_by_id(&store.markdown, "/repo", "dangling")
        .unwrap()
        .unwrap();
    record.links.push(MemoryLink {
        id: "gone".to_owned(),
        relation: "refines".to_owned(),
    });
    super::markdown_memory::write_existing_record(&store.markdown, &store.conn, &record).unwrap();

    let loaded = store.get_memory("/repo", "dangling").unwrap().unwrap();
    assert_eq!(loaded.links.len(), 1);
    let report = store
        .memory_health(
            "/repo",
            &MemoryListFilter::default(),
            &Default::default(),
            &|_| true,
        )
        .unwrap();
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.kind == "dangling_link")
    );
}

#[test]
fn supersedes_cycle_is_deterministic() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "cycle-a", "a", "cycle");
    seed(&store, "cycle-b", "b", "cycle");
    let mut a = super::markdown_memory::load_by_id(&store.markdown, "/repo", "cycle-a")
        .unwrap()
        .unwrap();
    a.links.push(MemoryLink {
        id: "cycle-b".to_owned(),
        relation: "supersedes".to_owned(),
    });
    super::markdown_memory::write_existing_record(&store.markdown, &store.conn, &a).unwrap();
    let mut b = super::markdown_memory::load_by_id(&store.markdown, "/repo", "cycle-b")
        .unwrap()
        .unwrap();
    b.links.push(MemoryLink {
        id: "cycle-a".to_owned(),
        relation: "supersedes".to_owned(),
    });
    super::markdown_memory::write_existing_record(&store.markdown, &store.conn, &b).unwrap();

    let visible = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert!(visible.is_empty(), "every cycle member stays superseded");
    let all = store
        .list_memories(
            "/repo",
            &MemoryListFilter {
                include_superseded: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|record| record.superseded_by.is_some()));
}

#[test]
fn single_member_group_is_a_noop() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "solo", "only fact", "solo");
    let plan = store
        .consolidate_memories("/repo", &MemoryListFilter::default(), false)
        .unwrap();
    assert!(plan.groups.is_empty());
    assert!(plan.consolidated_id_is_none());
}

#[test]
fn session_scope_requires_session_id_before_write() {
    let (_dir, mut store) = open_store(16, 1024);
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Session,
        topic: "validation".to_owned(),
        title: "invalid".to_owned(),
        body: "body".to_owned(),
        importance: MemoryImportance::Normal,
        source_id: None,
        tags: Vec::new(),
        links: Vec::new(),
        metadata: serde_json::json!({}),
    };
    assert!(store.store_memory(&input).is_err());
}

trait ConsolidationPlanExt {
    fn consolidated_id_is_none(&self) -> bool;
}

impl ConsolidationPlanExt for MemoryConsolidationPlan {
    fn consolidated_id_is_none(&self) -> bool {
        self.groups
            .iter()
            .all(|group| group.consolidated_id.is_none())
    }
}

#[test]
fn migrate_markdown_dry_run_export_and_finalize() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .conn
        .execute(
            "INSERT INTO memory_index
                (id, repo_root, scope, topic, title, body, importance,
                 created_at, updated_at, metadata_json, op_history_json)
             VALUES ('legacy-md', '/repo', 'project', 'legacy', 'legacy title',
                     'legacy body', 'normal',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '{}', '[]')",
            [],
        )
        .unwrap();

    let dry = store.migrate_markdown(true, false).unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.written.len(), 1);
    assert!(dry.backup.is_none());
    assert!(
        !store
            .markdown
            .absolute(std::path::Path::new("memories/legacy/legacy-md.md"))
            .exists()
    );

    let applied = store.migrate_markdown(false, false).unwrap();
    assert_eq!(applied.written.len(), 1);
    assert!(applied.backup.is_some());
    assert!(
        store
            .markdown
            .absolute(std::path::Path::new("memories/legacy/legacy-md.md"))
            .exists()
    );

    let rerun = store.migrate_markdown(false, false).unwrap();
    assert!(rerun.written.is_empty());

    store.migrate_markdown(false, true).unwrap();
    assert!(
        !std::path::Path::new(&applied.backup.unwrap()).exists(),
        "finalize must remove the backup"
    );
}

#[test]
fn hand_added_markdown_memory_is_indexed_for_list_and_recall() {
    let (_dir, store) = open_store(16, 1024);
    let record = MemoryRecord {
        id: "hand-added".to_owned(),
        repo_root: "/repo".to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: "handmade".to_owned(),
        title: "Hand added".to_owned(),
        body: "handcrafted markdown fact".to_owned(),
        importance: MemoryImportance::Normal,
        created_at: "2026-02-01T00:00:00Z".to_owned(),
        updated_at: "2026-02-01T00:00:00Z".to_owned(),
        last_accessed_at: "2026-02-01T00:00:00Z".to_owned(),
        decay_score: 0.0,
        source_id: None,
        metadata: serde_json::json!({}),
        tags: vec!["added".to_owned()],
        links: Vec::new(),
        superseded_by: None,
    };
    let encoded = super::markdown_memory::encode_record(&record).unwrap();
    store
        .markdown
        .write_record(
            std::path::Path::new(&super::markdown_memory::relative_path(
                "handmade",
                "hand-added",
            )),
            &encoded,
        )
        .unwrap();

    let listed = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "hand-added");

    let hits = store
        .recall_memories(
            "/repo",
            "handcrafted",
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.id, "hand-added");
}

#[test]
fn hand_deleted_markdown_memory_is_pruned_from_index_and_recall() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "kept", "kept fact", "prune");
    seed(&store, "gone", "gonefact unique token", "prune");

    let relative = super::markdown_memory::relative_path("prune", "gone");
    std::fs::remove_file(store.markdown.absolute(&relative)).unwrap();

    let listed = store
        .list_memories("/repo", &MemoryListFilter::default())
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "kept");

    let hits = store
        .recall_memories(
            "/repo",
            "gonefact",
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert!(hits.is_empty(), "deleted markdown must leave recall");
}

#[test]
fn hand_edited_markdown_body_is_reindexed_for_recall() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "edit-me", "alpha original text", "edit");
    let relative = super::markdown_memory::relative_path("edit", "edit-me");
    let mut record = store.markdown.read_record(&relative).unwrap();
    record.body = "zebra replacement text".to_owned();
    store.markdown.write_record(&relative, &record).unwrap();

    let hits = store
        .recall_memories(
            "/repo",
            "zebra",
            &MemoryListFilter::default(),
            false,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.body, "zebra replacement text");
}

#[test]
fn malformed_unrelated_file_does_not_block_get_by_id() {
    let (_dir, store) = open_store(16, 1024);
    seed(&store, "good-get", "valid fact", "health");
    let broken = store
        .markdown
        .absolute(std::path::Path::new("memories/health/broken.md"));
    std::fs::write(&broken, "---\nid: broken\nscope: nope\n---\nbody\n").unwrap();

    let record = store.get_memory("/repo", "good-get").unwrap();
    assert_eq!(record.map(|record| record.id), Some("good-get".to_owned()));
}
