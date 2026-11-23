use super::*;

#[test]
fn rrf_merge_deduplicates() {
    let make = |source_id: &str, chunk_id: &str, idx: usize, content: &str| ChunkResult {
        source_id: source_id.to_string(),
        chunk_id: chunk_id.to_string(),
        chunk_index: idx,
        title: None,
        content: content.to_string(),
        content_type: "text/plain".to_string(),
    };
    let a = vec![
        make("s1", "chunk-alpha", 0, "alpha"),
        make("s2", "chunk-beta", 0, "beta"),
    ];
    let b = vec![
        make("s1", "chunk-alpha", 9, "alpha moved"),
        make("s3", "chunk-gamma", 0, "gamma"),
    ];
    let merged = rrf_merge(&a, &b);
    assert_eq!(merged.len(), 3, "RRF merge should deduplicate");
    assert_eq!(merged[0].chunk_id, "chunk-alpha");
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
            ..ContentStoreConfig::default()
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
fn search_can_filter_by_agent_partition() {
    let mut store = open_store();
    let mut alpha = meta("src-agent-a");
    alpha.agent_id = Some("agent-alpha".into());
    store
        .index_artifact(alpha, "shared phrase alpha unique", "text/plain")
        .unwrap();

    let mut beta = meta("src-agent-b");
    beta.agent_id = Some("agent-beta".into());
    store
        .index_artifact(beta, "shared phrase beta unique", "text/plain")
        .unwrap();

    let alpha_results = store
        .search_with_fallback(
            "shared phrase",
            &SearchFilters {
                agent_id: Some("agent-alpha".into()),
                ..SearchFilters::default()
            },
        )
        .unwrap();
    assert_eq!(alpha_results.len(), 1);
    assert_eq!(alpha_results[0].source_id, "src-agent-a");
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
            ..ContentStoreConfig::default()
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
            ..ContentStoreConfig::default()
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

    let (src_count, _) = store.stats(None, None).unwrap();
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
