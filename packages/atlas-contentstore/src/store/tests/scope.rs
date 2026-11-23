use super::*;

#[test]
fn schema_migration_creates_registry_table() {
    let store = open_store();
    // Confirm the table exists by querying its schema.
    let count: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type='table' AND name='embedding_provider_registry'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "migration 007 must create embedding_provider_registry table"
    );
}

fn path_meta(id: &str, relative: &str) -> SourceMeta {
    let mut meta = meta(id);
    meta.identity_kind = "repo_path".into();
    meta.identity_value = relative.into();
    meta
}

fn seed_scoped(store: &mut ContentStore) {
    for (id, path) in [
        ("src-auth-a", "src/auth/a.rs"),
        ("src-auth-b", "src/auth/sub/b.rs"),
        ("src-util-c", "src/util/c.rs"),
    ] {
        store
            .index_artifact(path_meta(id, path), "shared token content", "text/rust")
            .unwrap();
    }
}

#[test]
fn sources_store_canonical_relative_and_parent_dir() {
    let mut store = open_store();
    seed_scoped(&mut store);
    let row = store.get_source("src-auth-b").unwrap().unwrap();
    assert_eq!(row.identity_value, "src/auth/sub/b.rs");
    let (relative, parent): (Option<String>, Option<String>) = store
        .conn
        .query_row(
            "SELECT relative_path, parent_dir FROM sources WHERE id = ?1",
            rusqlite::params!["src-auth-b"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(relative.as_deref(), Some("src/auth/sub/b.rs"));
    assert_eq!(parent.as_deref(), Some("src/auth/sub"));
}

#[test]
fn content_search_applies_recursive_and_exact_scope() {
    let mut store = open_store();
    seed_scoped(&mut store);

    let recursive = store
        .search_with_fallback(
            "shared",
            &SearchFilters {
                subpath_prefix: Some("src/auth".into()),
                ..SearchFilters::default()
            },
        )
        .unwrap();
    let ids: Vec<&str> = recursive.iter().map(|c| c.source_id.as_str()).collect();
    assert!(ids.contains(&"src-auth-a"));
    assert!(ids.contains(&"src-auth-b"));
    assert!(!ids.contains(&"src-util-c"));

    let non_recursive = store
        .search_with_fallback(
            "shared",
            &SearchFilters {
                exact_dir: Some("src/auth".into()),
                ..SearchFilters::default()
            },
        )
        .unwrap();
    let ids: Vec<&str> = non_recursive.iter().map(|c| c.source_id.as_str()).collect();
    assert_eq!(ids, vec!["src-auth-a"]);
}

#[test]
fn content_search_applies_exclusions_and_limit() {
    let mut store = open_store();
    seed_scoped(&mut store);

    let excluded = store
        .search_with_fallback(
            "shared",
            &SearchFilters {
                subpath_prefix: Some("src".into()),
                exclude_prefixes: vec!["src/auth/sub".into()],
                ..SearchFilters::default()
            },
        )
        .unwrap();
    let ids: Vec<&str> = excluded.iter().map(|c| c.source_id.as_str()).collect();
    assert!(ids.contains(&"src-auth-a"));
    assert!(ids.contains(&"src-util-c"));
    assert!(!ids.contains(&"src-auth-b"));

    let limited = store
        .search_with_fallback(
            "shared",
            &SearchFilters {
                limit: 1,
                ..SearchFilters::default()
            },
        )
        .unwrap();
    assert_eq!(limited.len(), 1);
}

#[test]
fn content_search_rejects_noncanonical_scope() {
    let mut store = open_store();
    seed_scoped(&mut store);
    let error = store
        .search_with_fallback(
            "shared",
            &SearchFilters {
                subpath_prefix: Some("../etc".into()),
                ..SearchFilters::default()
            },
        )
        .expect_err("traversal scope must fail closed");
    assert!(error.to_string().contains("invalid directory scope"));
}

#[test]
fn search_limit_is_capped_at_hard_limit() {
    let filters = SearchFilters {
        limit: 10_000,
        ..SearchFilters::default()
    };
    assert_eq!(filters.effective_limit(), 500);
    assert_eq!(SearchFilters::default().effective_limit(), 50);
}
