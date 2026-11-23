use super::*;

// --- FTS search ----------------------------------------------------------

#[test]
fn fts_search_finds_indexed_node() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "replace_file_graph",
        "store.rs::fn::replace_file_graph",
        "store.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "store.rs",
            "h",
            Some("rust"),
            None,
            &[node],
            &[],
        )
        .unwrap();

    let q = SearchQuery {
        text: "replace_file_graph".to_string(),
        limit: 5,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].node.name, "replace_file_graph");
}

#[test]
fn fts_search_empty_query_returns_empty() {
    let store = open_in_memory();
    let q = SearchQuery {
        text: "".to_string(),
        ..Default::default()
    };
    let err = store.search(&q).unwrap_err();
    assert!(
        err.to_string().contains("non-empty text or regex pattern"),
        "expected empty-query error, got: {err}"
    );
}

#[test]
fn fts_search_kind_accepts_aliases() {
    let mut store = open_in_memory();
    let func = make_node(
        NodeKind::Function,
        "process",
        "a.rs::fn::process",
        "a.rs",
        "rust",
    );
    let strct = make_node(
        NodeKind::Struct,
        "ProcessConfig",
        "a.rs::struct::ProcessConfig",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[func, strct], &[])
        .unwrap();

    let canonical = store
        .search(&SearchQuery {
            text: "process".to_string(),
            kind: Some("function".to_string()),
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    let aliased = store
        .search(&SearchQuery {
            text: "process".to_string(),
            kind: Some("fn".to_string()),
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(aliased.len(), canonical.len());
    assert_eq!(aliased[0].node.name, "process");
    assert_eq!(aliased[0].node.kind, NodeKind::Function);

    // Regression: the structural scan path (empty text + regex) also honors aliases.
    let structural = store
        .search(&SearchQuery {
            text: String::new(),
            regex_pattern: Some("Process".to_string()),
            kind: Some("record".to_string()),
            limit: 10,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(structural.len(), 1);
    assert_eq!(structural[0].node.name, "ProcessConfig");
}

#[test]
fn fts_search_respects_kind_filter() {
    let mut store = open_in_memory();
    let func = make_node(
        NodeKind::Function,
        "process",
        "a.rs::fn::process",
        "a.rs",
        "rust",
    );
    let strct = make_node(
        NodeKind::Struct,
        "ProcessConfig",
        "a.rs::struct::ProcessConfig",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[func, strct], &[])
        .unwrap();

    let q = SearchQuery {
        text: "process".to_string(),
        kind: Some("struct".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(
        results
            .iter()
            .all(|r| matches!(r.node.kind, NodeKind::Struct))
    );
}

#[test]
fn fts_search_respects_repo_id_filter() {
    let mut store = open_in_memory();
    let left = make_node(
        NodeKind::Function,
        "shared_name",
        "repo::repo_left::a.rs::fn::shared_name",
        "a.rs",
        "rust",
    );
    let right = make_node(
        NodeKind::Function,
        "shared_name",
        "repo::repo_right::a.rs::fn::shared_name",
        "a.rs",
        "rust",
    );
    store
        .replace_files_transactional_for_repo(
            "repo_left",
            &[ParsedFile {
                path: "a.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h1".to_string(),
                size: None,
                nodes: vec![left],
                edges: vec![],
            }],
        )
        .unwrap();
    store
        .replace_files_transactional_for_repo(
            "repo_right",
            &[ParsedFile {
                path: "a.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h2".to_string(),
                size: None,
                nodes: vec![right],
                edges: vec![],
            }],
        )
        .unwrap();

    let q = SearchQuery {
        text: "shared_name".to_string(),
        limit: 10,
        repo_ids: vec!["repo_right".to_string()],
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].node.qualified_name,
        "repo::repo_right::a.rs::fn::shared_name"
    );
}

#[test]
fn fts_search_not_found_after_delete() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "vanishing_fn",
        "a.rs::fn::vanishing_fn",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[node], &[])
        .unwrap();
    store.delete_file_graph("a.rs").unwrap();

    let q = SearchQuery {
        text: "vanishing_fn".to_string(),
        limit: 5,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(results.is_empty());
}

// --- FTS language / file_path / is_test filters --------------------------

// --- regex post-filter tests ---------------------------------------------

#[test]
fn regex_matches_name() {
    let mut store = open_in_memory();
    let f1 = make_node(
        NodeKind::Function,
        "handle_request",
        "a.rs::fn::handle_request",
        "a.rs",
        "rust",
    );
    let f2 = make_node(
        NodeKind::Function,
        "parse_body",
        "a.rs::fn::parse_body",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[f1, f2], &[])
        .unwrap();

    let q = SearchQuery {
        text: "handle".to_string(),
        regex_pattern: Some(r"handle_\w+".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty(), "expected at least one result");
    for r in &results {
        assert!(r.node.name.contains("handle") || r.node.qualified_name.contains("handle"));
    }
}

#[test]
fn regex_udf_eval_exercises_query_udf_path() {
    assert!(Store::eval_regexp_udf(r"foo\d+", "foo42").unwrap());

    let err = Store::eval_regexp_udf("(", "foo42").unwrap_err();
    assert!(
        err.to_string().contains("regex parse error"),
        "expected regex parse error, got: {err}"
    );
}

#[test]
fn regex_matches_qualified_name() {
    let mut store = open_in_memory();
    let f1 = make_node(
        NodeKind::Function,
        "foo",
        "pkg::service::foo",
        "a.rs",
        "rust",
    );
    let f2 = make_node(
        NodeKind::Function,
        "bar",
        "pkg::service::bar",
        "a.rs",
        "rust",
    );
    let f3 = make_node(NodeKind::Function, "baz", "pkg::util::baz", "a.rs", "rust");
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[f1, f2, f3], &[])
        .unwrap();

    // FTS text is empty → structural scan, regex filters qualified name
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"::service::".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 2);
    for r in &results {
        assert!(r.node.qualified_name.contains("::service::"));
    }
}

#[test]
fn regex_structural_scan_empty_text() {
    let mut store = open_in_memory();
    let f1 = make_node(
        NodeKind::Function,
        "fn_alpha",
        "a.rs::fn::fn_alpha",
        "a.rs",
        "rust",
    );
    let s1 = make_node(
        NodeKind::Struct,
        "MyStruct",
        "a.rs::struct::MyStruct",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[f1, s1], &[])
        .unwrap();

    // regex matches only lower-case names starting with fn_
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^fn_".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.name, "fn_alpha");
}

#[test]
fn regex_invalid_pattern_returns_error() {
    let store = open_in_memory();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some("[invalid".to_string()),
        limit: 10,
        ..Default::default()
    };
    let err = store.search(&q).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("invalid regex"),
        "expected invalid regex message, got: {msg}"
    );
}

#[test]
fn regex_combined_with_fts_postfilters() {
    let mut store = open_in_memory();
    let f1 = make_node(
        NodeKind::Function,
        "search_fast",
        "a.rs::fn::search_fast",
        "a.rs",
        "rust",
    );
    let f2 = make_node(
        NodeKind::Function,
        "search_slow",
        "a.rs::fn::search_slow",
        "a.rs",
        "rust",
    );
    let f3 = make_node(
        NodeKind::Function,
        "other_fn",
        "a.rs::fn::other_fn",
        "a.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[f1, f2, f3], &[])
        .unwrap();

    let q = SearchQuery {
        text: "search".to_string(),
        regex_pattern: Some(r"search_fast".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.name, "search_fast");
}

#[test]
fn regex_none_empty_text_returns_empty() {
    let store = open_in_memory();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: None,
        limit: 10,
        ..Default::default()
    };
    let err = store.search(&q).unwrap_err();
    assert!(
        err.to_string().contains("non-empty text or regex pattern"),
        "expected empty-query error, got: {err}"
    );
}

// --- regex UDF comprehensive tests --------------------------------------
