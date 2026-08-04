use super::*;

// ---------------------------------------------------------------------------
// Hybrid search internals
// ---------------------------------------------------------------------------

/// Merge two ranked lists using Reciprocal Rank Fusion (RRF).
///
/// RRF score for document `d` = Σ 1 / (k + rank(d, retriever)).
/// `k` (typically 60) dampens the influence of absolute rank position.
/// Both lists may be empty; an empty list contributes nothing to the scores.
pub fn reciprocal_rank_fusion(
    fts: &[ScoredNode],
    vector: &[ScoredNode],
    k: u32,
) -> Vec<ScoredNode> {
    let mut acc: HashMap<String, (ScoredNode, f64, Vec<HybridRankContribution>)> = HashMap::new();

    for (rank, n) in fts.iter().enumerate() {
        let contribution = 1.0 / (k as f64 + rank as f64 + 1.0);
        let entry = acc
            .entry(n.node.qualified_name.clone())
            .or_insert_with(|| (n.clone(), 0.0, Vec::new()));
        entry.1 += contribution;
        merge_result_evidence(&mut entry.0, n);
        entry.2.push(HybridRankContribution {
            source: HybridRankingSource::Fts5,
            rank: rank as u32 + 1,
            score_contribution: contribution,
        });
    }
    for (rank, n) in vector.iter().enumerate() {
        let contribution = 1.0 / (k as f64 + rank as f64 + 1.0);
        let entry = acc
            .entry(n.node.qualified_name.clone())
            .or_insert_with(|| (n.clone(), 0.0, Vec::new()));
        entry.1 += contribution;
        merge_result_evidence(&mut entry.0, n);
        entry.2.push(HybridRankContribution {
            source: HybridRankingSource::Vector,
            rank: rank as u32 + 1,
            score_contribution: contribution,
        });
    }

    let mut results: Vec<ScoredNode> = acc
        .into_values()
        .map(|(mut n, score, sources)| {
            n.score = score;
            let mut ranking_evidence = n
                .ranking_evidence
                .clone()
                .unwrap_or_else(|| RankingEvidence::new(RetrievalMode::Hybrid, score));
            ranking_evidence.base_mode = RetrievalMode::Hybrid;
            if ranking_evidence.raw_score.is_none() {
                ranking_evidence.raw_score = Some(score);
            }
            ranking_evidence.final_score = score;
            ranking_evidence.hybrid_rrf = Some(HybridRrfEvidence { sources });
            n.ranking_evidence = Some(ranking_evidence);
            n
        })
        .collect();
    sort_scored_nodes(&mut results);
    results
}

/// Run FTS + vector retrieval and merge with RRF.
pub(super) fn search_hybrid(
    store: &Store,
    query: &SearchQuery,
    embed_cfg: &embed::EmbeddingConfig,
) -> Result<Vec<ScoredNode>> {
    // FTS branch — fetch top_k_fts candidates then apply ranking boosts.
    let fts_q = SearchQuery {
        text: build_fts_query(&query.text),
        limit: query.top_k_fts,
        ..query.clone()
    };
    let fts_raw = store.search(&fts_q)?;

    let recent_set: HashSet<String> = if query.recent_file_boost {
        store.recently_indexed_files(50)?.into_iter().collect()
    } else {
        HashSet::new()
    };
    let changed_set: HashSet<String> = query.changed_files.iter().cloned().collect();

    let fts_boosted = apply_ranking_boosts(
        fts_raw,
        &query.text,
        query.reference_file.as_deref(),
        query.reference_language.as_deref(),
        query.fuzzy_match,
        &recent_set,
        &changed_set,
    );

    // Vector branch — embed query and fetch top_k_vector candidates.
    let query_vec = embed::embed_text_blocking(embed_cfg, &query.text)
        .map_err(|e| atlas_core::AtlasError::Other(e.to_string()))?;
    let vector_results = maybe_exclude_file_nodes(
        store.nodes_by_vector_similarity(&query_vec, query.top_k_vector)?,
        query.include_files,
        query.top_k_vector,
    );

    // RRF merge and truncate to requested limit.
    let merged = reciprocal_rank_fusion(&fts_boosted, &vector_results, query.rrf_k);
    Ok(maybe_exclude_file_nodes(
        merged,
        query.include_files,
        query.limit,
    ))
}
