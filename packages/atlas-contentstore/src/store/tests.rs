use tempfile::NamedTempFile;

use super::util::{is_corruption_error, levenshtein, rrf_merge};
use super::*;

fn open_store() -> ContentStore {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    std::mem::forget(file);
    let mut store = ContentStore::open(&path).unwrap();
    store.migrate().unwrap();
    store
}

fn meta(id: &str) -> SourceMeta {
    SourceMeta {
        id: id.to_string(),
        session_id: Some("sess1".into()),
        source_type: "review_context".into(),
        label: "test artifact".into(),
        repo_root: Some("/repo".into()),
        identity_kind: "artifact_label".into(),
        identity_value: "test artifact".into(),
    }
}

#[test]
fn index_and_retrieve_by_source_id() {
    let mut store = open_store();
    store
        .index_artifact(meta("src-1"), "hello world", "text/plain")
        .unwrap();
    let src = store.get_source("src-1").unwrap().unwrap();
    assert_eq!(src.identity_kind, "artifact_label");
    assert_eq!(src.identity_value, "test artifact");
    let chunks = store.get_chunks("src-1").unwrap();
    assert!(!chunks.is_empty());
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

#[test]
fn rrf_merge_deduplicates() {
    let make = |source_id: &str, idx: usize, content: &str| ChunkResult {
        source_id: source_id.to_string(),
        chunk_id: String::new(),
        chunk_index: idx,
        title: None,
        content: content.to_string(),
        content_type: "text/plain".to_string(),
    };
    let a = vec![make("s1", 0, "alpha"), make("s2", 0, "beta")];
    let b = vec![make("s1", 0, "alpha"), make("s3", 0, "gamma")];
    let merged = rrf_merge(&a, &b);
    assert_eq!(merged.len(), 3, "RRF merge should deduplicate");
    assert_eq!(merged[0].source_id, "s1");
}

#[test]
fn levenshtein_distances() {
    assert_eq!(levenshtein("kitten", "sitting"), 3);
    assert_eq!(levenshtein("fox", "fog"), 1);
    assert_eq!(levenshtein("identical", "identical"), 0);
    assert_eq!(levenshtein("", "abc"), 3);
}

#[test]
fn vocabulary_correction_suggests_close_term() {
    let mut store = open_store();
    store
        .index_artifact(
            meta("src-corr"),
            "the algorithm performs computation efficiently",
            "text/plain",
        )
        .unwrap();
    let correction = store.suggest_correction("algoritm").unwrap();
    assert_eq!(correction, Some("algorithm".to_string()));
}

#[test]
fn configurable_thresholds_respected() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    std::mem::forget(file);
    let mut store = ContentStore::open_with_config(
        &path,
        ContentStoreConfig {
            small_output_bytes: 10,
            preview_threshold_bytes: 50,
            fallback_min_results: 1,
            max_db_bytes: None,
        },
    )
    .unwrap();
    store.migrate().unwrap();

    let raw = store
        .route_output(meta("t1"), "hello", "text/plain")
        .unwrap();
    assert!(matches!(raw, OutputRouting::Raw(_)));

    let preview = store
        .route_output(
            meta("t2"),
            "this is a medium length output text!",
            "text/plain",
        )
        .unwrap();
    assert!(matches!(preview, OutputRouting::Preview { .. }));

    let big = "x".repeat(100);
    let pointer = store.route_output(meta("t3"), &big, "text/plain").unwrap();
    assert!(matches!(pointer, OutputRouting::Pointer { .. }));
}

#[test]
fn routing_stats_increment_correctly() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    std::mem::forget(file);
    let mut store = ContentStore::open_with_config(
        &path,
        ContentStoreConfig {
            small_output_bytes: 10,
            preview_threshold_bytes: 100,
            fallback_min_results: 3,
            max_db_bytes: None,
        },
    )
    .unwrap();
    store.migrate().unwrap();

    store
        .route_output(meta("rs-a"), "hi", "text/plain")
        .unwrap();
    store
        .route_output(meta("rs-b"), "a".repeat(50).as_str(), "text/plain")
        .unwrap();
    store
        .route_output(meta("rs-c"), "b".repeat(200).as_str(), "text/plain")
        .unwrap();

    let stats = store.routing_stats();
    assert_eq!(stats.raw_count, 1);
    assert_eq!(stats.preview_count, 1);
    assert_eq!(stats.pointer_count, 1);
    assert_eq!(stats.avoided_bytes, 50 + 200);
}

#[test]
fn size_limit_enforced_by_pruning_oldest_sources() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    std::mem::forget(file);
    let mut store = ContentStore::open_with_config(
        &path,
        ContentStoreConfig {
            small_output_bytes: 0,
            preview_threshold_bytes: 1,
            fallback_min_results: 3,
            max_db_bytes: Some(1),
        },
    )
    .unwrap();
    store.migrate().unwrap();

    for i in 0..5 {
        store
            .route_output(
                meta(&format!("sl-{i}")),
                &"content ".repeat(200),
                "text/plain",
            )
            .unwrap();
    }

    let (src_count, _) = store.stats(None).unwrap();
    assert!(
        src_count < 5,
        "size limit should have pruned old sources; got {src_count}"
    );
}

#[test]
fn routing_stats_default_is_zero() {
    let store = open_store();
    let stats = store.routing_stats();
    assert_eq!(stats, RoutingStats::default());
}

