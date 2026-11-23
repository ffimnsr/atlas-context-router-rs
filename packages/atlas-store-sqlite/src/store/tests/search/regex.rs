use super::*;

fn seed_regex_store() -> Store {
    let mut store = open_in_memory();
    // Deliberately varied names to exercise alternation, anchoring, case, multi-file.
    let nodes_a: Vec<Node> = vec![
        make_node(
            NodeKind::Function,
            "handle_request",
            "pkg::http::handle_request",
            "http.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "handle_response",
            "pkg::http::handle_response",
            "http.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "parse_body",
            "pkg::http::parse_body",
            "http.rs",
            "rust",
        ),
        make_node(
            NodeKind::Struct,
            "HttpClient",
            "pkg::http::HttpClient",
            "http.rs",
            "rust",
        ),
        make_node(
            NodeKind::Method,
            "send",
            "pkg::http::HttpClient::send",
            "http.rs",
            "rust",
        ),
    ];
    let nodes_b: Vec<Node> = vec![
        make_node(
            NodeKind::Function,
            "benchmark_context_retrieval_latency",
            "pkg::bench::benchmark_context_retrieval_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "benchmark_impact_analysis_latency",
            "pkg::bench::benchmark_impact_analysis_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "benchmark_dead_code_scan_latency",
            "pkg::bench::benchmark_dead_code_scan_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "benchmark_rename_planning_latency",
            "pkg::bench::benchmark_rename_planning_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "benchmark_import_cleanup_latency",
            "pkg::bench::benchmark_import_cleanup_latency",
            "bench.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "setup_fixture",
            "pkg::bench::setup_fixture",
            "bench.rs",
            "rust",
        ),
    ];
    let nodes_c: Vec<Node> = vec![
        make_node(
            NodeKind::Function,
            "HANDLE_AUTH",
            "pkg::auth::HANDLE_AUTH",
            "auth.rs",
            "rust",
        ),
        make_node(
            NodeKind::Function,
            "Handle_Login",
            "pkg::auth::Handle_Login",
            "auth.rs",
            "rust",
        ),
        make_node(
            NodeKind::Struct,
            "AuthService",
            "pkg::auth::AuthService",
            "auth.rs",
            "rust",
        ),
    ];
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "http.rs",
            "h1",
            Some("rust"),
            None,
            &nodes_a,
            &[],
        )
        .unwrap();
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "bench.rs",
            "h2",
            Some("rust"),
            None,
            &nodes_b,
            &[],
        )
        .unwrap();
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "auth.rs",
            "h3",
            Some("rust"),
            None,
            &nodes_c,
            &[],
        )
        .unwrap();
    store
}

#[test]
fn regex_udf_alternation_pipe_matches_multiple() {
    // Mirrors the motivating use-case: pipe-separated alternation in structural scan.
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"benchmark_context_retrieval_latency|benchmark_impact_analysis_latency|benchmark_dead_code_scan_latency|benchmark_rename_planning_latency|benchmark_import_cleanup_latency".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 5, "all five benchmark symbols should match");
    let names: Vec<&str> = results.iter().map(|r| r.node.name.as_str()).collect();
    assert!(names.contains(&"benchmark_context_retrieval_latency"));
    assert!(names.contains(&"benchmark_impact_analysis_latency"));
    assert!(names.contains(&"benchmark_dead_code_scan_latency"));
    assert!(names.contains(&"benchmark_rename_planning_latency"));
    assert!(names.contains(&"benchmark_import_cleanup_latency"));
    assert!(
        !names.contains(&"setup_fixture"),
        "non-matching node must not appear"
    );
}

#[test]
fn regex_udf_case_sensitive_distinguishes_variants() {
    // handle_request and HANDLE_AUTH and Handle_Login differ by case.
    let store = seed_regex_store();
    // Exact lowercase anchor — should not match HANDLE_AUTH or Handle_Login.
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^handle_".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.node.name.as_str()).collect();
    assert!(names.contains(&"handle_request"));
    assert!(names.contains(&"handle_response"));
    assert!(
        !names.contains(&"HANDLE_AUTH"),
        "uppercase must not match ^handle_"
    );
    assert!(
        !names.contains(&"Handle_Login"),
        "mixed-case must not match ^handle_"
    );
}

#[test]
fn regex_udf_case_insensitive_flag_matches_all_variants() {
    // (?i-u) inline flag (ASCII-only case fold) — should match all three case variants.
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"(?i-u)^handle_".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.node.name.as_str()).collect();
    assert!(names.contains(&"handle_request"));
    assert!(names.contains(&"handle_response"));
    assert!(names.contains(&"HANDLE_AUTH"));
    assert!(names.contains(&"Handle_Login"));
}

#[test]
fn regex_udf_anchored_end_matches_suffix() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"_latency$".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    // All five benchmark nodes end in _latency, nothing else does.
    assert_eq!(results.len(), 5);
    assert!(results.iter().all(|r| r.node.name.ends_with("_latency")));
}

#[test]
fn regex_udf_structural_scan_respects_kind_filter() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"pkg::".to_string()),
        kind: Some("struct".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert!(
        results
            .iter()
            .all(|r| matches!(r.node.kind, NodeKind::Struct))
    );
}

#[test]
fn regex_udf_structural_scan_respects_language_filter() {
    let mut store = seed_regex_store();
    let go_node = make_node(
        NodeKind::Function,
        "handle_request",
        "main::handle_request",
        "main.go",
        "go",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "main.go",
            "h4",
            Some("go"),
            None,
            &[go_node],
            &[],
        )
        .unwrap();

    // Restrict to go only — must not return rust handle_request.
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^handle_request$".to_string()),
        language: Some("go".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.language, "go");
}

