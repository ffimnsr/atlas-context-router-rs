use super::*;

#[test]
fn explain_query_reports_fts_mode_when_hybrid_backend_missing() {
    let query = SearchQuery {
        text: "helper".to_string(),
        hybrid: true,
        ..SearchQuery::default()
    };

    let explanation = explain_query_with_embedding(None, false, &query, false, None);

    assert_eq!(explanation.active_query_mode, "fts5");
    assert_eq!(explanation.search_path, "fts5");
    assert!(
        explanation
            .warnings
            .iter()
            .any(|warning| warning.contains("falls back to FTS-only ranking"))
    );
    assert!(
        !explanation
            .ranking_factors
            .iter()
            .any(|factor| factor == "vector_rrf_merge")
    );
}

#[test]
fn explain_query_reports_hybrid_mode_when_backend_available() {
    let query = SearchQuery {
        text: "helper".to_string(),
        hybrid: true,
        ..SearchQuery::default()
    };
    let embed_cfg =
        embed::EmbeddingConfig::new("http://localhost:11434", "nomic-embed-text", 30, 3, 500);

    let explanation = explain_query_with_embedding(None, false, &query, false, Some(&embed_cfg));

    assert_eq!(explanation.active_query_mode, "fts5_vector_hybrid");
    assert!(
        explanation
            .ranking_factors
            .iter()
            .any(|factor| factor == "vector_rrf_merge")
    );
}
