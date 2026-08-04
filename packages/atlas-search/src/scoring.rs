use super::*;

pub(super) fn scoring_base_mode(result: &ScoredNode) -> RetrievalMode {
    result
        .ranking_evidence
        .as_ref()
        .map(|e| e.base_mode.clone())
        .unwrap_or(RetrievalMode::Fts5)
}

pub(super) fn ensure_ranking_evidence(result: &mut ScoredNode) -> &mut RankingEvidence {
    let base_mode = scoring_base_mode(result);
    result.ranking_evidence.get_or_insert_with(|| {
        RankingEvidence::new(base_mode, result.score).with_raw_score(result.score)
    })
}

pub(super) fn annotate_graph_seed(
    result: &mut ScoredNode,
    hop_distance: u32,
    seed_qualified_name: String,
) {
    let evidence = ensure_ranking_evidence(result);
    evidence.graph_expansion = Some(GraphExpansionEvidence {
        hop_distance,
        seed_qualified_name: Some(seed_qualified_name),
    });
    result.sync_ranking_score();
}

pub(super) fn merge_result_evidence(preferred: &mut ScoredNode, discarded: &ScoredNode) {
    match (&mut preferred.ranking_evidence, &discarded.ranking_evidence) {
        (Some(preferred_evidence), Some(discarded_evidence)) => {
            preferred_evidence.merge_from(discarded_evidence);
        }
        (None, Some(discarded_evidence)) => {
            preferred.ranking_evidence = Some(discarded_evidence.clone());
        }
        _ => {}
    }
    preferred.sync_ranking_score();
}

pub(crate) fn maybe_exclude_file_nodes(
    mut results: Vec<ScoredNode>,
    include_files: bool,
    limit: usize,
) -> Vec<ScoredNode> {
    if !include_files {
        results.retain(|result| result.node.kind != NodeKind::File);
    }
    results.truncate(limit);
    results
}

// ---------------------------------------------------------------------------
// Post-FTS ranking boosts
// ---------------------------------------------------------------------------

/// Apply heuristic score boosts on top of the raw BM25 scores returned by the
/// FTS5 query.
///
/// Priorities (highest first):
///   1. Exact `name` match           (+20)
///   2. `name` prefix match           (+5)
///   3. Exact `qualified_name` match (+15)
///   4. Fuzzy `name` match (opt-in)  (+4, only when no exact/prefix already)
///   5. Public / exported symbol      (+2)
///   6. High-value kinds: fn/method   (+3), class/struct/trait (+2), enum (+1)
///   7. Same directory as `reference_file` (+3)
///   8. Same language as `reference_language` (+2)
///   9. Recent-file boost (opt-in)   (+4)
///  10. Changed-file boost            (+5)
pub fn apply_ranking_boosts(
    mut results: Vec<ScoredNode>,
    query: &str,
    reference_file: Option<&str>,
    reference_language: Option<&str>,
    fuzzy_match: bool,
    recent_files: &HashSet<String>,
    changed_files: &HashSet<String>,
) -> Vec<ScoredNode> {
    let primitives = GraphSearchRankingPrimitives::default();
    let q_lower = query.trim().to_lowercase();
    let fuzzy_cap = fuzzy_threshold(q_lower.chars().count());

    // Pre-compute the directory of the reference file (everything before the
    // last `/`).  An empty reference dir means the root, and every root-level
    // file would match — that is intentional and consistent.
    let ref_dir: Option<String> = reference_file.map(|f| match f.rfind('/') {
        Some(idx) => f[..idx].to_string(),
        None => String::new(),
    });

    let ref_lang: Option<String> = reference_language.map(|l| l.to_lowercase());

    for r in &mut results {
        let raw_score = r.score;
        let name_lower = r.node.name.to_lowercase();
        let qualified_name_lower = r.node.qualified_name.to_lowercase();
        let file_path = r.node.file_path.clone();
        let language_lower = r.node.language.to_lowercase();
        let name = r.node.name.clone();
        let kind = r.node.kind;
        let public_exported = r
            .node
            .modifiers
            .as_ref()
            .map(|mods| {
                let mods = mods.to_lowercase();
                mods.contains("pub") || mods.contains("public") || mods.contains("export")
            })
            .unwrap_or(false);

        // Exact name match
        let exact_name_match = name_lower == q_lower;
        let prefix_match = !exact_name_match && name_lower.starts_with(&q_lower);
        let exact_or_prefix = exact_name_match || prefix_match;

        // Exact qualified_name match
        let exact_qualified_name_match = qualified_name_lower == q_lower;

        // Fuzzy name match — only when no exact/prefix hit already and the
        // query is long enough to have a non-zero threshold.
        let fuzzy = if fuzzy_match && !exact_or_prefix {
            fuzzy_typo_details(&r.node, &q_lower, fuzzy_cap, &primitives)
        } else {
            None
        };

        // Kind boost
        let kind_boost = primitives.kind_boost(kind);

        // Same-directory boost
        let same_directory = ref_dir.as_ref().is_some_and(|rdir| {
            let node_dir = match file_path.rfind('/') {
                Some(idx) => &file_path[..idx],
                None => "",
            };
            node_dir == rdir.as_str()
        });

        // Same-language boost
        let same_language = ref_lang
            .as_ref()
            .is_some_and(|rlang| language_lower == *rlang);

        // Recent-file boost: reward nodes in recently indexed files.
        let recent_file_match = !recent_files.is_empty() && recent_files.contains(&file_path);

        // Changed-file boost: reward nodes in files that are part of the
        // current diff, making them rise above unrelated matches.
        let changed_file_match = !changed_files.is_empty() && changed_files.contains(&file_path);

        let mut score_delta = 0.0;
        if exact_name_match {
            score_delta += primitives.exact_name_boost;
        }
        if prefix_match {
            score_delta += primitives.prefix_name_boost;
        }
        if exact_qualified_name_match {
            score_delta += primitives.exact_qualified_name_boost;
        }
        if let Some((_, bonus)) = fuzzy {
            score_delta += bonus;
        }
        score_delta += kind_boost;
        if public_exported {
            score_delta += primitives.public_api_boost;
        }
        if same_directory {
            score_delta += primitives.same_directory_boost;
        }
        if same_language {
            score_delta += primitives.same_language_boost;
        }
        if recent_file_match {
            score_delta += primitives.recent_file_boost;
        }
        if changed_file_match {
            score_delta += primitives.changed_file_boost;
        }

        r.score += score_delta;

        let evidence = ensure_ranking_evidence(r);
        if evidence.raw_score.is_none() {
            evidence.raw_score = Some(raw_score);
        }
        if exact_name_match {
            evidence.exact_name_match = true;
            evidence.add_matched_field(SearchMatchedField::Name);
        }
        if prefix_match {
            evidence.prefix_match = true;
            evidence.add_matched_field(SearchMatchedField::Name);
        }
        if exact_qualified_name_match {
            evidence.exact_qualified_name_match = true;
            evidence.add_matched_field(SearchMatchedField::QualifiedName);
        }
        if let Some((distance, _)) = fuzzy {
            evidence.fuzzy = Some(atlas_core::FuzzyCorrectionEvidence {
                corrected_term: Some(name),
                edit_distance: Some(distance as u8),
                fuzzy_threshold: Some(fuzzy_cap as u8),
            });
            evidence.add_matched_field(SearchMatchedField::Name);
        }
        if kind_boost > 0.0 {
            evidence.kind_boost = Some(kind_boost);
        }
        if public_exported {
            evidence.public_exported_boost = Some(primitives.public_api_boost);
        }
        if same_directory {
            evidence.same_directory_boost = Some(primitives.same_directory_boost);
        }
        if same_language {
            evidence.same_language_boost = Some(primitives.same_language_boost);
        }
        if recent_file_match {
            evidence.recent_file_boost = Some(primitives.recent_file_boost);
        }
        if changed_file_match {
            evidence.changed_file_boost = Some(primitives.changed_file_boost);
        }

        r.sync_ranking_score();
    }

    sort_scored_nodes(&mut results);
    results
}

