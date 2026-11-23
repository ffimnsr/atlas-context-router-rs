use super::*;

#[test]
fn index_and_retrieve_by_source_id() {
    let mut store = open_store();
    store
        .index_artifact(meta("src-1"), "hello world", "text/plain")
        .unwrap();
    let src = store.get_source("src-1").unwrap().unwrap();
    assert_eq!(src.identity_kind, "artifact_label");
    assert_eq!(src.identity_value, "test artifact");
    assert_eq!(src.agent_id.as_deref(), Some("agent-alpha"));
    assert!(src.repo_id.is_some());
    assert_eq!(src.repo_ids.len(), 1);
    let chunks = store.get_chunks("src-1").unwrap();
    assert!(!chunks.is_empty());
}

#[test]
fn open_stamps_migration_history_and_provenance() {
    let store = open_store();
    let version: i32 = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, crate::migrations::LATEST_VERSION);

    let history_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(history_count, i64::from(crate::migrations::LATEST_VERSION));

    let (db_kind, created_by): (String, String) = store
        .conn
        .query_row(
            "SELECT db_kind, created_by FROM atlas_provenance WHERE singleton_key = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(db_kind, "context");
    assert_eq!(created_by, format!("atlas v{}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn rollback_and_reupgrade_restore_content_schema() {
    let mut store = open_store();
    store.migrate_to(3).unwrap();
    let version: i32 = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);
    assert!(
        !table_columns(&store.conn, "chunks")
            .iter()
            .any(|column| column == "chunk_id")
    );
    assert!(
        !table_columns(&store.conn, "sources")
            .iter()
            .any(|column| column == "identity_kind")
    );

    store.migrate().unwrap();
    let restored_version: i32 = store
        .conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(restored_version, crate::migrations::LATEST_VERSION);
    assert!(
        table_columns(&store.conn, "chunks")
            .iter()
            .any(|column| column == "chunk_id")
    );
    assert!(
        table_columns(&store.conn, "sources")
            .iter()
            .any(|column| column == "identity_kind")
    );
    assert!(
        table_columns(&store.conn, "sources")
            .iter()
            .any(|column| column == "agent_id")
    );
    assert!(
        table_columns(&store.conn, "sources")
            .iter()
            .any(|column| column == "repo_roots_json")
    );
    assert!(
        table_columns(&store.conn, "sources")
            .iter()
            .any(|column| column == "repo_id")
    );
    assert!(
        table_columns(&store.conn, "sources")
            .iter()
            .any(|column| column == "repo_ids_json")
    );
    assert!(
        table_columns(&store.conn, "sources")
            .iter()
            .any(|column| column == "relative_path")
    );
    assert!(
        table_columns(&store.conn, "sources")
            .iter()
            .any(|column| column == "parent_dir")
    );
}

#[test]
fn delete_source_removes_chunks() {
    let mut store = open_store();
    store
        .index_artifact(meta("src-2"), "some content here", "text/plain")
        .unwrap();
    store.delete_source("src-2").unwrap();
    let src = store.get_source("src-2").unwrap();
    assert!(src.is_none());
    let chunks = store.get_chunks("src-2").unwrap();
    assert!(chunks.is_empty());
}

#[test]
fn routing_small_returns_raw() {
    let mut store = open_store();
    let routing = store
        .route_output(meta("src-3"), "tiny output", "text/plain")
        .unwrap();
    assert!(matches!(routing, OutputRouting::Raw(_)));
}

#[test]
fn routing_large_returns_pointer() {
    let mut store = open_store();
    let big = "word ".repeat(2000);
    let routing = store
        .route_output(meta("src-4"), &big, "text/plain")
        .unwrap();
    assert!(matches!(routing, OutputRouting::Pointer { .. }));
}

#[test]
fn search_returns_indexed_chunk() {
    let mut store = open_store();
    store
        .index_artifact(meta("src-5"), "the quick brown fox", "text/plain")
        .unwrap();
    let results = store.search("quick", &SearchFilters::default()).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].content.contains("quick"));
}

#[test]
fn idempotent_reindex_replaces_chunks() {
    let mut store = open_store();
    store
        .index_artifact(meta("src-6"), "version one content here", "text/plain")
        .unwrap();
    let before = store.get_chunks("src-6").unwrap().len();
    store
        .index_artifact(
            meta("src-6"),
            "version two different content entirely",
            "text/plain",
        )
        .unwrap();
    let after = store.get_chunks("src-6").unwrap();
    assert!(
        after
            .iter()
            .any(|chunk| chunk.content.contains("version two"))
    );
    assert!(after.len() <= before + 5);
}

#[test]
fn trigram_search_finds_substring() {
    let mut store = open_store();
    store
        .index_artifact(
            meta("src-tri"),
            "the mitochondria is the powerhouse of the cell",
            "text/plain",
        )
        .unwrap();
    let results = store
        .search_trigram("mitochondria", &SearchFilters::default())
        .unwrap();
    assert!(!results.is_empty(), "trigram should find 'mitochondria'");
}

#[test]
fn vocabulary_populated_on_index() {
    let mut store = open_store();
    store
        .index_artifact(
            meta("src-vocab"),
            "photosynthesis occurs in chloroplasts",
            "text/plain",
        )
        .unwrap();
    let count: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM vocabulary WHERE term = 'photosynthesis'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "vocabulary should contain indexed terms");
}

#[test]
fn noncanonical_repo_path_sources_are_reported() {
    let store = open_store();
    store
        .conn
        .execute(
            "INSERT INTO sources (
                 id, session_id, source_type, label, repo_root, identity_kind, identity_value, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "bad-source",
                "sess1",
                "review_context",
                "bad source",
                "/repo",
                "repo_path",
                "./src/lib.rs",
                "2025-01-01T00:00:00Z"
            ],
        )
        .unwrap();

    let issues = store.noncanonical_repo_path_sources(100).unwrap();
    assert!(issues.iter().any(|issue| {
        issue.contains("noncanonical_path:")
            && issue.contains("source_id=bad-source")
            && issue.contains("canonical=src/lib.rs")
    }));
}

#[test]
fn search_with_fallback_returns_fts_results() {
    let mut store = open_store();
    store
        .index_artifact(
            meta("src-fb"),
            "the quick brown fox jumped over the lazy dog",
            "text/plain",
        )
        .unwrap();
    let results = store
        .search_with_fallback("fox", &SearchFilters::default())
        .unwrap();
    assert!(
        !results.is_empty(),
        "search_with_fallback should find 'fox'"
    );
}

#[test]
fn search_with_fallback_uses_trigram_when_fts_sparse() {
    let mut store = open_store();
    store
        .index_artifact(
            meta("src-tri2"),
            "polychromatic spectroscopy measurements",
            "text/plain",
        )
        .unwrap();
    let results = store
        .search_with_fallback("spectrosc", &SearchFilters::default())
        .unwrap();
    assert!(
        !results.is_empty(),
        "trigram fallback should find substring 'spectrosc'"
    );
}