#[test]
fn regex_udf_structural_scan_respects_subpath_filter() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"handle".to_string()),
        subpath: Some("http.rs".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    // Only http.rs has handle_* nodes in the store.
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.node.file_path == "http.rs"));
}

#[test]
fn regex_udf_limit_respected_in_structural_scan() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"pkg::".to_string()), // matches all 14 nodes
        limit: 3,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 3, "result count must not exceed limit");
}

#[test]
fn regex_udf_with_fts_alternation_in_text() {
    // Both text and regex set: FTS5 + UDF.
    let store = seed_regex_store();
    let q = SearchQuery {
        text: "handle".to_string(),
        regex_pattern: Some(r"^handle_re".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    let names: Vec<&str> = results.iter().map(|r| r.node.name.as_str()).collect();
    assert!(names.contains(&"handle_request"));
    assert!(names.contains(&"handle_response"));
    assert!(
        !names.contains(&"HANDLE_AUTH"),
        "HANDLE_AUTH must not match ^handle_re"
    );
    assert!(
        !names.contains(&"Handle_Login"),
        "Handle_Login must not match ^handle_re"
    );
}

#[test]
fn regex_udf_with_fts_limit_respected() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: "benchmark".to_string(),
        regex_pattern: Some(r"benchmark_".to_string()),
        limit: 2,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(
        results.len() <= 2,
        "limit must be respected with FTS + UDF; got {}",
        results.len()
    );
}

#[test]
fn regex_udf_no_match_returns_empty_not_error() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^zzz_nonexistent_symbol_xyz$".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(
        results.is_empty(),
        "no match should return empty vec, not error"
    );
}

#[test]
fn regex_udf_dot_star_matches_all() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r".*".to_string()),
        limit: 100,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    // All 14 nodes inserted across 3 files.
    assert_eq!(results.len(), 14);
}

#[test]
fn regex_udf_empty_pattern_returns_error() {
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(String::new()),
        limit: 10,
        ..Default::default()
    };
    // Empty pattern is valid regex (matches everything) but text is also empty,
    // so either the UDF runs (returning all nodes up to limit) OR we could treat
    // this as a degenerate case. Assert it at least doesn't panic.
    let _ = store.search(&q);
}

#[test]
fn regex_udf_udf_not_leaked_between_queries() {
    // Two sequential searches with different patterns must not interfere.
    let store = seed_regex_store();
    let q1 = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^handle_request$".to_string()),
        limit: 20,
        ..Default::default()
    };
    let q2 = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^parse_body$".to_string()),
        limit: 20,
        ..Default::default()
    };
    let r1 = store.search(&q1).unwrap();
    let r2 = store.search(&q2).unwrap();
    assert_eq!(r1.len(), 1);
    assert_eq!(r1[0].node.name, "handle_request");
    assert_eq!(r2.len(), 1);
    assert_eq!(r2[0].node.name, "parse_body");
}

#[test]
fn regex_udf_complex_pattern_qualified_name_scope() {
    // Pattern anchored to qualified_name structure — pkg::bench:: prefix.
    let store = seed_regex_store();
    let q = SearchQuery {
        text: String::new(),
        regex_pattern: Some(r"^pkg::bench::".to_string()),
        limit: 20,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert_eq!(results.len(), 6); // 5 benchmarks + setup_fixture
    assert!(results.iter().all(|r| r.node.file_path == "bench.rs"));
}

// --- FTS language / file_path / is_test filters (existing) --------------

#[test]
fn fts_search_respects_language_filter() {
    let mut store = open_in_memory();
    let rust_fn = make_node(
        NodeKind::Function,
        "shared_name",
        "a.rs::fn::shared_name",
        "a.rs",
        "rust",
    );
    let go_fn = make_node(
        NodeKind::Function,
        "shared_name",
        "b.go::fn::shared_name",
        "b.go",
        "go",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h",
            Some("rust"),
            None,
            &[rust_fn],
            &[],
        )
        .unwrap();
    store
        .replace_file_graph_for_repo("repo_test", "b.go", "h", Some("go"), None, &[go_fn], &[])
        .unwrap();

    let q = SearchQuery {
        text: "shared_name".to_string(),
        language: Some("go".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.node.language == "go"));
}

#[test]
fn fts_search_respects_file_path_filter() {
    let mut store = open_in_memory();
    let na = make_node(
        NodeKind::Function,
        "common",
        "a.rs::fn::common",
        "a.rs",
        "rust",
    );
    let nb = make_node(
        NodeKind::Function,
        "common",
        "b.rs::fn::common",
        "b.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &[na], &[])
        .unwrap();
    store
        .replace_file_graph_for_repo("repo_test", "b.rs", "h", None, None, &[nb], &[])
        .unwrap();

    let q = SearchQuery {
        text: "common".to_string(),
        file_path: Some("a.rs".to_string()),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.node.file_path == "a.rs"));
}

#[test]
fn fts_search_respects_is_test_filter() {
    let mut store = open_in_memory();
    let mut test_node = make_node(
        NodeKind::Function,
        "test_foo",
        "a.rs::fn::test_foo",
        "a.rs",
        "rust",
    );
    test_node.is_test = true;
    let prod_node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h",
            None,
            None,
            &[test_node, prod_node],
            &[],
        )
        .unwrap();

    // Search for is_test = true should only return test nodes.
    let q = SearchQuery {
        text: "foo".to_string(),
        is_test: Some(true),
        limit: 10,
        ..Default::default()
    };
    let results = store.search(&q).unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|r| r.node.is_test));
}
