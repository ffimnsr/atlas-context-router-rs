use super::*;

// ── ICM-A1 — shared memory model and storage schema ──────────────────────────

#[test]
fn memories_table_schema_matches_golden() {
    let (_dir, store) = open_store(16, 1024);

    let columns = table_columns(&store.conn, "memories");
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
            "last_accessed_at",
            "decay_score",
            "source_id",
            "metadata_json",
        ]
    );

    let sql: String = store
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sql.contains("CHECK (scope IN ('project', 'session', 'frontend', 'global'))"));
    assert!(sql.contains("CHECK (importance IN ('critical', 'high', 'normal', 'low'))"));
    assert!(sql.contains("scope <> 'frontend' OR (frontend IS NOT NULL AND frontend <> '')"));
    assert!(sql.contains("scope <> 'session' OR (session_id IS NOT NULL AND session_id <> '')"));
    assert!(sql.contains("decay_score      REAL NOT NULL DEFAULT 0"));
}

#[test]
fn memories_indexes_match_golden() {
    let (_dir, store) = open_store(16, 1024);

    for (name, expected) in [
        (
            "idx_memories_repo_topic",
            "CREATE INDEX idx_memories_repo_topic\n    ON memories(repo_root, topic)",
        ),
        (
            "idx_memories_repo_importance",
            "CREATE INDEX idx_memories_repo_importance\n    ON memories(repo_root, importance)",
        ),
        (
            "idx_memories_repo_scope",
            "CREATE INDEX idx_memories_repo_scope\n    ON memories(repo_root, scope)",
        ),
        (
            "idx_memories_repo_session",
            "CREATE INDEX idx_memories_repo_session\n    ON memories(repo_root, session_id)",
        ),
        (
            "idx_memories_repo_accessed",
            "CREATE INDEX idx_memories_repo_accessed\n    ON memories(repo_root, last_accessed_at DESC)",
        ),
    ] {
        let sql: String = store
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params![name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sql, expected, "index {name} must match golden SQL");
    }
}

#[test]
fn memory_schema_issues_are_empty_when_healthy() {
    let (_dir, store) = open_store(16, 1024);
    assert!(store.memory_schema_issues().is_empty());
}

#[test]
fn memory_schema_issues_detect_missing_table_and_indexes() {
    let (_dir, store) = open_store(16, 1024);
    store.conn.execute_batch("DROP TABLE memories").unwrap();
    let issues = store.memory_schema_issues();
    assert_eq!(issues, vec!["missing table: memories"]);

    // Re-create the table without indexes: every index must be reported.
    store
        .conn
        .execute_batch(
            "CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                repo_root TEXT NOT NULL,
                session_id TEXT,
                frontend TEXT,
                scope TEXT NOT NULL DEFAULT 'project',
                topic TEXT NOT NULL DEFAULT '',
                title TEXT NOT NULL DEFAULT '',
                body TEXT NOT NULL,
                importance TEXT NOT NULL DEFAULT 'normal',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_accessed_at TEXT NOT NULL,
                decay_score REAL NOT NULL DEFAULT 0,
                source_id TEXT,
                metadata_json TEXT NOT NULL DEFAULT '{}'
            )",
        )
        .unwrap();
    let issues = store.memory_schema_issues();
    assert_eq!(
        issues,
        vec![
            "missing index: idx_memories_repo_topic",
            "missing index: idx_memories_repo_importance",
            "missing index: idx_memories_repo_scope",
            "missing index: idx_memories_repo_session",
            "missing index: idx_memories_repo_accessed",
        ]
    );
}

#[test]
fn memory_importance_and_scope_parse_rejects_unknown_values() {
    for (value, parsed) in [
        ("critical", MemoryImportance::Critical),
        ("high", MemoryImportance::High),
        ("normal", MemoryImportance::Normal),
        ("low", MemoryImportance::Low),
    ] {
        assert_eq!(value.parse::<MemoryImportance>().unwrap(), parsed);
    }
    for value in ["urgent", "CRITICAL", "", "normal "] {
        assert!(
            value.parse::<MemoryImportance>().is_err(),
            "{value:?} must be rejected"
        );
    }

    for (value, parsed) in [
        ("project", MemoryScope::Project),
        ("session", MemoryScope::Session),
        ("frontend", MemoryScope::Frontend),
        ("global", MemoryScope::Global),
    ] {
        assert_eq!(value.parse::<MemoryScope>().unwrap(), parsed);
    }
    for value in ["org", "workspace", "PROJECT", ""] {
        assert!(
            value.parse::<MemoryScope>().is_err(),
            "{value:?} must be rejected"
        );
    }
}

