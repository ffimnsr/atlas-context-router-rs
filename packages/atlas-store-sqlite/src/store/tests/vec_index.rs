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
fn scoped_vector_query_falls_back_to_blob_scan() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);

    let (results, candidates) = store
        .nodes_by_vector_similarity_with_stats(
            &[1.0, 0.0],
            &VectorScope {
                subpath_prefix: Some("src".to_string()),
                repo_ids: vec!["repo_a".to_string()],
                ..Default::default()
            },
            2,
        )
        .unwrap();

    assert_eq!(candidates, 2);
    assert_eq!(results[0].node.qualified_name, "src/a.rs::fn::x");
}

#[test]
fn deleting_chunks_removes_vec0_rows() {
    let mut store = open_in_memory();
    seed_vectors(&mut store);
    assert_eq!(vec_table_count(&store), 2);

    store.delete_chunks_for_file("repo_a", "src/a.rs").unwrap();
    assert_eq!(vec_table_count(&store), 0);
}
