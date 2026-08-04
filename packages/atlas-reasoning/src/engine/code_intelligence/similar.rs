use super::*;

impl<'s> InsightsEngine<'s> {
    pub fn find_similar_functions(
        &self,
        repo_root: impl AsRef<Path>,
        request: SimilarFunctionRequest,
    ) -> Result<SimilarFunctionAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other(
                "similar-function analysis requires a store-backed insights engine".to_owned(),
            )
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let rust_complexity = load_rust_complexity(repo_root.as_ref(), &snapshot.nodes)?;
        let node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);
        let module_by_qname = node_metrics
            .iter()
            .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
            .collect::<HashMap<_, _>>();
        let fingerprints = callable_fingerprints(repo_root.as_ref(), &snapshot, &module_by_qname)?;
        let thresholds = similarity_thresholds(self.config());
        let min_score = request.min_score.unwrap_or(thresholds.low);
        let limit = request.limit.unwrap_or(self.config().max_findings);

        let Some(source) = resolve_callable_source(&request.symbol, &fingerprints) else {
            let source = unresolved_symbol_summary(&request.symbol);
            let report = self.pattern_report(vec![InsightFinding {
                id: format!("similar_function:unresolved:{}", request.symbol),
                title: format!("unresolved callable {}", request.symbol),
                severity: InsightSeverity::Low,
                category: "similar_functions".to_owned(),
                message: format!(
                    "could not resolve callable symbol `{}` in current graph",
                    request.symbol
                ),
                evidence: Vec::new(),
                ranking_reason: "callable symbol did not resolve in current graph snapshot"
                    .to_owned(),
                details: Some(json!({ "symbol": request.symbol })),
                score: 0.0,
            }]);
            return Ok(SimilarFunctionAnalysis {
                request,
                source,
                thresholds,
                report,
                matches: Vec::new(),
            });
        };

        let matches = rank_similar_function_matches(
            source,
            &fingerprints,
            min_score,
            limit,
            request.include_same_file,
            &thresholds,
        );
        let findings = matches
            .iter()
            .map(similar_match_to_finding)
            .collect::<Vec<_>>();
        let report = self.pattern_report(findings);
        let retained_ids = retained_finding_ids(&report);
        let matches = matches
            .into_iter()
            .filter(|item| retained_ids.contains(&similar_match_id(item)))
            .collect::<Vec<_>>();
        let source = source.summary.clone();

        Ok(SimilarFunctionAnalysis {
            request,
            source,
            thresholds,
            report,
            matches,
        })
    }
}

pub(super) fn resolve_callable_source<'a>(
    symbol: &str,
    fingerprints: &'a [CallableFingerprint],
) -> Option<&'a CallableFingerprint> {
    let normalized = super::helpers::normalize_qn_kind_tokens(symbol);
    fingerprints.iter().find(|item| {
        item.summary.qualified_name == normalized || item.summary.display_name == symbol
    })
}

pub(super) fn unresolved_symbol_summary(symbol: &str) -> InsightSymbolSummary {
    InsightSymbolSummary {
        qualified_name: symbol.to_owned(),
        display_name: symbol.rsplit("::").next().unwrap_or(symbol).to_owned(),
        file_path: String::new(),
        line_start: 0,
        line_end: 0,
        language: String::new(),
        node_kind: "function".to_owned(),
        module_id: String::new(),
    }
}

pub(super) fn rank_similar_function_matches(
    source: &CallableFingerprint,
    fingerprints: &[CallableFingerprint],
    min_score: f64,
    limit: usize,
    include_same_file: bool,
    thresholds: &SimilarityThresholds,
) -> Vec<SimilarFunctionMatch> {
    let mut candidates = fingerprints
        .iter()
        .filter(|candidate| candidate.summary.qualified_name != source.summary.qualified_name)
        .filter(|candidate| candidate.summary.language == source.summary.language)
        .filter(|candidate| candidate.summary.node_kind == source.summary.node_kind)
        .filter(|candidate| {
            include_same_file || candidate.summary.file_path != source.summary.file_path
        })
        .filter(|candidate| source.arity.abs_diff(candidate.arity) <= 1)
        .take(MAX_SIMILAR_CANDIDATES_PER_SOURCE)
        .filter_map(|candidate| build_similar_match(source, candidate, min_score, thresholds))
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.candidate.file_path.cmp(&right.candidate.file_path))
            .then_with(|| left.candidate.line_start.cmp(&right.candidate.line_start))
            .then_with(|| {
                left.candidate
                    .qualified_name
                    .cmp(&right.candidate.qualified_name)
            })
    });
    if candidates.len() > limit {
        candidates.truncate(limit);
    }
    candidates
}

pub(super) fn build_similar_match(
    source: &CallableFingerprint,
    candidate: &CallableFingerprint,
    min_score: f64,
    thresholds: &SimilarityThresholds,
) -> Option<SimilarFunctionMatch> {
    let name_overlap = jaccard(&source.name_tokens, &candidate.name_tokens);
    let signature_overlap = jaccard(&source.signature_tokens, &candidate.signature_tokens);
    let body_overlap = jaccard(&source.body_shingles, &candidate.body_shingles).max(jaccard(
        &source.duplicate_shingles,
        &candidate.duplicate_shingles,
    ));
    let neighbor_overlap = jaccard(&source.neighbor_names, &candidate.neighbor_names);
    let module_overlap = if source.summary.module_id == candidate.summary.module_id {
        1.0
    } else {
        0.0
    };
    let size_overlap = overlap_ratio(source.loc, candidate.loc);

    let score = (name_overlap * 0.18)
        + (signature_overlap * 0.20)
        + (body_overlap * 0.34)
        + (neighbor_overlap * 0.18)
        + (module_overlap * 0.05)
        + (size_overlap * 0.05);
    if score < min_score {
        return None;
    }
    if body_overlap < 0.15 && neighbor_overlap < 0.15 && signature_overlap < 0.30 {
        return None;
    }

    let feature_scores = BTreeMap::from([
        ("body_overlap".to_owned(), body_overlap),
        ("module_overlap".to_owned(), module_overlap),
        ("name_overlap".to_owned(), name_overlap),
        ("neighbor_overlap".to_owned(), neighbor_overlap),
        ("signature_overlap".to_owned(), signature_overlap),
        ("size_overlap".to_owned(), size_overlap),
    ]);
    let mut matched_features = Vec::new();
    let mut differing_features = Vec::new();
    for (name, value) in &feature_scores {
        if *value >= 0.5 {
            matched_features.push(name.clone());
        } else if *value <= 0.15 {
            differing_features.push(name.clone());
        }
    }

    Some(SimilarFunctionMatch {
        source: source.summary.clone(),
        candidate: candidate.summary.clone(),
        score,
        score_band: similarity_band(score, thresholds).to_owned(),
        matched_features,
        differing_features,
        feature_scores,
    })
}
