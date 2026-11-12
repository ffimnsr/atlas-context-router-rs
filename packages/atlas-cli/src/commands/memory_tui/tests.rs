//! ICM-H2 smoke tests: the TUI model collects read-only sections without a
//! terminal, without panicking, and without mutating storage.

use super::*;
use atlas_session::{MemoryImportance, MemoryScope, NewMemory, SessionStore};

fn open_store() -> (tempfile::TempDir, SessionStore) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join(".atlas").join("session.db");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("atlas dir");
    let store = SessionStore::open(path.to_str().expect("path")).expect("session store");
    (dir, store)
}

fn seed(store: &mut SessionStore, body: &str, topic: &str, importance: MemoryImportance) {
    store
        .store_memory(&NewMemory {
            repo_root: "/repo".to_owned(),
            session_id: None,
            frontend: None,
            scope: MemoryScope::Project,
            topic: topic.to_owned(),
            title: format!("title-{topic}"),
            body: body.to_owned(),
            importance,
            source_id: None,
            metadata: serde_json::json!({}),
        })
        .expect("store memory");
}

#[test]
fn empty_store_collects_every_section_without_panicking() {
    let (_dir, store) = open_store();
    let model = collect_model("/repo", &store, &TuiFilters::default()).expect("collect model");
    for section in TuiSection::ALL {
        assert!(
            model.rows(section).is_empty(),
            "{} must be empty on an empty store",
            section.title()
        );
    }
    assert_eq!(model.total_rows(), 0);
}

#[test]
fn populated_store_renders_memories_topics_and_health() {
    let (_dir, mut store) = open_store();
    seed(
        &mut store,
        "postgres pool sizing keeps connections bounded",
        "postgres",
        MemoryImportance::Critical,
    );
    seed(
        &mut store,
        "hooks must stay non-blocking",
        "hooks",
        MemoryImportance::Low,
    );

    let filters = TuiFilters {
        limit: 10,
        ..Default::default()
    };
    let model = collect_model("/repo", &store, &filters).expect("collect model");

    let memories = model.rows(TuiSection::Memories);
    assert_eq!(memories.len(), 2);
    assert!(memories[0].secondary.contains("id "));

    let topics = model.rows(TuiSection::Topics);
    assert_eq!(topics.len(), 2);
    assert!(topics.iter().any(|row| row.primary.contains("postgres")));

    let lines = frame_lines(&model, TuiSection::Memories, 0, 10, 100);
    assert!(lines.join("\n").contains("postgres"));

    let json = model_json(&model, &filters);
    assert_eq!(json["mode"], "read_only");
    assert_eq!(json["row_count"].as_u64(), Some(model.total_rows() as u64));
    assert!(json["sections"].as_array().is_some_and(|s| s.len() == 7));
}

#[test]
fn filters_narrow_the_collected_model() {
    let (_dir, mut store) = open_store();
    seed(
        &mut store,
        "postgres pool sizing",
        "postgres",
        MemoryImportance::Critical,
    );
    seed(
        &mut store,
        "hooks stay non-blocking",
        "hooks",
        MemoryImportance::Low,
    );

    let filters = TuiFilters {
        memory: atlas_session::MemoryListFilter {
            topic: Some("hooks".to_owned()),
            ..Default::default()
        },
        limit: 10,
        ..Default::default()
    };
    let model = collect_model("/repo", &store, &filters).expect("collect model");
    assert_eq!(model.rows(TuiSection::Memories).len(), 1);
    assert_eq!(model.rows(TuiSection::Topics).len(), 1);
}

#[test]
fn section_cycling_wraps_in_both_directions() {
    let first = TuiSection::Memories;
    let last = TuiSection::Artifacts;
    assert_eq!(last.next(), first);
    assert_eq!(first.prev(), last);
    for section in TuiSection::ALL {
        assert_eq!(section.next().prev(), section);
    }
}

#[test]
fn preview_is_character_bounded() {
    let text = "a".repeat(50);
    let preview = super::model::preview(&text, 10);
    assert_eq!(preview.chars().count(), 10);
    assert!(preview.ends_with('…'));
    assert_eq!(super::model::preview("short", 10), "short");
}