pub(super) fn exact_symbol_hits(store: &Store, query: &SearchQuery) -> Result<Vec<ScoredNode>> {
    let trimmed = query.text.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    let mut merged: HashMap<String, ScoredNode> = HashMap::new();

    if let Some(node) = store.node_by_qname(trimmed)?
        && (query.include_files || node.kind != NodeKind::File)
        && matches_subpath(&node.file_path, query.subpath.as_deref())
    {
        merged.insert(
            node.qualified_name.clone(),
            ScoredNode::with_ranking_evidence(node, 100.0, {
                let mut evidence = RankingEvidence::new(RetrievalMode::Fts5, 100.0)
                    .with_matched_field(SearchMatchedField::QualifiedName);
                evidence.exact_qualified_name_match = true;
                evidence
            }),
        );
    }

    if !trimmed.chars().any(char::is_whitespace) {
        for node in store
            .nodes_by_name(trimmed, query.limit.max(25))?
            .into_iter()
            .filter(|node| query.include_files || node.kind != NodeKind::File)
            .filter(|node| matches_subpath(&node.file_path, query.subpath.as_deref()))
        {
            let qn = node.qualified_name.clone();
            let score = if merged.contains_key(&qn) {
                100.0
            } else {
                80.0
            };
            let scored = ScoredNode::with_ranking_evidence(node, score, {
                let mut evidence = RankingEvidence::new(RetrievalMode::Fts5, score)
                    .with_matched_field(SearchMatchedField::Name);
                evidence.exact_name_match = true;
                evidence
            });
            match merged.get_mut(&qn) {
                Some(existing) => merge_result_evidence(existing, &scored),
                None => {
                    merged.insert(qn, scored);
                }
            }
        }
    }

    Ok(merged.into_values().collect())
}

pub(super) fn merge_scored_nodes(
    primary: Vec<ScoredNode>,
    secondary: Vec<ScoredNode>,
) -> Vec<ScoredNode> {
    let mut merged: HashMap<String, ScoredNode> = HashMap::new();

    for result in primary.into_iter().chain(secondary) {
        let qn = result.node.qualified_name.clone();
        match merged.get_mut(&qn) {
            Some(existing) if result.score > existing.score => {
                let mut replacement = result;
                merge_result_evidence(&mut replacement, existing);
                *existing = replacement;
            }
            Some(existing) => {
                merge_result_evidence(existing, &result);
            }
            None => {
                merged.insert(qn, result);
            }
        }
    }

    merged.into_values().collect()
}
