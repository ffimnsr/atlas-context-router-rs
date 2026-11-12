use super::*;

fn chunk_row(store: &Store, qn: &str) -> (i64, String, Option<Vec<u8>>) {
    store
        .conn
        .query_row(
            "SELECT id, text, embedding FROM retrieval_chunks
             WHERE node_qn = ?1 AND chunk_idx = 0",
            [qn],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

#[test]
fn chunk_replacement_preserves_embedding_when_text_unchanged() {
    let mut store = open_in_memory();
    let parsed = |name: &str| ParsedFile {
        path: "a.rs".to_string(),
        language: Some("rust".to_string()),
        hash: "h1".to_string(),
        size: None,
        nodes: vec![make_node(
            NodeKind::Function,
            name,
            "a.rs::fn::x",
            "a.rs",
            "rust",
        )],
        edges: vec![],
    };

    store
        .replace_files_transactional_for_repo("repo_a", &[parsed("x")])
        .unwrap();
    store
        .replace_chunks_for_parsed_files("repo_a", &[parsed("x")])
        .unwrap();

    let (id, text, _) = chunk_row(&store, "a.rs::fn::x");
    store.set_chunk_embedding(id, &[0.25, 0.5]).unwrap();
    let expected: Vec<u8> = [0.25f32, 0.5]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();

    // Identical text must keep the stored vector, so updates only embed changes.
    store
        .replace_chunks_for_parsed_files("repo_a", &[parsed("x")])
        .unwrap();
    let (_, same_text, embedding) = chunk_row(&store, "a.rs::fn::x");
    assert_eq!(same_text, text);
    assert_eq!(embedding.as_deref(), Some(expected.as_slice()));

    // Changed chunk text must invalidate the vector so it is re-embedded.
    store
        .replace_chunks_for_parsed_files("repo_a", &[parsed("renamed")])
        .unwrap();
    let (_, changed_text, embedding) = chunk_row(&store, "a.rs::fn::x");
    assert_ne!(changed_text, text);
    assert!(embedding.is_none());
}
