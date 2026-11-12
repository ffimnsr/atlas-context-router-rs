use super::*;

fn parsed_file() -> ParsedFile {
    ParsedFile {
        path: "src/a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "vec-h1".to_string(),
        size: None,
        nodes: vec![
            make_node(
                NodeKind::Function,
                "x",
                "src/a.rs::fn::x",
                "src/a.rs",
                "rust",
            ),
            make_node(
                NodeKind::Function,
                "y",
                "src/a.rs::fn::y",
                "src/a.rs",
                "rust",
            ),
        ],
        edges: vec![],
    }
}

fn seed_vectors(store: &mut Store) {
    store
        .replace_files_transactional_for_repo("repo_a", &[parsed_file()])
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", &[parsed_file()])
        .unwrap();

    let id = |qn: &str| -> i64 {
        store
            .conn
            .query_row(
                "SELECT id FROM retrieval_chunks WHERE node_qn = ?1",
                [qn],
                |row| row.get(0),
            )
            .unwrap()
    };
    store
        .set_chunk_embedding(id("src/a.rs::fn::x"), &[1.0, 0.0])
        .unwrap();
    store
        .set_chunk_embedding(id("src/a.rs::fn::y"), &[0.0, 1.0])
        .unwrap();
}

fn vec_table_count(store: &Store) -> i64 {
    store
        .conn
        .query_row("SELECT COUNT(*) FROM retrieval_vec_2", [], |row| row.get(0))
        .unwrap()
}

fn chunk_id(store: &Store, qn: &str) -> i64 {
    store
        .conn
        .query_row(
            "SELECT id FROM retrieval_chunks WHERE node_qn = ?1",
            [qn],
            |row| row.get(0),
        )
        .unwrap()
}

fn seed_scoped_vectors(store: &mut Store) {
    let file_a = ParsedFile {
        path: "src/a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "vec-a".to_string(),
        size: None,
        nodes: vec![
            make_node(
                NodeKind::Function,
                "x",
                "src/a.rs::fn::x",
                "src/a.rs",
                "rust",
            ),
            make_node(
                NodeKind::Function,
                "y",
                "src/a.rs::fn::y",
                "src/a.rs",
                "rust",
            ),
        ],
        edges: vec![],
    };
    let file_py = ParsedFile {
        path: "src/nested/b.py".to_string(),
        language: Some("python".to_string()),
        hash: "vec-b".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Class,
            "z",
            "src/nested/b.py::class::z",
            "src/nested/b.py",
            "python",
        )],
        edges: vec![],
    };
    let file_repo_b = ParsedFile {
        path: "lib/w.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "vec-c".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            "w",
            "lib/w.rs::fn::w",
            "lib/w.rs",
            "rust",
        )],
        edges: vec![],
    };

    store
        .replace_files_transactional_for_repo("repo_a", &[file_a.clone(), file_py.clone()])
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", &[file_a, file_py])
        .unwrap();
    store
        .replace_files_transactional_for_repo("repo_b", std::slice::from_ref(&file_repo_b))
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_b", &[file_repo_b])
        .unwrap();

    for (qn, vector) in [
        ("src/a.rs::fn::x", [1.0, 0.0]),
        ("src/a.rs::fn::y", [0.0, 1.0]),
        ("src/nested/b.py::class::z", [0.9, 0.1]),
        ("lib/w.rs::fn::w", [1.0, 0.0]),
    ] {
        store
            .set_chunk_embedding(chunk_id(store, qn), &vector)
            .unwrap();
    }
}

/// Zero the stored BLOB while leaving the vec0 row intact.
///
/// Cosine similarity against a zero vector is `0.0`, so a non-zero score proves
/// the query was answered by the vec0 index rather than the BLOB scan.
fn poison_blob(store: &Store, qn: &str) {
    let blob_len: i64 = store
        .conn
        .query_row(
            "SELECT length(embedding) FROM retrieval_chunks WHERE node_qn = ?1",
            [qn],
            |row| row.get(0),
        )
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE retrieval_chunks SET embedding = zeroblob(?1) WHERE node_qn = ?2",
            rusqlite::params![blob_len, qn],
        )
        .unwrap();
}

fn score_of(results: &[atlas_core::ScoredNode], qn: &str) -> f64 {
    results
        .iter()
        .find(|node| node.node.qualified_name == qn)
        .unwrap_or_else(|| panic!("missing {qn} in results"))
        .score
}

#[test]
fn unscoped_vector_query_uses_vec0_index() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);

    let version: String = store
        .conn
        .query_row("SELECT vec_version()", [], |row| row.get(0))
        .unwrap();
    assert!(version.starts_with('v'), "sqlite-vec must be registered");
    assert_eq!(vec_table_count(&store), 2);

    let (results, candidates) = store
        .nodes_by_vector_similarity_with_stats(&[1.0, 0.0], &VectorScope::default(), 2)
        .unwrap();

    assert_eq!(candidates, 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].node.qualified_name, "src/a.rs::fn::x");
    assert!((results[0].score - 1.0).abs() < 1e-5);
}

#[test]
fn vec0_index_rebuilds_when_table_is_missing() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);

    store
        .conn
        .execute_batch("DROP TABLE retrieval_vec_2")
        .unwrap();
    let (results, _) = store
        .nodes_by_vector_similarity_with_stats(&[1.0, 0.0], &VectorScope::default(), 1)
        .unwrap();

    assert_eq!(results[0].node.qualified_name, "src/a.rs::fn::x");
    assert_eq!(vec_table_count(&store), 2);
}