#[test]
fn corrupt_content_db_is_quarantined_on_open() {
    use std::path::Path;

    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    drop(file);
    std::fs::write(&path, b"not a sqlite database").unwrap();

    let result = ContentStore::open(&path);
    assert!(result.is_err(), "corrupt DB must return error");

    let quarantine = format!("{path}.quarantine");
    assert!(
        Path::new(&quarantine).exists(),
        "quarantine file must be created: {quarantine}"
    );
}

#[test]
fn quarantine_allows_fresh_content_db_open() {
    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_str().unwrap().to_string();
    drop(file);
    std::fs::write(&path, b"garbage").unwrap();

    let _ = ContentStore::open(&path);

    let mut store = ContentStore::open(&path).expect("fresh open must succeed after quarantine");
    store.migrate().unwrap();
}

#[test]
fn is_corruption_error_detects_known_messages() {
    let cases = [
        "database disk image is malformed",
        "file is not a database",
        "not a database",
    ];
    for msg in cases {
        let err = atlas_core::AtlasError::Db(msg.to_string());
        assert!(is_corruption_error(&err), "must match: {msg}");
    }
}

#[test]
fn begin_indexing_sets_state_to_indexing() {
    let mut store = open_store();
    store.begin_indexing("/repo/a", 42).unwrap();
    let status = store.get_index_status("/repo/a").unwrap().unwrap();
    assert_eq!(status.state, IndexState::Indexing);
    assert_eq!(status.files_discovered, 42);
    assert_eq!(status.files_indexed, 0);
    assert_eq!(status.chunks_written, 0);
    assert!(status.last_indexed_at.is_none());
    assert!(status.last_error.is_none());
}

#[test]
fn finish_indexing_marks_indexed_and_stamps_time() {
    let mut store = open_store();
    store.begin_indexing("/repo/b", 10).unwrap();
    store
        .finish_indexing(
            "/repo/b",
            &IndexingStats {
                files_indexed: 9,
                chunks_written: 30,
                chunks_reused: 1,
            },
        )
        .unwrap();
    let status = store.get_index_status("/repo/b").unwrap().unwrap();
    assert_eq!(status.state, IndexState::Indexed);
    assert_eq!(status.files_indexed, 9);
    assert_eq!(status.chunks_written, 30);
    assert_eq!(status.chunks_reused, 1);
    assert!(status.last_indexed_at.is_some());
    assert!(status.last_error.is_none());
}

#[test]
fn fail_indexing_sets_error_state() {
    let mut store = open_store();
    store.begin_indexing("/repo/c", 5).unwrap();
    store
        .fail_indexing("/repo/c", "parse error on main.rs")
        .unwrap();
    let status = store.get_index_status("/repo/c").unwrap().unwrap();
    assert_eq!(status.state, IndexState::IndexFailed);
    assert_eq!(status.last_error.unwrap(), "parse error on main.rs");
}

#[test]
fn missing_repo_returns_none() {
    let store = open_store();
    let status = store.get_index_status("/nonexistent/repo").unwrap();
    assert!(status.is_none());
}

#[test]
fn list_index_statuses_returns_all_repos() {
    let mut store = open_store();
    store.begin_indexing("/repo/x", 1).unwrap();
    store
        .finish_indexing("/repo/x", &IndexingStats::default())
        .unwrap();
    store.begin_indexing("/repo/y", 2).unwrap();
    let statuses = store.list_index_statuses().unwrap();
    assert_eq!(statuses.len(), 2);
    let roots: Vec<&str> = statuses
        .iter()
        .map(|status| status.repo_root.as_str())
        .collect();
    assert!(roots.contains(&"/repo/x"));
    assert!(roots.contains(&"/repo/y"));
}

#[test]
fn begin_indexing_resets_counters_on_restart() {
    let mut store = open_store();
    store.begin_indexing("/repo/d", 20).unwrap();
    store
        .finish_indexing(
            "/repo/d",
            &IndexingStats {
                files_indexed: 18,
                chunks_written: 60,
                chunks_reused: 2,
            },
        )
        .unwrap();
    store.begin_indexing("/repo/d", 25).unwrap();
    let status = store.get_index_status("/repo/d").unwrap().unwrap();
    assert_eq!(status.state, IndexState::Indexing);
    assert_eq!(status.files_discovered, 25);
    assert_eq!(status.files_indexed, 0);
    assert_eq!(status.chunks_written, 0);
}

#[test]
fn index_artifact_auto_increments_index_counters() {
    let mut store = open_store();
    store
        .index_artifact(meta("src-ai"), "auto increment test content", "text/plain")
        .unwrap();
    let status = store.get_index_status("/repo").unwrap().unwrap();
    assert_eq!(status.state, IndexState::Indexed);
    assert!(status.files_indexed >= 1);
    assert!(status.chunks_written >= 1);
}

#[test]
fn interrupted_indexing_visible_as_indexing_state() {
    let mut store = open_store();
    store.begin_indexing("/repo/interrupted", 100).unwrap();
    let status = store
        .get_index_status("/repo/interrupted")
        .unwrap()
        .unwrap();
    assert_eq!(
        status.state,
        IndexState::Indexing,
        "interrupted run must show as 'indexing' to signal recovery needed"
    );
}
