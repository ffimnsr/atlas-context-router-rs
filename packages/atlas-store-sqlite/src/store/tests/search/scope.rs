use super::*;

fn seed_dsq_nodes(store: &mut Store) {
    for (qname, path) in [
        ("src/auth/a.rs::fn::process_a", "src/auth/a.rs"),
        ("src/auth/sub/b.rs::fn::process_b", "src/auth/sub/b.rs"),
        ("src/util/c.rs::fn::process_c", "src/util/c.rs"),
    ] {
        let node = make_node(
            NodeKind::Function,
            qname.rsplit("::").next().unwrap_or(qname),
            qname,
            path,
            "rust",
        );
        store
            .replace_file_graph_for_repo("repo_test", path, "h", Some("rust"), None, &[node], &[])
            .unwrap();
    }
}

fn search_paths(store: &Store, query: SearchQuery) -> Vec<String> {
    let mut paths: Vec<String> = store
        .search(&query)
        .unwrap()
        .into_iter()
        .map(|node| node.node.file_path)
        .collect();
    paths.sort();
    paths
}

#[test]
fn fts_search_applies_exact_dir_scope() {
    let mut store = open_in_memory();
    seed_dsq_nodes(&mut store);
    let paths = search_paths(
        &store,
        SearchQuery {
            text: "process".to_string(),
            exact_dir: Some("src/auth".to_string()),
            limit: 10,
            ..Default::default()
        },
    );
    assert_eq!(paths, vec!["src/auth/a.rs"]);
}

#[test]
fn fts_search_applies_exclude_subpaths() {
    let mut store = open_in_memory();
    seed_dsq_nodes(&mut store);
    let paths = search_paths(
        &store,
        SearchQuery {
            text: "process".to_string(),
            subpath: Some("src".to_string()),
            exclude_subpaths: vec!["src/auth/sub".to_string()],
            limit: 10,
            ..Default::default()
        },
    );
    assert_eq!(paths, vec!["src/auth/a.rs", "src/util/c.rs"]);
}

#[test]
fn fts_search_rejects_noncanonical_exact_dir() {
    let store = open_in_memory();
    let error = store
        .search(&SearchQuery {
            text: "process".to_string(),
            exact_dir: Some("../etc".to_string()),
            ..Default::default()
        })
        .unwrap_err();
    assert!(error.to_string().contains("invalid exact directory scope"));
}

#[test]
fn vector_search_applies_scope_before_scoring() {
    let mut store = open_in_memory();
    seed_dsq_nodes(&mut store);
    for qname in [
        "src/auth/a.rs::fn::process_a",
        "src/auth/sub/b.rs::fn::process_b",
        "src/util/c.rs::fn::process_c",
    ] {
        store
            .upsert_chunk("repo_test", qname, 0, "process text")
            .unwrap();
        let id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM retrieval_chunks WHERE node_qn = ?1",
                [qname],
                |row| row.get(0),
            )
            .unwrap();
        store.set_chunk_embedding(id, &[1.0, 0.0]).unwrap();
    }

    let (results, candidates) = store
        .nodes_by_vector_similarity_with_stats(
            &[1.0, 0.0],
            &VectorScope {
                subpath_prefix: Some("src/auth".to_string()),
                repo_ids: vec!["repo_test".to_string()],
                ..Default::default()
            },
            10,
        )
        .unwrap();
    assert_eq!(candidates, 2);
    assert!(
        results
            .iter()
            .all(|node| node.node.file_path.starts_with("src/auth"))
    );

    let error = store
        .nodes_by_vector_similarity(
            &[1.0, 0.0],
            &VectorScope {
                subpath_prefix: Some("src/auth".to_string()),
                ..Default::default()
            },
            10,
        )
        .unwrap_err();
    assert!(error.to_string().contains("explicit repo scope"));
}

