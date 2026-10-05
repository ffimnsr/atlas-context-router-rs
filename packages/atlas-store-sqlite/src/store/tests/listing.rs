use super::*;

fn seed_listing_store() -> Store {
    let mut store = open_in_memory();
    let api = make_node(
        NodeKind::Function,
        "handle_request",
        "src/api.rs::fn::handle_request",
        "src/api.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo("repo_test", "src/api.rs", "h", None, None, &[api], &[])
        .unwrap();

    let compute = make_node(
        NodeKind::Function,
        "compute",
        "src/service.rs::fn::compute",
        "src/service.rs",
        "rust",
    );
    let service = make_node(
        NodeKind::Struct,
        "Service",
        "src/service.rs::struct::Service",
        "src/service.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "src/service.rs",
            "h",
            None,
            None,
            &[compute, service],
            &[],
        )
        .unwrap();

    let test_node = make_node(
        NodeKind::Test,
        "compute_test",
        "tests/service_test.rs::fn::compute_test",
        "tests/service_test.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "tests/service_test.rs",
            "h",
            None,
            None,
            &[test_node],
            &[],
        )
        .unwrap();

    let helper = make_node(
        NodeKind::Function,
        "helper",
        "packages/core/lib.rs::fn::helper",
        "packages/core/lib.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "packages/core/lib.rs",
            "h",
            None,
            None,
            &[helper],
            &[],
        )
        .unwrap();

    store
}

#[test]
fn list_nodes_paginates_in_deterministic_order() {
    let store = seed_listing_store();
    let filter = NodeListFilter::default();

    let first = store.list_nodes(&filter, 2, 0).unwrap();
    assert_eq!(first.total, 5);
    assert_eq!(first.limit, 2);
    assert_eq!(
        first
            .nodes
            .iter()
            .map(|node| node.qualified_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "packages/core/lib.rs::fn::helper",
            "src/api.rs::fn::handle_request"
        ]
    );
    assert!(first.has_more());
    assert_eq!(first.next_offset(), Some(2));

    let last = store.list_nodes(&filter, 2, 4).unwrap();
    assert_eq!(last.nodes.len(), 1);
    assert!(!last.has_more());
    assert_eq!(last.next_offset(), None);

    let past_end = store.list_nodes(&filter, 2, 99).unwrap();
    assert!(past_end.nodes.is_empty());
    assert_eq!(past_end.total, 5);
    assert_eq!(past_end.next_offset(), None);
}

#[test]
fn list_nodes_filters_by_kind_language_and_subpath() {
    let store = seed_listing_store();

    let functions = store
        .list_nodes(
            &NodeListFilter {
                kind: Some("function"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(functions.total, 3);

    // Store-level alias normalization: `fn` behaves like `function`.
    let aliased = store
        .list_nodes(
            &NodeListFilter {
                kind: Some("fn"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(aliased.total, functions.total);

    let rust_only = store
        .list_nodes(
            &NodeListFilter {
                language: Some("rust"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(rust_only.total, 5);

    let src_prefix = store
        .list_nodes(
            &NodeListFilter {
                subpath: Some("src"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(src_prefix.total, 3);
    assert!(
        src_prefix
            .nodes
            .iter()
            .all(|node| node.file_path.starts_with("src/"))
    );

    let exact_prefix = store
        .list_nodes(
            &NodeListFilter {
                subpath: Some("src/service"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(exact_prefix.total, 2);

    let missing = store
        .list_nodes(
            &NodeListFilter {
                kind: Some("enum"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(missing.total, 0);
    assert!(missing.nodes.is_empty());
}

#[test]
fn list_nodes_clamps_zero_limit_to_one() {
    let store = seed_listing_store();
    let page = store.list_nodes(&NodeListFilter::default(), 0, 0).unwrap();
    assert_eq!(page.limit, 1);
    assert_eq!(page.nodes.len(), 1);
}

#[test]
fn list_nodes_filters_by_repo_id_column() {
    let store = seed_listing_store();

    let current = store
        .list_nodes(
            &NodeListFilter {
                repo_id: Some("repo_test"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(current.total, 5);

    let other = store
        .list_nodes(
            &NodeListFilter {
                repo_id: Some("repo_other"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(other.total, 0);
    assert!(other.nodes.is_empty());
}

#[test]
fn list_nodes_subpath_matches_exact_paths_and_unicode_prefixes() {
    let mut store = seed_listing_store();
    let unicode_a = make_node(
        NodeKind::Module,
        "unicodemd",
        "docs/ünicode.md::module::document",
        "docs/ünicode.md",
        "markdown",
    );
    let unicode_b = make_node(
        NodeKind::Module,
        "unicodeextra",
        "docs/ünicode_extra.md::module::document",
        "docs/ünicode_extra.md",
        "markdown",
    );
    let zeta = make_node(
        NodeKind::Module,
        "zeta",
        "docs/zeta.md::module::document",
        "docs/zeta.md",
        "markdown",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "docs/ünicode.md",
            "h",
            None,
            None,
            &[unicode_a],
            &[],
        )
        .unwrap();
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "docs/ünicode_extra.md",
            "h",
            None,
            None,
            &[unicode_b],
            &[],
        )
        .unwrap();
    store
        .replace_file_graph_for_repo("repo_test", "docs/zeta.md", "h", None, None, &[zeta], &[])
        .unwrap();

    let unicode_prefix = store
        .list_nodes(
            &NodeListFilter {
                subpath: Some("docs/ü"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(unicode_prefix.total, 2);
    assert!(
        unicode_prefix
            .nodes
            .iter()
            .all(|node| node.file_path.starts_with("docs/ü"))
    );

    let exact_path = store
        .list_nodes(
            &NodeListFilter {
                subpath: Some("src/api.rs"),
                ..Default::default()
            },
            10,
            0,
        )
        .unwrap();
    assert_eq!(exact_path.total, 1);
    assert_eq!(
        exact_path.nodes[0].qualified_name,
        "src/api.rs::fn::handle_request"
    );
}

#[test]
fn prefix_upper_bound_handles_ascii_unicode_and_boundaries() {
    use crate::store::graph::prefix_upper_bound;

    assert_eq!(prefix_upper_bound("a").as_deref(), Some("b"));
    assert_eq!(prefix_upper_bound("src/").as_deref(), Some("src0"));
    assert_eq!(prefix_upper_bound("ü").as_deref(), Some("ý"));
    // Skips the surrogate code-point gap instead of panicking.
    assert_eq!(prefix_upper_bound("\u{D7FF}").as_deref(), Some("\u{E000}"));
    // Trailing maximal scalar is dropped and the previous char increments.
    assert_eq!(prefix_upper_bound("a\u{10FFFF}").as_deref(), Some("b"));
    // No successor exists for the empty or all-maximal prefix.
    assert_eq!(prefix_upper_bound(""), None);
    assert_eq!(prefix_upper_bound("\u{10FFFF}"), None);
}

#[test]
fn list_nodes_subpath_matches_substr_reference() {
    let store = seed_listing_store();

    for prefix in [
        "p",
        "packages",
        "packages/core",
        "packages/core/lib.rs",
        "s",
        "src",
        "src/",
        "src/a",
        "src/api",
        "src/api.rs",
        "src/service",
        "t",
        "tests/",
        "zzz",
    ] {
        let expected_total: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE substr(file_path, 1, length(?1)) = ?1",
                [prefix],
                |row| row.get(0),
            )
            .unwrap();
        let page = store
            .list_nodes(
                &NodeListFilter {
                    subpath: Some(prefix),
                    ..Default::default()
                },
                100,
                0,
            )
            .unwrap();
        assert_eq!(page.total as i64, expected_total, "prefix {prefix:?} total");

        let mut stmt = store
            .conn
            .prepare(
                "SELECT qualified_name FROM nodes
                 WHERE substr(file_path, 1, length(?1)) = ?1
                 ORDER BY file_path, line_start, qualified_name",
            )
            .unwrap();
        let expected = stmt
            .query_map([prefix], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let actual = page
            .nodes
            .iter()
            .map(|node| node.qualified_name.clone())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "prefix {prefix:?} rows");
    }
}

#[test]
fn list_nodes_subpath_filter_uses_file_path_index() {
    let store = seed_listing_store();
    let mut stmt = store
        .conn
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT id FROM nodes
             WHERE file_path >= 'src' AND file_path < 'srct'
             ORDER BY file_path, line_start, qualified_name
             LIMIT 10",
        )
        .unwrap();
    let plan = stmt
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .filter_map(Result::ok)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("idx_nodes_file_path"),
        "subpath range filter should use the file_path index, plan was: {plan}"
    );
}
