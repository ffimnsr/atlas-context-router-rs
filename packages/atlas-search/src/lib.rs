use std::collections::{HashMap, HashSet};

use atlas_core::{NodeKind, Result, ScoredNode, SearchQuery};
use atlas_store_sqlite::Store;
use tracing::debug;

pub mod embed;

// ---------------------------------------------------------------------------
// Token splitting
// ---------------------------------------------------------------------------

/// Split a camelCase identifier into its component words.
///
/// Examples:
///   `"ReplaceFileGraph"` → `["Replace", "File", "Graph"]`
///   `"camelCase"` → `["camel", "Case"]`
fn split_camel(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        let prev_lower = i > 0 && chars[i - 1].is_lowercase();
        let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
        if ch.is_uppercase() && i > 0 && (prev_lower || next_lower) && !current.is_empty() {
            parts.push(current.clone());
            current.clear();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Build an FTS5 query string from user input, adding token variants from
/// camelCase and snake_case splitting so that "ReplaceFileGraph" also matches
/// documents containing "replace", "file", or "graph".
///
/// The original term is always preserved as the leading token to keep it
/// highest-priority for BM25.
pub fn build_fts_query(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }

    let mut tokens: Vec<String> = vec![trimmed.to_lowercase()];

    // camelCase splitting
    let camel_parts = split_camel(trimmed);
    if camel_parts.len() > 1 {
        tokens.extend(camel_parts.iter().map(|s| s.to_lowercase()));
    }

    // snake_case splitting
    let snake_parts: Vec<&str> = trimmed.split('_').filter(|s| !s.is_empty()).collect();
    if snake_parts.len() > 1 {
        tokens.extend(snake_parts.iter().map(|s| s.to_lowercase()));
    }

    // whitespace splitting (multi-word input)
    let word_parts: Vec<&str> = trimmed.split_whitespace().collect();
    if word_parts.len() > 1 {
        tokens.extend(word_parts.iter().map(|s| s.to_lowercase()));
    }

    tokens.dedup();
    tokens.join(" OR ")
}

/// Build a relaxed FTS5 query for typo-tolerant lookup.
///
/// Uses short prefix wildcards derived from the original token and any
/// camelCase / snake_case splits so a typo like `greter` can still retrieve
/// `greet_twice` candidates before fuzzy ranking runs.
fn build_relaxed_fts_query(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut prefixes: Vec<String> = Vec::new();
    let lower = trimmed.to_lowercase();
    if let Some(prefix) = relaxed_prefix(&lower) {
        prefixes.push(prefix);
    }

    let camel_parts = split_camel(trimmed);
    if camel_parts.len() > 1 {
        for part in camel_parts {
            if let Some(prefix) = relaxed_prefix(&part.to_lowercase()) {
                prefixes.push(prefix);
            }
        }
    }

    let snake_parts: Vec<&str> = trimmed.split('_').filter(|part| !part.is_empty()).collect();
    if snake_parts.len() > 1 {
        for part in snake_parts {
            if let Some(prefix) = relaxed_prefix(&part.to_lowercase()) {
                prefixes.push(prefix);
            }
        }
    }

    let word_parts: Vec<&str> = trimmed.split_whitespace().collect();
    if word_parts.len() > 1 {
        for part in word_parts {
            if let Some(prefix) = relaxed_prefix(&part.to_lowercase()) {
                prefixes.push(prefix);
            }
        }
    }

    prefixes.dedup();
    prefixes.join(" OR ")
}

fn relaxed_prefix(token: &str) -> Option<String> {
    let len = token.chars().count();
    if len < 4 {
        return None;
    }

    let prefix_len = if len >= 6 { 3 } else { 2 };
    let prefix: String = token.chars().take(prefix_len).collect();
    Some(format!("{prefix}*"))
}

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// Compute the edit distance (Levenshtein) between two strings, capped at
/// `cap + 1` so we can bail out early for clearly dissimilar strings.
fn edit_distance(a: &str, b: &str, cap: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();

    // Quick bounds check — if length difference alone exceeds cap, bail.
    if m.abs_diff(n) > cap {
        return cap + 1;
    }

    // Two-row DP (space-efficient).
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        let mut row_min = i;
        for j in 1..=n {
            curr[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1]
            } else {
                1 + prev[j - 1].min(prev[j]).min(curr[j - 1])
            };
            row_min = row_min.min(curr[j]);
        }
        // Early exit if entire row exceeds cap.
        if row_min > cap {
            return cap + 1;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Return the edit-distance threshold for a query of length `len`.
///
/// Short queries need tighter matching to avoid noise:
///   len ≤ 3 → 0 (exact only)
///   len ≤ 5 → 1
///   len ≤ 8 → 2
///   len > 8 → 3
fn fuzzy_threshold(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=5 => 1,
        6..=8 => 2,
        _ => 3,
    }
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
        let n = &r.node;
        let name_lower = n.name.to_lowercase();

        // Exact name match
        let exact_or_prefix = if name_lower == q_lower {
            r.score += 20.0;
            true
        } else if name_lower.starts_with(&q_lower) {
            r.score += 5.0;
            true
        } else {
            false
        };

        // Exact qualified_name match
        if n.qualified_name.to_lowercase() == q_lower {
            r.score += 15.0;
        }

        // Fuzzy name match — only when no exact/prefix hit already and the
        // query is long enough to have a non-zero threshold.
        if fuzzy_match && !exact_or_prefix && fuzzy_cap > 0 {
            let dist = edit_distance(&q_lower, &name_lower, fuzzy_cap);
            if dist <= fuzzy_cap {
                r.score += 4.0;
            }
        }

        // Kind boost
        match n.kind {
            NodeKind::Function | NodeKind::Method => r.score += 3.0,
            NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface => {
                r.score += 2.0
            }
            NodeKind::Enum => r.score += 1.0,
            _ => {}
        }

        // Public / exported API boost
        if let Some(mods) = &n.modifiers {
            let m = mods.to_lowercase();
            if m.contains("pub") || m.contains("public") || m.contains("export") {
                r.score += 2.0;
            }
        }

        // Same-directory boost
        if let Some(rdir) = &ref_dir {
            let node_dir = match n.file_path.rfind('/') {
                Some(idx) => &n.file_path[..idx],
                None => "",
            };
            if node_dir == rdir.as_str() {
                r.score += 3.0;
            }
        }

        // Same-language boost
        if let Some(rlang) = &ref_lang
            && n.language.to_lowercase() == *rlang
        {
            r.score += 2.0;
        }

        // Recent-file boost: reward nodes in recently indexed files.
        if !recent_files.is_empty() && recent_files.contains(&n.file_path) {
            r.score += 4.0;
        }

        // Changed-file boost: reward nodes in files that are part of the
        // current diff, making them rise above unrelated matches.
        if !changed_files.is_empty() && changed_files.contains(&n.file_path) {
            r.score += 5.0;
        }
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

// ---------------------------------------------------------------------------
// Graph-aware expansion
// ---------------------------------------------------------------------------

/// Expand a set of FTS seed results through the graph, adding neighboring
/// nodes at a distance-decayed score.
///
/// The caller's original scored seeds occupy hop-0 (distance 0). Each
/// successive hop decays the maximum seed score by `1 / (hop + 1)`.
/// Nodes already present at a shorter distance are never overwritten.
/// The combined set is truncated to `limit` after sorting by score.
pub fn graph_expand(
    store: &Store,
    seeds: Vec<ScoredNode>,
    max_hops: u32,
    limit: usize,
) -> Result<Vec<ScoredNode>> {
    // Map qualified_name → ScoredNode; seeds are inserted at their own score.
    let mut result_map: HashMap<String, ScoredNode> = HashMap::new();

    for s in &seeds {
        result_map
            .entry(s.node.qualified_name.clone())
            .or_insert_with(|| s.clone());
    }

    let max_seed_score = seeds
        .iter()
        .map(|s| s.score)
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let mut frontier: Vec<String> = seeds
        .iter()
        .map(|s| s.node.qualified_name.clone())
        .collect();

    for hop in 1..=max_hops {
        if frontier.is_empty() || result_map.len() >= limit {
            break;
        }

        let frontier_refs: Vec<&str> = frontier.iter().map(String::as_str).collect();
        let neighbors = store.nodes_connected_to(&frontier_refs)?;

        debug!(hop, neighbors = neighbors.len(), "graph expansion hop");

        let hop_score = max_seed_score / (hop as f64 + 1.0);
        let mut next_frontier = Vec::new();

        for neighbor in neighbors {
            let qn = neighbor.qualified_name.clone();
            if !result_map.contains_key(&qn) {
                result_map.insert(
                    qn.clone(),
                    ScoredNode {
                        node: neighbor,
                        score: hop_score,
                    },
                );
                next_frontier.push(qn);
            }
        }

        frontier = next_frontier;
    }

    let mut results: Vec<ScoredNode> = result_map.into_values().collect();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    Ok(results)
}

fn exact_symbol_hits(store: &Store, query: &SearchQuery) -> Result<Vec<ScoredNode>> {
    let trimmed = query.text.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    let mut merged: HashMap<String, ScoredNode> = HashMap::new();

    if let Some(node) = store.node_by_qname(trimmed)? {
        merged.insert(
            node.qualified_name.clone(),
            ScoredNode { node, score: 100.0 },
        );
    }

    if !trimmed.chars().any(char::is_whitespace) {
        for node in store.nodes_by_name(trimmed, query.limit.max(25))? {
            let qn = node.qualified_name.clone();
            let score = if merged.contains_key(&qn) { 100.0 } else { 80.0 };
            merged.entry(qn).or_insert(ScoredNode { node, score });
        }
    }

    Ok(merged.into_values().collect())
}

fn merge_scored_nodes(primary: Vec<ScoredNode>, secondary: Vec<ScoredNode>) -> Vec<ScoredNode> {
    let mut merged: HashMap<String, ScoredNode> = HashMap::new();

    for result in primary.into_iter().chain(secondary) {
        let qn = result.node.qualified_name.clone();
        match merged.get_mut(&qn) {
            Some(existing) if result.score > existing.score => *existing = result,
            Some(_) => {}
            None => {
                merged.insert(qn, result);
            }
        }
    }

    merged.into_values().collect()
}

// ---------------------------------------------------------------------------
// Top-level search entry point
// ---------------------------------------------------------------------------

/// Full enhanced search: FTS5 + ranking boosts + optional graph expansion.
///
/// This is the primary search entry point for callers that want all
/// Slice 15 features. Raw `Store::search` is still available for cases
/// where only basic FTS is needed.
///
/// When `query.hybrid` is `true` **and** `ATLAS_EMBED_URL` is set in the
/// environment, the hybrid path is taken: FTS and vector results are merged
/// via Reciprocal Rank Fusion.  Falls back silently to FTS-only when no
/// embedding backend is configured.
pub fn search(store: &Store, query: &SearchQuery) -> Result<Vec<ScoredNode>> {
    // ---- hybrid path -------------------------------------------------------
    if query.hybrid {
        if let Some(embed_cfg) = embed::EmbeddingConfig::from_env() {
            return search_hybrid(store, query, &embed_cfg);
        }
        debug!("hybrid=true but ATLAS_EMBED_URL not set; falling back to FTS");
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

    if fts_results.is_empty() && query.fuzzy_match {
        let relaxed_text = build_relaxed_fts_query(&query.text);
        if !relaxed_text.is_empty() {
            let relaxed_query = SearchQuery {
                text: relaxed_text,
                limit: query.limit.saturating_mul(5).max(25),
                ..query.clone()
            };
            let relaxed_results = store.search(&relaxed_query)?;
            let fuzzy_cap = fuzzy_threshold(query.text.trim().chars().count());
            fts_results = relaxed_results
                .into_iter()
                .filter(|result| {
                    fuzzy_cap > 0
                        && edit_distance(
                            &query.text.trim().to_lowercase(),
                            &result.node.name.to_lowercase(),
                            fuzzy_cap,
                        ) <= fuzzy_cap
                })
                .collect();
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
        graph_expand(store, boosted, query.graph_max_hops, limit)
    } else {
        Ok(boosted)
    }
}

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
    let mut acc: HashMap<String, (ScoredNode, f64)> = HashMap::new();

    for (rank, n) in fts.iter().enumerate() {
        let entry = acc
            .entry(n.node.qualified_name.clone())
            .or_insert_with(|| (n.clone(), 0.0));
        entry.1 += 1.0 / (k as f64 + rank as f64 + 1.0);
    }
    for (rank, n) in vector.iter().enumerate() {
        let entry = acc
            .entry(n.node.qualified_name.clone())
            .or_insert_with(|| (n.clone(), 0.0));
        entry.1 += 1.0 / (k as f64 + rank as f64 + 1.0);
    }

    let mut results: Vec<ScoredNode> = acc
        .into_values()
        .map(|(mut n, score)| {
            n.score = score;
            n
        })
        .collect();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

/// Run FTS + vector retrieval and merge with RRF.
fn search_hybrid(
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
    let query_vec = embed::embed_text(embed_cfg, &query.text)
        .map_err(|e| atlas_core::AtlasError::Other(e.to_string()))?;
    let vector_results = store.nodes_by_vector_similarity(&query_vec, query.top_k_vector)?;

    // RRF merge and truncate to requested limit.
    let mut merged = reciprocal_rank_fusion(&fts_boosted, &vector_results, query.rrf_k);
    merged.truncate(query.limit);
    Ok(merged)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_camel_basic() {
        let parts = split_camel("ReplaceFileGraph");
        assert_eq!(parts, vec!["Replace", "File", "Graph"]);
    }

    #[test]
    fn split_camel_all_lower() {
        let parts = split_camel("lowercase");
        assert_eq!(parts, vec!["lowercase"]);
    }

    #[test]
    fn split_camel_acronym_boundary() {
        // "HTTPClient" → ["HTTP", "Client"]
        let parts = split_camel("HTTPClient");
        assert!(parts.len() >= 2, "expected at least 2 parts, got {parts:?}");
        assert_eq!(parts.last().unwrap(), "Client");
    }

    #[test]
    fn build_fts_query_camel() {
        let q = build_fts_query("ReplaceFileGraph");
        assert!(q.contains("replace"), "should contain 'replace': {q}");
        assert!(q.contains("file"), "should contain 'file': {q}");
        assert!(q.contains("graph"), "should contain 'graph': {q}");
    }

    #[test]
    fn build_fts_query_snake() {
        let q = build_fts_query("impact_radius");
        assert!(q.contains("impact"), "should contain 'impact': {q}");
        assert!(q.contains("radius"), "should contain 'radius': {q}");
    }

    #[test]
    fn build_fts_query_plain() {
        let q = build_fts_query("simple");
        assert_eq!(q, "simple");
    }

    #[test]
    fn build_relaxed_fts_query_plain() {
        let q = build_relaxed_fts_query("greter");
        assert_eq!(q, "gre*");
    }

    #[test]
    fn build_relaxed_fts_query_snake() {
        let q = build_relaxed_fts_query("gret_twice");
        assert!(q.contains("gre*"), "expected typo prefix token: {q}");
        assert!(q.contains("tw*"), "expected stable suffix token: {q}");
    }

    #[test]
    fn apply_ranking_boosts_exact_name() {
        use atlas_core::{Node, NodeId, NodeKind};

        let node = Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: "search".to_string(),
            qualified_name: "src/lib.rs::fn::search".to_string(),
            file_path: "src/lib.rs".to_string(),
            line_start: 1,
            line_end: 10,
            language: "rust".to_string(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: Some("pub".to_string()),
            is_test: false,
            file_hash: "abc".to_string(),
            extra_json: serde_json::Value::Null,
        };

        let input = vec![ScoredNode { node, score: 5.0 }];
        let boosted = apply_ranking_boosts(
            input,
            "search",
            None,
            None,
            false,
            &HashSet::new(),
            &HashSet::new(),
        );

        // Exact name (+20) + fn kind (+3) + pub (+2) = +25 on top of 5.0
        assert!(
            boosted[0].score >= 30.0,
            "expected score >= 30, got {}",
            boosted[0].score
        );
    }

    fn make_test_node(name: &str, qn: &str, file_path: &str, language: &str) -> ScoredNode {
        use atlas_core::{Node, NodeId, NodeKind};
        ScoredNode {
            node: Node {
                id: NodeId::UNSET,
                kind: NodeKind::Function,
                name: name.to_string(),
                qualified_name: qn.to_string(),
                file_path: file_path.to_string(),
                line_start: 1,
                line_end: 10,
                language: language.to_string(),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: "h".to_string(),
                extra_json: serde_json::Value::Null,
            },
            score: 1.0,
        }
    }

    #[test]
    fn same_directory_boost_applied() {
        let same_dir = make_test_node("foo", "src/util.rs::fn::foo", "src/util.rs", "rust");
        let diff_dir = make_test_node("foo", "other/lib.rs::fn::foo", "other/lib.rs", "rust");

        let input = vec![diff_dir.clone(), same_dir.clone()];
        let boosted = apply_ranking_boosts(
            input,
            "foo",
            Some("src/main.rs"),
            None,
            false,
            &HashSet::new(),
            &HashSet::new(),
        );

        let same_score = boosted
            .iter()
            .find(|r| r.node.file_path == "src/util.rs")
            .unwrap()
            .score;
        let diff_score = boosted
            .iter()
            .find(|r| r.node.file_path == "other/lib.rs")
            .unwrap()
            .score;
        assert!(
            same_score > diff_score,
            "same-dir result should score higher; same={same_score} diff={diff_score}"
        );
    }

    #[test]
    fn same_language_boost_applied() {
        let rust_node = make_test_node("parse", "src/a.rs::fn::parse", "src/a.rs", "rust");
        let go_node = make_test_node("parse", "src/a.go::fn::parse", "src/a.go", "go");

        let input = vec![go_node.clone(), rust_node.clone()];
        let boosted = apply_ranking_boosts(
            input,
            "parse",
            None,
            Some("rust"),
            false,
            &HashSet::new(),
            &HashSet::new(),
        );

        let rust_score = boosted
            .iter()
            .find(|r| r.node.language == "rust")
            .unwrap()
            .score;
        let go_score = boosted
            .iter()
            .find(|r| r.node.language == "go")
            .unwrap()
            .score;
        assert!(
            rust_score > go_score,
            "same-language result should score higher; rust={rust_score} go={go_score}"
        );
    }

    #[test]
    fn same_dir_and_same_lang_both_applied() {
        // Node in same dir AND same language should get both boosts.
        let best = make_test_node("helper", "src/a.rs::fn::helper", "src/a.rs", "rust");
        let dir_only = make_test_node("helper", "src/b.go::fn::helper", "src/b.go", "go");
        let neither = make_test_node("helper", "lib/c.py::fn::helper", "lib/c.py", "python");

        let input = vec![neither.clone(), dir_only.clone(), best.clone()];
        let boosted = apply_ranking_boosts(
            input,
            "helper",
            Some("src/main.rs"),
            Some("rust"),
            false,
            &HashSet::new(),
            &HashSet::new(),
        );

        let best_score = boosted
            .iter()
            .find(|r| r.node.file_path == "src/a.rs")
            .unwrap()
            .score;
        let dir_only_score = boosted
            .iter()
            .find(|r| r.node.file_path == "src/b.go")
            .unwrap()
            .score;
        let neither_score = boosted
            .iter()
            .find(|r| r.node.file_path == "lib/c.py")
            .unwrap()
            .score;

        assert!(
            best_score > dir_only_score,
            "dir+lang node must beat dir-only; best={best_score} dir_only={dir_only_score}"
        );
        assert!(
            dir_only_score > neither_score,
            "dir-only node must beat neither; dir_only={dir_only_score} neither={neither_score}"
        );
    }

    #[test]
    fn no_reference_file_no_boost() {
        let n1 = make_test_node("f", "src/a.rs::fn::f", "src/a.rs", "rust");
        let n2 = make_test_node("f", "lib/b.rs::fn::f", "lib/b.rs", "rust");

        let input = vec![n1.clone(), n2.clone()];
        let boosted = apply_ranking_boosts(
            input,
            "f",
            None,
            None,
            false,
            &HashSet::new(),
            &HashSet::new(),
        );

        // Both same language, no reference → scores should be equal (both
        // start at 1.0 with only the fn-kind +3 applied equally).
        let score_a = boosted
            .iter()
            .find(|r| r.node.file_path == "src/a.rs")
            .unwrap()
            .score;
        let score_b = boosted
            .iter()
            .find(|r| r.node.file_path == "lib/b.rs")
            .unwrap()
            .score;
        assert_eq!(
            score_a, score_b,
            "without reference both nodes should score equally"
        );
    }

    #[test]
    fn edit_distance_basic() {
        assert_eq!(edit_distance("kitten", "sitting", 10), 3);
        assert_eq!(edit_distance("abc", "abc", 5), 0);
        assert_eq!(edit_distance("abc", "xyz", 10), 3);
        // Cap early-exit: length diff > cap
        assert_eq!(edit_distance("short", "muchlongerstring", 2), 3);
    }

    #[test]
    fn fuzzy_match_boost_applied() {
        // "sarch" is 1 edit away from "search" → should get fuzzy boost.
        let close = make_test_node("search", "src/lib.rs::fn::search", "src/lib.rs", "rust");
        let distant = make_test_node(
            "transform",
            "src/lib.rs::fn::transform",
            "src/lib.rs",
            "rust",
        );

        let input = vec![distant.clone(), close.clone()];
        let boosted = apply_ranking_boosts(
            input,
            "sarch",
            None,
            None,
            true,
            &HashSet::new(),
            &HashSet::new(),
        );

        let close_score = boosted
            .iter()
            .find(|r| r.node.name == "search")
            .unwrap()
            .score;
        let distant_score = boosted
            .iter()
            .find(|r| r.node.name == "transform")
            .unwrap()
            .score;
        assert!(
            close_score > distant_score,
            "fuzzy-close name should score higher; close={close_score} distant={distant_score}"
        );
    }

    #[test]
    fn fuzzy_match_off_no_boost() {
        // Same setup but fuzzy_match=false → no extra boost for "sarch".
        let close = make_test_node("search", "src/lib.rs::fn::search", "src/lib.rs", "rust");
        let input = vec![close];
        let no_fuzzy = apply_ranking_boosts(
            input.clone(),
            "sarch",
            None,
            None,
            false,
            &HashSet::new(),
            &HashSet::new(),
        );
        let with_fuzzy = apply_ranking_boosts(
            input,
            "sarch",
            None,
            None,
            true,
            &HashSet::new(),
            &HashSet::new(),
        );

        assert!(
            with_fuzzy[0].score > no_fuzzy[0].score,
            "fuzzy=true should score higher than fuzzy=false for a close mismatch"
        );
    }

    #[test]
    fn recent_file_boost_applied() {
        let recent = make_test_node(
            "do_work",
            "src/fresh.rs::fn::do_work",
            "src/fresh.rs",
            "rust",
        );
        let old = make_test_node(
            "do_work",
            "src/stale.rs::fn::do_work",
            "src/stale.rs",
            "rust",
        );

        let recent_set: HashSet<String> = ["src/fresh.rs".to_string()].into();
        let input = vec![old.clone(), recent.clone()];
        let boosted = apply_ranking_boosts(
            input,
            "do_work",
            None,
            None,
            false,
            &recent_set,
            &HashSet::new(),
        );

        let recent_score = boosted
            .iter()
            .find(|r| r.node.file_path == "src/fresh.rs")
            .unwrap()
            .score;
        let old_score = boosted
            .iter()
            .find(|r| r.node.file_path == "src/stale.rs")
            .unwrap()
            .score;
        assert!(
            recent_score > old_score,
            "recent-file node must score higher; recent={recent_score} old={old_score}"
        );
    }

    #[test]
    fn recent_file_boost_empty_set_no_effect() {
        let n = make_test_node("work", "src/a.rs::fn::work", "src/a.rs", "rust");
        let base = apply_ranking_boosts(
            vec![n.clone()],
            "work",
            None,
            None,
            false,
            &HashSet::new(),
            &HashSet::new(),
        );
        let with_empty_recent = apply_ranking_boosts(
            vec![n],
            "work",
            None,
            None,
            false,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(
            base[0].score, with_empty_recent[0].score,
            "empty recent set must not change score"
        );
    }

    #[test]
    fn changed_file_boost_applied() {
        let changed = make_test_node(
            "do_work",
            "src/changed.rs::fn::do_work",
            "src/changed.rs",
            "rust",
        );
        let unchanged = make_test_node(
            "do_work",
            "src/stable.rs::fn::do_work",
            "src/stable.rs",
            "rust",
        );

        let changed_set: HashSet<String> = ["src/changed.rs".to_string()].into();
        let input = vec![unchanged.clone(), changed.clone()];
        let boosted = apply_ranking_boosts(
            input,
            "do_work",
            None,
            None,
            false,
            &HashSet::new(),
            &changed_set,
        );

        let changed_score = boosted
            .iter()
            .find(|r| r.node.file_path == "src/changed.rs")
            .unwrap()
            .score;
        let stable_score = boosted
            .iter()
            .find(|r| r.node.file_path == "src/stable.rs")
            .unwrap()
            .score;
        assert!(
            changed_score > stable_score,
            "changed-file node must score higher; changed={changed_score} stable={stable_score}"
        );
    }
}