#[test]
fn triehi_bitmap_matches_sql_scope_and_reduces_candidates() {
    let mut store = open_in_memory();
    seed_dsq_nodes(&mut store);
    for qname in [
        "src/auth/a.rs::fn::process_a",
        "src/auth/sub/b.rs::fn::process_b",
        "src/util/c.rs::fn::process_c",
    ] {
        store
            .upsert_chunk("repo_test", qname, 0, "process text")
            .unwrap();
        let id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM retrieval_chunks WHERE node_qn = ?1",
                [qname],
                |row| row.get(0),
            )
            .unwrap();
        store.set_chunk_embedding(id, &[1.0, 0.0]).unwrap();
    }

    let scope = |min_chunks: usize| VectorScope {
        subpath_prefix: Some("src/auth".to_string()),
        repo_ids: vec!["repo_test".to_string()],
        triehi_min_chunks: min_chunks,
        ..Default::default()
    };

    // `triehi_min_chunks = 0` forces the bitmap path; `usize::MAX` keeps SQL.
    let (trie_results, trie_candidates) = store
        .nodes_by_vector_similarity_with_stats(&[1.0, 0.0], &scope(0), 10)
        .unwrap();
    let (sql_results, sql_candidates) = store
        .nodes_by_vector_similarity_with_stats(&[1.0, 0.0], &scope(usize::MAX), 10)
        .unwrap();

    let mut trie_paths: Vec<String> = trie_results
        .iter()
        .map(|node| node.node.file_path.clone())
        .collect();
    let mut sql_paths: Vec<String> = sql_results
        .iter()
        .map(|node| node.node.file_path.clone())
        .collect();
    trie_paths.sort();
    sql_paths.sort();

    assert_eq!(trie_paths, sql_paths);
    assert_eq!(trie_paths, vec!["src/auth/a.rs", "src/auth/sub/b.rs"]);
    assert_eq!(trie_candidates, 2);
    assert_eq!(trie_candidates, sql_candidates);

    let (unscoped, unscoped_candidates) = store
        .nodes_by_vector_similarity_with_stats(
            &[1.0, 0.0],
            &VectorScope {
                repo_ids: vec!["repo_test".to_string()],
                triehi_min_chunks: 0,
                ..Default::default()
            },
            10,
        )
        .unwrap();
    assert_eq!(unscoped_candidates, 3);
    assert!(unscoped.len() >= trie_results.len());
}

#[test]
fn trie_relinks_chunks_after_stable_file_rename() {
    let mut store = open_in_memory();
    seed_dsq_nodes(&mut store);
    for qname in [
        "src/auth/a.rs::fn::process_a",
        "src/util/c.rs::fn::process_c",
    ] {
        store
            .upsert_chunk("repo_test", qname, 0, "process text")
            .unwrap();
        let id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM retrieval_chunks WHERE node_qn = ?1",
                [qname],
                |row| row.get(0),
            )
            .unwrap();
        store.set_chunk_embedding(id, &[1.0, 0.0]).unwrap();
    }

    let scope = |prefix: &str| VectorScope {
        subpath_prefix: Some(prefix.to_owned()),
        repo_ids: vec!["repo_test".to_string()],
        triehi_min_chunks: 0,
        ..Default::default()
    };
    // Force the trie cache to build before the rename.
    let _ = store
        .nodes_by_vector_similarity(&[1.0, 0.0], &scope("src/auth"), 10)
        .unwrap();

    store
        .rename_file_graph_for_repo("repo_test", "src/auth/a.rs", "src/moved/a.rs")
        .unwrap();

    let old_scope = store
        .nodes_by_vector_similarity(&[1.0, 0.0], &scope("src/auth"), 10)
        .unwrap();
    assert!(old_scope.is_empty());
    let new_scope = store
        .nodes_by_vector_similarity(&[1.0, 0.0], &scope("src/moved"), 10)
        .unwrap();
    assert_eq!(new_scope.len(), 1);
    assert_eq!(new_scope[0].node.file_path, "src/moved/a.rs");
}

// --- traverse_from_qnames -------------------------------------------------

#[test]
fn traverse_from_qnames_finds_dependents_when_seed_set_meets_node_cap() {
    let mut store = open_in_memory();
    let seeds = (0..5)
        .map(|index| {
            make_node(
                NodeKind::Function,
                &format!("f{index}"),
                &format!("a.rs::fn::f{index}"),
                "a.rs",
                "rust",
            )
        })
        .collect::<Vec<_>>();
    let dependent = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
    let edge = make_edge(EdgeKind::Calls, "a.rs::fn::f0", "b.rs::fn::b", "a.rs");
    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &seeds, &[edge])
        .unwrap();
    store
        .replace_file_graph_for_repo("repo_test", "b.rs", "h", None, None, &[dependent], &[])
        .unwrap();

    let seed_qnames: Vec<&str> = seeds
        .iter()
        .map(|node| node.qualified_name.as_str())
        .collect();
    let result = store.traverse_from_qnames(&seed_qnames, 2, 2, 100).unwrap();

    assert_eq!(
        result.changed_nodes.len(),
        1,
        "seed set yields half the budget"
    );
    assert!(
        result
            .impacted_nodes
            .iter()
            .any(|node| node.qualified_name == "b.rs::fn::b"),
        "dependents must survive a seed-heavy traversal"
    );
    assert!(result.impacted_files.iter().any(|entry| entry == "b.rs"));
}
