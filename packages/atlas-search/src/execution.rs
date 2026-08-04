use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueryExecutionMode {
    Fts5,
    RegexStructuralScan,
    RegexStructuralScanGraphExpand,
    Fts5VectorHybrid,
    Fts5GraphExpand,
    Fts5RegexFilter,
    Fts5RegexFilterGraphExpand,
}

impl QueryExecutionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fts5 => "fts5",
            Self::RegexStructuralScan => "regex_structural_scan",
            Self::RegexStructuralScanGraphExpand => "regex_structural_scan_graph_expand",
            Self::Fts5VectorHybrid => "fts5_vector_hybrid",
            Self::Fts5GraphExpand => "fts5_graph_expand",
            Self::Fts5RegexFilter => "fts5_regex_filter",
            Self::Fts5RegexFilterGraphExpand => "fts5_regex_filter_graph_expand",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryExplainInput {
    pub text: String,
    pub kind: Option<String>,
    pub language: Option<String>,
    pub limit: usize,
    pub semantic: bool,
    pub expand: bool,
    pub expand_hops: u32,
    pub regex: Option<String>,
    pub subpath: Option<String>,
    pub fuzzy: bool,
    pub hybrid: bool,
    pub include_files: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryExplainFiltersApplied {
    pub kind: bool,
    pub language: bool,
    pub subpath: bool,
    pub fuzzy: bool,
    pub hybrid: bool,
    pub semantic: bool,
    pub expand: bool,
    pub include_files: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryExplainMatch {
    pub score: f64,
    pub kind: String,
    pub qualified_name: String,
    pub file_path: String,
    pub line_start: u32,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ranking_evidence: Option<RankingEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryExplanation {
    pub active_query_mode: String,
    pub search_path: String,
    pub input: QueryExplainInput,
    pub fts_tokens: Vec<String>,
    pub fts_phrase: Option<String>,
    pub regex_valid: bool,
    pub regex_error: Option<String>,
    pub ranking_factors: Vec<String>,
    pub filters_applied: QueryExplainFiltersApplied,
    pub indexed_node_count: Option<i64>,
    pub db_exists: bool,
    pub warnings: Vec<String>,
    pub latency_ms: Option<u128>,
    pub result_count: Option<usize>,
    pub matches: Option<Vec<QueryExplainMatch>>,
    /// Active backend capability flags at explain time.
    pub active_capabilities: BackendCapabilities,
}

pub(super) fn query_execution_mode_for(
    query: &SearchQuery,
    semantic: bool,
    hybrid_backend_available: bool,
) -> QueryExecutionMode {
    let graph_aware = semantic || query.graph_expand;
    match (
        query.text.trim().is_empty(),
        query.regex_pattern.is_some(),
        graph_aware,
        query.hybrid && hybrid_backend_available,
    ) {
        (true, true, false, _) => QueryExecutionMode::RegexStructuralScan,
        (true, true, true, _) => QueryExecutionMode::RegexStructuralScanGraphExpand,
        (false, false, _, true) => QueryExecutionMode::Fts5VectorHybrid,
        (false, false, true, false) => QueryExecutionMode::Fts5GraphExpand,
        (false, true, false, false) => QueryExecutionMode::Fts5RegexFilter,
        (false, true, true, false) => QueryExecutionMode::Fts5RegexFilterGraphExpand,
        _ => QueryExecutionMode::Fts5,
    }
}

pub(super) fn ranking_factors_for(
    query: &SearchQuery,
    semantic: bool,
    hybrid_backend_available: bool,
) -> Vec<String> {
    let mut ranking_factors = vec!["fts5_bm25".to_string()];
    if query.fuzzy_match {
        ranking_factors.push("fuzzy_edit_distance_boost".to_string());
    }
    if hybrid_backend_available {
        ranking_factors.push("vector_rrf_merge".to_string());
    }
    if semantic {
        ranking_factors.push("graph_neighbor_rerank".to_string());
    }
    if query.graph_expand {
        ranking_factors.push("graph_distance_expansion".to_string());
    }
    ranking_factors
}

pub(super) fn tokenize_fts(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        vec![]
    } else {
        text.split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|token| !token.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

pub fn execute_query(
    store: &Store,
    query: &SearchQuery,
    semantic: bool,
) -> Result<Vec<ScoredNode>> {
    execute_query_with_embedding(store, query, semantic, None)
}

pub fn execute_query_with_embedding(
    store: &Store,
    query: &SearchQuery,
    semantic: bool,
    embed_cfg: Option<&embed::EmbeddingConfig>,
) -> Result<Vec<ScoredNode>> {
    let mut results = if semantic {
        semantic::expanded_search(store, query)?
    } else {
        search_with_embedding(store, query, embed_cfg)?
    };

    if semantic && query.graph_expand && !results.is_empty() {
        results = graph_expand(store, results, query.graph_max_hops, query.limit)?;
        results = maybe_exclude_file_nodes(results, query.include_files, query.limit);
    }

    Ok(results)
}

pub fn explain_query(
    store: Option<&Store>,
    db_exists: bool,
    query: &SearchQuery,
    semantic: bool,
) -> QueryExplanation {
    explain_query_with_embedding(store, db_exists, query, semantic, None)
}

pub fn explain_query_with_embedding(
    store: Option<&Store>,
    db_exists: bool,
    query: &SearchQuery,
    semantic: bool,
    embed_cfg: Option<&embed::EmbeddingConfig>,
) -> QueryExplanation {
    let fts_tokens = tokenize_fts(&query.text);
    let fts_phrase = if fts_tokens.is_empty() {
        None
    } else if fts_tokens.len() == 1 {
        Some(format!("\"{}\"", fts_tokens[0]))
    } else {
        Some(
            fts_tokens
                .iter()
                .map(|token| format!("\"{token}\""))
                .collect::<Vec<_>>()
                .join(" "),
        )
    };

    let (regex_valid, regex_error) = if let Some(ref pattern) = query.regex_pattern {
        match regex::Regex::new(pattern) {
            Ok(_) => (true, None),
            Err(err) => (false, Some(err.to_string())),
        }
    } else {
        (true, None)
    };

    let caps = capabilities::derive_capabilities(embed_cfg);
    let hybrid_backend_available = query.hybrid && caps.hybrid_lexical_vector;
    let mode = query_execution_mode_for(query, semantic, hybrid_backend_available);
    let mut warnings: Vec<String> = Vec::new();
    if fts_tokens.len() > 1 {
        warnings.push(
            "Multi-token text is matched as implicit AND across all tokens; this often returns zero results. Prefer a single short identifier.".to_string(),
        );
    }
    if query.text.contains(' ') && query.regex_pattern.is_none() {
        warnings.push(
            "Natural-language phrases rarely match FTS5 symbol names. Use regex for pattern matching or pass a single exact identifier.".to_string(),
        );
    }
    if !regex_valid {
        warnings.push("regex pattern is invalid; the query would return an error.".to_string());
    }
    if query.hybrid && !caps.hybrid_lexical_vector {
        warnings.push(
            "hybrid retrieval requested but search.embedding.url is not configured; execution falls back to FTS-only ranking.".to_string(),
        );
    }

    let indexed_node_count =
        store.and_then(|store| store.stats().ok().map(|stats| stats.node_count));
    let (latency_ms, result_count, matches) = if regex_valid {
        if let Some(store) = store {
            let t0 = std::time::Instant::now();
            let results =
                execute_query_with_embedding(store, query, semantic, embed_cfg).unwrap_or_default();
            let latency_ms = t0.elapsed().as_millis();
            let matches: Vec<QueryExplainMatch> = results
                .iter()
                .map(|result| QueryExplainMatch {
                    score: result.score,
                    kind: result.node.kind.as_str().to_owned(),
                    qualified_name: result.node.qualified_name.clone(),
                    file_path: result.node.file_path.clone(),
                    line_start: result.node.line_start,
                    language: result.node.language.clone(),
                    ranking_evidence: result.ranking_evidence.clone(),
                })
                .collect();
            (Some(latency_ms), Some(matches.len()), Some(matches))
        } else {
            (None, None, None)
        }
    } else {
        (None, None, None)
    };

    QueryExplanation {
        active_query_mode: mode.as_str().to_owned(),
        search_path: mode.as_str().to_owned(),
        input: QueryExplainInput {
            text: query.text.clone(),
            kind: query.kind.clone(),
            language: query.language.clone(),
            limit: query.limit,
            semantic,
            expand: query.graph_expand,
            expand_hops: query.graph_max_hops,
            regex: query.regex_pattern.clone(),
            subpath: query.subpath.clone(),
            fuzzy: query.fuzzy_match,
            hybrid: query.hybrid,
            include_files: query.include_files,
        },
        fts_tokens,
        fts_phrase,
        regex_valid,
        regex_error,
        ranking_factors: ranking_factors_for(query, semantic, hybrid_backend_available),
        filters_applied: QueryExplainFiltersApplied {
            kind: query.kind.is_some(),
            language: query.language.is_some(),
            subpath: query.subpath.is_some(),
            fuzzy: query.fuzzy_match,
            hybrid: query.hybrid,
            semantic,
            expand: query.graph_expand,
            include_files: query.include_files,
        },
        indexed_node_count,
        db_exists,
        warnings,
        latency_ms,
        result_count,
        matches,
        active_capabilities: caps,
    }
}
