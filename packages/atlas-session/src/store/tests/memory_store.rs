use super::*;

#[test]
fn store_memory_persists_record_with_defaults() {
    let (_dir, mut store) = open_store(16, 1024);
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        body: "remember to run the hook tests".to_owned(),
        ..Default::default()
    };
    let record = store.store_memory(&input).unwrap();

    assert_eq!(record.repo_root, "/repo");
    assert_eq!(record.importance, MemoryImportance::Normal);
    assert_eq!(record.scope, MemoryScope::Project);
    assert_eq!(record.decay_score, 0.0);
    assert_eq!(record.session_id, None);
    assert_eq!(record.frontend, None);
    assert_eq!(record.source_id, None);
    assert_eq!(record.metadata, serde_json::json!({}));
    assert_eq!(record.topic, "");
    assert_eq!(record.title, "");
    assert_eq!(record.body, "remember to run the hook tests");
    assert_eq!(record.id.len(), 64, "id must be a stable sha256 hex id");
    assert_eq!(record.created_at, record.updated_at);
    assert_eq!(record.updated_at, record.last_accessed_at);

    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn store_memory_preserves_exact_text_and_metadata() {
    let (_dir, mut store) = open_store(16, 1024);
    let body = "  spaced text with\nnewline and \"quotes\"  ";
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        topic: "hooks".to_owned(),
        title: "Hook notes".to_owned(),
        body: body.to_owned(),
        importance: MemoryImportance::Critical,
        scope: MemoryScope::Frontend,
        frontend: Some("codex".to_owned()),
        source_id: Some("artifact-1".to_owned()),
        metadata: serde_json::json!({ "source_kind": "hook", "count": 3 }),
        ..Default::default()
    };
    let stored = store.store_memory(&input).unwrap();

    let record = store
        .conn
        .query_row(
            "SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                    created_at, updated_at, last_accessed_at, decay_score, source_id,
                    metadata_json
             FROM memories WHERE id = ?1",
            [&stored.id],
            super::memory::row_to_memory,
        )
        .unwrap();
    assert_eq!(record.body, body, "body must be stored exactly as provided");
    assert_eq!(record.importance, MemoryImportance::Critical);
    assert_eq!(record.scope, MemoryScope::Frontend);
    assert_eq!(record.frontend.as_deref(), Some("codex"));
    assert_eq!(record.source_id.as_deref(), Some("artifact-1"));
    assert_eq!(
        record.metadata,
        serde_json::json!({ "source_kind": "hook", "count": 3 })
    );
}

#[test]
fn store_memory_rejects_invalid_inputs() {
    let (_dir, mut store) = open_store(16, 1024);

    let empty = NewMemory {
        repo_root: "/repo".to_owned(),
        body: "   ".to_owned(),
        ..Default::default()
    };
    let error = store.store_memory(&empty).unwrap_err().to_string();
    assert!(error.contains("must not be empty"), "got: {error}");

    let frontend_without_name = NewMemory {
        repo_root: "/repo".to_owned(),
        body: "x".to_owned(),
        scope: MemoryScope::Frontend,
        ..Default::default()
    };
    let error = store
        .store_memory(&frontend_without_name)
        .unwrap_err()
        .to_string();
    assert!(error.contains("frontend identifier"), "got: {error}");

    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "rejected writes must not persist rows");
}
