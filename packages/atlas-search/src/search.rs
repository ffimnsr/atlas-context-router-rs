use super::*;

// ---------------------------------------------------------------------------
// Top-level search entry point
// ---------------------------------------------------------------------------

/// Full enhanced search: FTS5 + ranking boosts + optional graph expansion.
///
/// This is the primary search entry point for callers that want all
/// Slice 15 features. Raw `Store::search` is still available for cases
/// where only basic FTS is needed.
///
/// When `query.hybrid` is `true` **and** an embedding backend config is
/// provided, the hybrid path is taken: FTS and vector results are merged via
/// Reciprocal Rank Fusion. Falls back silently to FTS-only when no embedding
/// backend is configured.
pub fn search(store: &Store, query: &SearchQuery) -> Result<Vec<ScoredNode>> {
    search_with_embedding(store, query, None)
}

pub fn search_with_embedding(
    store: &Store,
    query: &SearchQuery,
    embed_cfg: Option<&embed::EmbeddingConfig>,
) -> Result<Vec<ScoredNode>> {
    // ---- hybrid path -------------------------------------------------------
    if query.hybrid {
        if let Some(embed_cfg) = embed_cfg {
            return search_hybrid(store, query, embed_cfg);
        }
        debug!("hybrid=true but search.embedding.url not set; falling back to FTS");
    }

    // ---- FTS path ----------------------------------------------------------
    let exact_hits = exact_symbol_hits(store, query)?;

    // Build an FTS query that includes camelCase/snake_case token variants.
    let expanded_text = build_fts_query(&query.text);
    let effective_query = SearchQuery {
        text: expanded_text,
        ..query.clone()
    };

    let mut fts_results = store.search(&effective_query)?;

    if query.fuzzy_match {
        let relaxed_text = build_relaxed_fts_query(&query.text);
        if !relaxed_text.is_empty() {
            let relaxed_query = SearchQuery {
                text: relaxed_text,
                limit: query.limit.saturating_mul(5).max(25),
                ..query.clone()
            };
            let relaxed_results = store.search(&relaxed_query)?;
            let fuzzy_cap = fuzzy_threshold(query.text.trim().chars().count());
            let relaxed_results: Vec<_> = relaxed_results
                .into_iter()
                .filter_map(|mut result| {
                    if fuzzy_cap == 0 {
                        return None;
                    }
                    let distance = edit_distance(
                        &query.text.trim().to_lowercase(),
                        &result.node.name.to_lowercase(),
                        fuzzy_cap,
                    );
                    if distance > fuzzy_cap {
                        return None;
                    }
                    let corrected_term = result.node.name.clone();
                    let evidence = ensure_ranking_evidence(&mut result);
                    evidence.fuzzy = Some(atlas_core::FuzzyCorrectionEvidence {
                        corrected_term: Some(corrected_term),
                        edit_distance: Some(distance as u8),
                        fuzzy_threshold: Some(fuzzy_cap as u8),
                    });
                    evidence.add_matched_field(SearchMatchedField::Name);
                    result.sync_ranking_score();
                    Some(result)
                })
                .collect();
            fts_results = merge_scored_nodes(fts_results, relaxed_results);
        }
    }

    // Optionally fetch recently indexed file paths for the recent-file boost.
    let recent_set: HashSet<String> = if query.recent_file_boost {
        // Top-50 recent files is enough signal without being expensive.
        store.recently_indexed_files(50)?.into_iter().collect()
    } else {
        HashSet::new()
    };

    // Build the changed-file set from the caller-supplied paths.
    let changed_set: HashSet<String> = query.changed_files.iter().cloned().collect();

    // Apply post-FTS ranking boosts using the original (un-expanded) text so
    // boost comparisons are made against what the user actually typed.
    let boosted = apply_ranking_boosts(
        merge_scored_nodes(exact_hits, fts_results),
        &query.text,
        query.reference_file.as_deref(),
        query.reference_language.as_deref(),
        query.fuzzy_match,
        &recent_set,
        &changed_set,
    );

    if query.graph_expand && !boosted.is_empty() {
        let limit = query.limit;
        let expanded = graph_expand(store, boosted, query.graph_max_hops, limit)?;
        Ok(maybe_exclude_file_nodes(
            expanded,
            query.include_files,
            limit,
        ))
    } else {
        Ok(maybe_exclude_file_nodes(
            boosted,
            query.include_files,
            query.limit,
        ))
    }
}
