use super::*;

fn seed_op_memory(store: &SessionStore) -> MemoryRecord {
    seed_memory(
        store,
        "op-memory",
        "2026-01-01T00:00:00Z",
        "operator target body",
        "operators",
        MemoryImportance::Normal,
        MemoryScope::Project,
    )
}

fn failure_event(id: i64, payload: serde_json::Value) -> SessionEventRow {
    SessionEventRow {
        id,
        session_id: SessionId::derive("/repo", "main", "cli"),
        event_type: SessionEventType::CommandFail,
        priority: 3,
        payload_json: payload.to_string(),
        event_hash: format!("hash-{id}"),
        created_at: format!("2026-01-01T00:00:0{id}Z"),
    }
}

fn metadata_json(store: &SessionStore, topic: &str) -> serde_json::Value {
    let raw: String = store
        .conn
        .query_row(
            "SELECT metadata_json FROM memories WHERE topic = ?1 AND title = 'Failure log'",
            [topic],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn apply_memory_op_persists_state_and_history() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_op_memory(&store);

    let outcome = store
        .apply_memory_op(
            "/repo",
            "op-memory",
            &MemoryOp::Count {
                field: "fail_count".to_owned(),
            },
        )
        .unwrap();
    assert_eq!(outcome.memory_id, "op-memory");
    assert_eq!(outcome.op, "count");
    assert_eq!(
        outcome.changed_fields,
        vec!["metadata.fail_count", "metadata.last_seen"]
    );

    let history = store
        .memory_op_history("/repo", "op-memory")
        .unwrap()
        .expect("history exists");
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.entries[0].op, "store");
    assert_eq!(history.entries[1].op, "count");

    let metadata: String = store
        .conn
        .query_row(
            "SELECT metadata_json FROM memories WHERE id = 'op-memory'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata).unwrap();
    assert_eq!(metadata["fail_count"], serde_json::json!(1));
    assert_eq!(metadata["seed"], serde_json::json!("op-memory"));
}

#[test]
fn apply_memory_op_rejects_missing_and_superseded_rows() {
    let (_dir, mut store) = open_store(16, 1024);
    let op = MemoryOp::Count {
        field: "runs".to_owned(),
    };
    assert!(store.apply_memory_op("/repo", "missing", &op).is_err());

    seed_op_memory(&store);
    store
        .conn
        .execute(
            "UPDATE memories SET superseded_by = 'newer' WHERE id = 'op-memory'",
            [],
        )
        .unwrap();
    let error = store
        .apply_memory_op("/repo", "op-memory", &op)
        .unwrap_err()
        .to_string();
    assert!(error.contains("superseded"), "unexpected error: {error}");
}

#[test]
fn invalid_operator_leaves_row_unchanged() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_op_memory(&store);

    let error = store
        .apply_memory_op(
            "/repo",
            "op-memory",
            &MemoryOp::Set {
                field: "body".to_owned(),
                value: serde_json::json!("   "),
            },
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("non-empty"), "unexpected error: {error}");

    let history = store
        .memory_op_history("/repo", "op-memory")
        .unwrap()
        .expect("history exists");
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].op, "store");
    let body: String = store
        .conn
        .query_row(
            "SELECT body FROM memories WHERE id = 'op-memory'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(body, "operator target body");
}