#[test]
fn memory_json_deserialization_rejects_unknown_importance_and_scope() {
    let unknown_importance = serde_json::from_str::<NewMemory>(
        r#"{"repo_root":"/repo","body":"x","importance":"urgent"}"#,
    );
    assert!(unknown_importance.is_err(), "unknown importance rejected");

    let unknown_scope =
        serde_json::from_str::<NewMemory>(r#"{"repo_root":"/repo","body":"x","scope":"org"}"#);
    assert!(unknown_scope.is_err(), "unknown scope rejected");
}

#[test]
fn new_memory_defaults_to_normal_importance_and_project_scope() {
    let input =
        serde_json::from_str::<NewMemory>(r#"{"repo_root":"/repo","body":"remember this"}"#)
            .unwrap();
    assert_eq!(input.importance, MemoryImportance::Normal);
    assert_eq!(input.scope, MemoryScope::Project);
    assert_eq!(input.metadata, serde_json::json!({}));
    assert!(input.validate().is_ok());
}

#[test]
fn new_memory_validation_requires_frontend_for_frontend_scope_and_session_for_session_scope() {
    let base = serde_json::from_str::<NewMemory>(r#"{"repo_root":"/repo","body":"remember this"}"#)
        .unwrap();

    let frontend_scoped = NewMemory {
        scope: MemoryScope::Frontend,
        ..base.clone()
    };
    let error = frontend_scoped.validate().unwrap_err().to_string();
    assert!(
        error.contains("frontend identifier"),
        "error must name the missing frontend: {error}"
    );

    let frontend_scoped_ok = NewMemory {
        scope: MemoryScope::Frontend,
        frontend: Some("codex".to_owned()),
        ..base.clone()
    };
    assert!(frontend_scoped_ok.validate().is_ok());

    let session_scoped = NewMemory {
        scope: MemoryScope::Session,
        ..base.clone()
    };
    let error = session_scoped.validate().unwrap_err().to_string();
    assert!(
        error.contains("session_id"),
        "error must name the missing session_id: {error}"
    );

    let session_scoped_ok = NewMemory {
        scope: MemoryScope::Session,
        session_id: Some("s1".to_owned()),
        ..base
    };
    assert!(session_scoped_ok.validate().is_ok());
}

#[test]
fn memory_storage_boundary_rejects_unknown_importance_and_scope() {
    let (_dir, store) = open_store(16, 1024);

    let insert = |importance: &str, scope: &str| -> rusqlite::Result<()> {
        store
            .conn
            .execute(
                "INSERT INTO memories
                (id, repo_root, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, metadata_json)
             VALUES ('m1', '/repo', ?1, 'hooks', 'T', 'body', ?2,
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 0, '{}')",
                params![scope, importance],
            )
            .map(|_| ())
    };

    let unknown_importance = insert("urgent", "project").unwrap_err().to_string();
    assert!(
        unknown_importance.contains("CHECK constraint failed"),
        "unknown importance must fail at storage: {unknown_importance}"
    );
    let unknown_scope = insert("normal", "org").unwrap_err().to_string();
    assert!(
        unknown_scope.contains("CHECK constraint failed"),
        "unknown scope must fail at storage: {unknown_scope}"
    );

    // Session-scoped memory without session id and frontend-scoped memory
    // without frontend identifier are rejected by the storage boundary too.
    let session_without_id = store
        .conn
        .execute(
            "INSERT INTO memories
                (id, repo_root, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, metadata_json)
             VALUES ('m2', '/repo', 'session', 'hooks', 'T', 'body', 'normal',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 0, '{}')",
            [],
        )
        .unwrap_err()
        .to_string();
    assert!(
        session_without_id.contains("CHECK constraint failed"),
        "session scope without session_id must fail: {session_without_id}"
    );
    let frontend_without_name = store
        .conn
        .execute(
            "INSERT INTO memories
                (id, repo_root, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, metadata_json)
             VALUES ('m3', '/repo', 'frontend', 'hooks', 'T', 'body', 'normal',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z',
                     '2026-01-01T00:00:00Z', 0, '{}')",
            [],
        )
        .unwrap_err()
        .to_string();
    assert!(
        frontend_without_name.contains("CHECK constraint failed"),
        "frontend scope without frontend must fail: {frontend_without_name}"
    );

    // Valid rows with every scope and importance value are accepted.
    store
        .conn
        .execute_batch(
            r#"INSERT INTO memories
                (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json)
             VALUES
                ('m4', '/repo', NULL, NULL, 'project', 'hooks', 'T', 'b', 'critical',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0.5, 'src-1', '{"k":1}'),
                ('m5', '/repo', NULL, NULL, 'global', 'hooks', 'T', 'b', 'high',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, NULL, '{}'),
                ('m6', '/repo', 's1', NULL, 'session', 'hooks', 'T', 'b', 'normal',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, NULL, '{}'),
                ('m7', '/repo', NULL, 'codex', 'frontend', 'hooks', 'T', 'b', 'low',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 0, NULL, '{}')"#,
        )
        .unwrap();
}

#[test]
fn memory_row_round_trips_through_record_shape() {
    let (_dir, store) = open_store(16, 1024);
    store
        .conn
        .execute_batch(
            r#"INSERT INTO memories
                (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                 created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json)
             VALUES ('m1', '/repo', 's1', 'codex', 'frontend', 'hooks', 'Hook notes', 'body text',
                     'critical', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z',
                     '2026-01-03T00:00:00Z', 0.25, 'src-9', '{"source_kind":"hook"}')"#,
        )
        .unwrap();

    let record = store
        .conn
        .query_row(
            "SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                    created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json
             FROM memories WHERE id = 'm1'",
            [],
            super::memory::row_to_memory,
        )
        .unwrap();
    assert_eq!(
        record,
        MemoryRecord {
            id: "m1".to_owned(),
            repo_root: "/repo".to_owned(),
            session_id: Some("s1".to_owned()),
            frontend: Some("codex".to_owned()),
            scope: MemoryScope::Frontend,
            topic: "hooks".to_owned(),
            title: "Hook notes".to_owned(),
            body: "body text".to_owned(),
            importance: MemoryImportance::Critical,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-02T00:00:00Z".to_owned(),
            last_accessed_at: "2026-01-03T00:00:00Z".to_owned(),
            decay_score: 0.25,
            source_id: Some("src-9".to_owned()),
            metadata: serde_json::json!({ "source_kind": "hook" }),
        }
    );
}
