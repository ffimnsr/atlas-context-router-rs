use std::collections::HashMap;

use atlas_core::{NodeKind, Result, ScoredNode, SearchQuery};
use atlas_store_sqlite::Store;
use tracing::debug;

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
///   4. Public / exported symbol      (+2)
///   5. High-value kinds: fn/method   (+3), class/struct/trait (+2), enum (+1)
///   6. Same directory as `reference_file` (+3)
///   7. Same language as `reference_language` (+2)
pub fn apply_ranking_boosts(
    mut results: Vec<ScoredNode>,
    query: &str,
    reference_file: Option<&str>,
    reference_language: Option<&str>,
) -> Vec<ScoredNode> {
    let q_lower = query.trim().to_lowercase();

    // Pre-compute the directory of the reference file (everything before the
    // last `/`).  An empty reference dir means the root, and every root-level
    // file would match — that is intentional and consistent.
    let ref_dir: Option<String> = reference_file.map(|f| {
        match f.rfind('/') {
            Some(idx) => f[..idx].to_string(),
            None => String::new(),
        }
    });

    let ref_lang: Option<String> = reference_language.map(|l| l.to_lowercase());

    for r in &mut results {
        let n = &r.node;

        // Exact name match
        if n.name.to_lowercase() == q_lower {
            r.score += 20.0;
        } else if n.name.to_lowercase().starts_with(&q_lower) {
            r.score += 5.0;
        }

        // Exact qualified_name match
        if n.qualified_name.to_lowercase() == q_lower {
            r.score += 15.0;
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
    }

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
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
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    Ok(results)
}

// ---------------------------------------------------------------------------
// Top-level search entry point
// ---------------------------------------------------------------------------

/// Full enhanced search: FTS5 + ranking boosts + optional graph expansion.
///
/// This is the primary search entry point for callers that want all
/// Slice 15 features. Raw `Store::search` is still available for cases
/// where only basic FTS is needed.
pub fn search(store: &Store, query: &SearchQuery) -> Result<Vec<ScoredNode>> {
    // Build an FTS query that includes camelCase/snake_case token variants.
    let expanded_text = build_fts_query(&query.text);
    let effective_query = SearchQuery {
        text: expanded_text,
        ..query.clone()
    };

    let fts_results = store.search(&effective_query)?;

    // Apply post-FTS ranking boosts using the original (un-expanded) text so
    // boost comparisons are made against what the user actually typed.
    let boosted = apply_ranking_boosts(
        fts_results,
        &query.text,
        query.reference_file.as_deref(),
        query.reference_language.as_deref(),
    );

    if query.graph_expand && !boosted.is_empty() {
        let limit = query.limit;
        graph_expand(store, boosted, query.graph_max_hops, limit)
    } else {
        Ok(boosted)
    }
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
        let boosted = apply_ranking_boosts(input, "search", None, None);

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
        let boosted = apply_ranking_boosts(input, "foo", Some("src/main.rs"), None);

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
        let boosted = apply_ranking_boosts(input, "parse", None, Some("rust"));

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
        let best =
            make_test_node("helper", "src/a.rs::fn::helper", "src/a.rs", "rust");
        let dir_only =
            make_test_node("helper", "src/b.go::fn::helper", "src/b.go", "go");
        let neither =
            make_test_node("helper", "lib/c.py::fn::helper", "lib/c.py", "python");

        let input = vec![neither.clone(), dir_only.clone(), best.clone()];
        let boosted =
            apply_ranking_boosts(input, "helper", Some("src/main.rs"), Some("rust"));

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
        let boosted = apply_ranking_boosts(input, "f", None, None);

        // Both same language, no reference → scores should be equal (both
        // start at 1.0 with only the fn-kind +3 applied equally).
        let score_a = boosted.iter().find(|r| r.node.file_path == "src/a.rs").unwrap().score;
        let score_b = boosted.iter().find(|r| r.node.file_path == "lib/b.rs").unwrap().score;
        assert_eq!(score_a, score_b, "without reference both nodes should score equally");
    }
}