#[test]
fn failure_events_aggregate_into_topic_entities() {
    let (_dir, store) = open_store(16, 1024);
    let events = vec![
        failure_event(
            1,
            serde_json::json!({"command": "cargo test --quiet", "stderr": "error: expected one of `,`"}),
        ),
        failure_event(
            2,
            serde_json::json!({"command": "cargo build", "stderr": "error:   missing semicolon"}),
        ),
    ];

    crate::store::memory_ops::record_failure_events(
        &store.conn,
        "/repo",
        &events,
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    let test_metadata = metadata_json(&store, "test-failures");
    assert_eq!(test_metadata["fail_count"], serde_json::json!(1));
    assert_eq!(
        test_metadata["error_signatures"].as_array().unwrap().len(),
        1
    );
    assert_eq!(test_metadata["last_failed_at"], "2026-01-01T00:00:01Z");

    let build_metadata = metadata_json(&store, "build-errors");
    assert_eq!(build_metadata["fail_count"], serde_json::json!(1));
    assert_eq!(build_metadata["last_failed_at"], "2026-01-01T00:00:02Z");
    assert_eq!(build_metadata["entity_key"], "failure_log");

    // A repeated signature increments the counter without growing signatures.
    crate::store::memory_ops::record_failure_events(
        &store.conn,
        "/repo",
        &events[..1],
        "2026-01-01T00:00:03Z",
    )
    .unwrap();
    let test_metadata = metadata_json(&store, "test-failures");
    assert_eq!(test_metadata["fail_count"], serde_json::json!(2));
    assert_eq!(
        test_metadata["error_signatures"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn failure_events_do_not_create_entities_without_failures() {
    let (_dir, store) = open_store(16, 1024);
    let mut event = failure_event(1, serde_json::json!({"command": "cargo build"}));
    event.event_type = SessionEventType::CommandRun;
    crate::store::memory_ops::record_failure_events(
        &store.conn,
        "/repo",
        &[event],
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn schema_issues_detect_missing_fts_and_triggers() {
    let (_dir, store) = open_store(16, 1024);
    assert!(store.memory_schema_issues().is_empty());

    store.conn.execute_batch("DROP TABLE memories_fts").unwrap();
    let issues = store.memory_schema_issues();
    assert!(
        issues
            .iter()
            .any(|issue| issue == "missing table: memories_fts"),
        "issues: {issues:?}"
    );

    store
        .conn
        .execute_batch(
            "DROP TRIGGER memories_ai; DROP TRIGGER memories_ad; DROP TRIGGER memories_au;",
        )
        .unwrap();
    let issues = store.memory_schema_issues();
    for trigger in ["memories_ai", "memories_ad", "memories_au"] {
        assert!(
            issues.contains(&format!("missing trigger: {trigger}")),
            "issues: {issues:?}"
        );
    }
}

#[test]
fn schema_issues_detect_fts_drift_and_malformed_history() {
    let (_dir, store) = open_store(16, 1024);
    seed_op_memory(&store);
    seed_memory(
        &store,
        "second-memory",
        "2026-01-01T00:00:00Z",
        "second body",
        "other",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    store
        .conn
        .execute(
            "UPDATE memories SET op_history_json = 'not-json' WHERE id = 'second-memory'",
            [],
        )
        .unwrap();

    // Removing a row without its delete trigger leaves the FTS index stale.
    store
        .conn
        .execute_batch("DROP TRIGGER memories_ad; DELETE FROM memories WHERE id = 'op-memory';")
        .unwrap();

    let issues = store.memory_schema_issues();
    assert!(
        issues.iter().any(|issue| issue.contains("out of sync")),
        "issues: {issues:?}"
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.contains("malformed op_history_json")),
        "issues: {issues:?}"
    );
}

#[test]
fn migration_013_backfills_fts_for_existing_memories() {
    let (_dir, mut store) = open_store(16, 1024);
    store.migrate_to(12).unwrap();
    // Insert with the pre-013 column set directly: `store_memory_at` writes the
    // post-013 `op_history_json` column and must not be used on an older schema.
    store
        .conn
        .execute(
            "INSERT INTO memories
                (id, repo_root, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, metadata_json)
             VALUES ('pre-013', '/repo', 'project', 'migration', 'title-migration',
                     'backfilled connection pool memory', 'normal',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', '{}')",
            [],
        )
        .unwrap();
    store.migrate_to(13).unwrap();

    let fts_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memories_fts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fts_count, 1);

    let matches: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'connection'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(matches, 1);
}