#[test]
fn single_repo_scope_uses_vec0_partition() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);

    store
        .conn
        .execute_batch("DROP TABLE retrieval_vec_2")
        .unwrap();
    let (results, candidates) = store
        .nodes_by_vector_similarity_with_stats(
            &[1.0, 0.0],
            &VectorScope {
                repo_ids: vec!["repo_a".to_string()],
                ..Default::default()
            },
            2,
        )
        .unwrap();

    assert_eq!(candidates, 2);
    assert_eq!(results[0].node.qualified_name, "src/a.rs::fn::x");
    assert_eq!(vec_table_count(&store), 2);
}

#[test]
fn subpath_scope_uses_vec0_rowid_filter() {
    let mut store = open_in_memory();
    seed_scoped_vectors(&mut store);
    poison_blob(&store, "src/a.rs::fn::x");

    let (results, candidates) = store
        .nodes_by_vector_similarity_with_stats(
            &[1.0, 0.0],
            &VectorScope {
                subpath_prefix: Some("src".to_string()),
                repo_ids: vec!["repo_a".to_string()],
                ..Default::default()
            },
            10,
        )
        .unwrap();

    assert_eq!(candidates, 3);
    assert!((score_of(&results, "src/a.rs::fn::x") - 1.0).abs() < 1e-5);
}

#[test]
fn kind_scope_uses_vec0_rowid_filter() {
    let mut store = open_in_memory();
    seed_scoped_vectors(&mut store);
    poison_blob(&store, "src/a.rs::fn::x");

    let (results, candidates) = store
        .nodes_by_vector_similarity_with_stats(
            &[1.0, 0.0],
            &VectorScope {
                repo_ids: vec!["repo_a".to_string()],
                kinds: vec!["function".to_string()],
                ..Default::default()
            },
            10,
        )
        .unwrap();

    assert_eq!(candidates, 2);
    assert!((score_of(&results, "src/a.rs::fn::x") - 1.0).abs() < 1e-5);
}

#[test]
fn language_scope_uses_vec0_rowid_filter() {
    let mut store = open_in_memory();
    seed_scoped_vectors(&mut store);
    poison_blob(&store, "src/nested/b.py::class::z");

    let (results, candidates) = store
        .nodes_by_vector_similarity_with_stats(
            &[1.0, 0.0],
            &VectorScope {
                repo_ids: vec!["repo_a".to_string()],
                languages: vec!["python".to_string()],
                ..Default::default()
            },
            10,
        )
        .unwrap();

    assert_eq!(candidates, 1);
    // [1,0] vs [0.9,0.1] -> ~0.9939; zero-BLOB scoring would give 0.0.
    assert!(score_of(&results, "src/nested/b.py::class::z") > 0.9);
}

#[test]
fn multi_repo_scope_uses_vec0_rowid_filter() {
    let mut store = open_in_memory();
    seed_scoped_vectors(&mut store);
    poison_blob(&store, "lib/w.rs::fn::w");

    let (results, candidates) = store
        .nodes_by_vector_similarity_with_stats(
            &[1.0, 0.0],
            &VectorScope {
                repo_ids: vec!["repo_a".to_string(), "repo_b".to_string()],
                ..Default::default()
            },
            10,
        )
        .unwrap();

    assert_eq!(candidates, 4);
    assert!((score_of(&results, "lib/w.rs::fn::w") - 1.0).abs() < 1e-5);
}

#[test]
fn knn_rowid_filter_restricts_results() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);
    let y_id = chunk_id(&store, "src/a.rs::fn::y");

    let (hits, candidates) =
        crate::vec_index::try_knn(&store.conn, &[1.0, 0.0], 10, None, Some(&[y_id])).unwrap();

    assert_eq!(candidates, 1);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "src/a.rs::fn::y");
    assert!(hits[0].1.abs() < 1e-5);
}

#[test]
fn knn_rowid_filter_empty_set_returns_no_hits() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);

    let (hits, candidates) =
        crate::vec_index::try_knn(&store.conn, &[1.0, 0.0], 10, None, Some(&[])).unwrap();

    assert_eq!(candidates, 0);
    assert!(hits.is_empty());
}

#[test]
fn knn_rowid_batches_merge_exact_top_k() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);
    let ids = [
        chunk_id(&store, "src/a.rs::fn::x"),
        chunk_id(&store, "src/a.rs::fn::y"),
    ];

    // Batch size 1 forces one KNN statement per id; the merged top-1 must still
    // be the globally nearest vector.
    let (hits, candidates) =
        crate::vec_index::try_knn_batched(&store.conn, &[1.0, 0.0], 1, None, Some(&ids), 1)
            .unwrap();

    assert_eq!(candidates, 2);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "src/a.rs::fn::x");
    assert!((hits[0].1 - 1.0).abs() < 1e-5);
}

#[test]
fn deleting_chunks_removes_vec0_rows() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);
    assert_eq!(vec_table_count(&store), 2);

    store.delete_chunks_for_file("repo_a", "src/a.rs").unwrap();
    assert_eq!(vec_table_count(&store), 0);
}
