//! Semantic retrieval: graph-aware symbol lookup, query expansion, concept
//! clustering, and cross-file linking (Phase CM9).
//!
//! These functions sit on top of the lexical FTS layer and use graph
//! relationships to find conceptually related artifacts that keyword search
//! alone would miss.

use std::collections::HashMap;

use atlas_core::{Node, Result, ScoredNode, SearchQuery};
use atlas_store_sqlite::Store;
use tracing::debug;

use crate::{apply_ranking_boosts, build_fts_query, maybe_exclude_file_nodes, merge_scored_nodes};

// ---------------------------------------------------------------------------
// Symbol neighborhood
// ---------------------------------------------------------------------------

/// All symbol-level relationships for a single `qualified_name`.
///
/// Built from directed edge queries so callers can distinguish the role of
/// each related symbol (caller vs callee vs test vs sibling).
#[derive(Debug, Clone, Default)]
pub struct SymbolNeighborhood {
    /// Nodes that call this symbol (inbound `calls` edges).
    pub callers: Vec<Node>,
    /// Nodes called by this symbol (outbound `calls` edges).
    pub callees: Vec<Node>,
    /// Test nodes linked via `tests` / `tested_by` edges.
    pub tests: Vec<Node>,
    /// Other nodes that share the same parent in the same file.
    pub siblings: Vec<Node>,
    /// Nodes linked via `imports` edges (either direction).
    pub import_neighbors: Vec<Node>,
}

/// Populate a [`SymbolNeighborhood`] for the given `qname`.
///
/// Each sub-list is bounded by `per_kind_limit` to keep the result set
/// manageable without truncating any single category completely.
pub fn symbol_neighborhood(
    store: &Store,
    qname: &str,
    per_kind_limit: usize,
) -> Result<SymbolNeighborhood> {
    let callers = store
        .direct_callers(qname, per_kind_limit)?
        .into_iter()
        .map(|(n, _)| n)
        .collect();

    let callees = store
        .direct_callees(qname, per_kind_limit)?
        .into_iter()
        .map(|(n, _)| n)
        .collect();

    let tests = store
        .test_neighbors(qname, per_kind_limit)?
        .into_iter()
        .map(|(n, _)| n)
        .collect();

    let siblings = store.containment_siblings(qname, per_kind_limit)?;

    let import_neighbors = store
        .import_neighbors(qname, per_kind_limit)?
        .into_iter()
        .map(|(n, _)| n)
        .collect();

    Ok(SymbolNeighborhood {
        callers,
        callees,
        tests,
        siblings,
        import_neighbors,
    })
}

// ---------------------------------------------------------------------------
// Graph-based query expansion
// ---------------------------------------------------------------------------

/// Derive extra FTS query terms from the graph neighbours of `seeds`.
///
/// For each seed node, the one-hop neighbours are fetched via
/// [`Store::nodes_connected_to`] and their `name` components are extracted.
/// The resulting terms supplement the original query so that conceptually
/// related symbols surface even when the query text does not mention them.
///
/// Returns up to `max_terms` distinct lowercase name tokens, excluding the
/// names already present in `seeds`.
pub fn graph_query_expansion(
    store: &Store,
    seeds: &[ScoredNode],
    max_terms: usize,
) -> Result<Vec<String>> {
    if seeds.is_empty() || max_terms == 0 {
        return Ok(vec![]);
    }

    let seed_qns: Vec<&str> = seeds
        .iter()
        .map(|s| s.node.qualified_name.as_str())
        .collect();

    let seed_names: std::collections::HashSet<String> =
        seeds.iter().map(|s| s.node.name.to_lowercase()).collect();

    let neighbors = store.nodes_connected_to(&seed_qns)?;

    debug!(
        seeds = seed_qns.len(),
        neighbors = neighbors.len(),
        "graph_query_expansion"
    );

    let mut terms: Vec<String> = neighbors
        .into_iter()
        .map(|n| n.name.to_lowercase())
        .filter(|name| !seed_names.contains(name.as_str()))
        .collect();

    terms.sort();
    terms.dedup();
    terms.truncate(max_terms);
    Ok(terms)
}

/// Run an FTS search and then expand the query with graph-neighbour terms,
/// returning a merged, re-ranked result set.
///
/// Workflow:
/// 1. Run the caller-supplied `query` to get initial FTS seeds.
/// 2. Extract neighbour names via [`graph_query_expansion`].
/// 3. Run a second FTS pass with the expanded terms.
/// 4. Merge both result sets (keeping the highest score per node).
/// 5. Apply ranking boosts and return, bounded by `query.limit`.
pub fn expanded_search(store: &Store, query: &SearchQuery) -> Result<Vec<ScoredNode>> {
    // Phase 1: initial FTS.
    let fts_q = SearchQuery {
        text: build_fts_query(&query.text),
        ..query.clone()
    };
    let initial = store.search(&fts_q)?;

    // Phase 2: graph expansion terms.
    let extra_terms = graph_query_expansion(store, &initial, 8)?;
    if extra_terms.is_empty() {
        debug!("no graph expansion terms; returning initial results");
        let boosted = apply_ranking_boosts(
            initial,
            &query.text,
            query.reference_file.as_deref(),
            query.reference_language.as_deref(),
            query.fuzzy_match,
            &Default::default(),
            &query.changed_files.iter().cloned().collect(),
        );
        return Ok(maybe_exclude_file_nodes(
            boosted,
            query.include_files,
            query.limit,
        ));
    }

    // Phase 3: second FTS pass with expanded query.
    let expanded_text = format!(
        "{} OR {}",
        build_fts_query(&query.text),
        extra_terms.join(" OR ")
    );
    debug!(expanded_text = %expanded_text, "graph-expanded FTS query");

    let expanded_q = SearchQuery {
        text: expanded_text,
        limit: query.limit.saturating_mul(2).max(40),
        ..query.clone()
    };
    let expanded = store.search(&expanded_q)?;

    // Phase 4: merge, boost, truncate.
    let merged = merge_scored_nodes(initial, expanded);
    let changed_set: std::collections::HashSet<String> =
        query.changed_files.iter().cloned().collect();
    let boosted = apply_ranking_boosts(
        merged,
        &query.text,
        query.reference_file.as_deref(),
        query.reference_language.as_deref(),
        query.fuzzy_match,
        &Default::default(),
        &changed_set,
    );
    Ok(maybe_exclude_file_nodes(
        boosted,
        query.include_files,
        query.limit,
    ))
}

// ---------------------------------------------------------------------------
// Cross-file semantic links
// ---------------------------------------------------------------------------

/// A directed semantic link from one file to another via shared symbols.
#[derive(Debug, Clone)]
pub struct SemanticLink {
    /// File that defines the referenced symbols.
    pub from_file: String,
    /// File that references those symbols.
    pub to_file: String,
    /// Qualified names of the symbols shared between the two files.
    pub via_symbols: Vec<String>,
    /// Reference count (higher = stronger coupling).
    pub strength: f64,
}

/// Find files that reference symbols defined in `file_path`.
///
/// Returns up to `limit` [`SemanticLink`]s ordered by strength (reference
/// count) descending.  An internal edge budget of `max_edges` (default 500)
/// prevents runaway queries on densely-connected files.
pub fn cross_file_links(store: &Store, file_path: &str, limit: usize) -> Result<Vec<SemanticLink>> {
    const EDGE_BUDGET: usize = 500;

    let pairs = store.files_referencing_symbols_in(file_path, EDGE_BUDGET)?;

    // Group by referencing file.
    let mut file_map: HashMap<String, Vec<String>> = HashMap::new();
    for (referencing_file, symbol_qn) in pairs {
        file_map
            .entry(referencing_file)
            .or_default()
            .push(symbol_qn);
    }

    let mut links: Vec<SemanticLink> = file_map
        .into_iter()
        .map(|(to_file, mut symbols)| {
            symbols.sort();
            symbols.dedup();
            let strength = symbols.len() as f64;
            SemanticLink {
                from_file: file_path.to_string(),
                to_file,
                via_symbols: symbols,
                strength,
            }
        })
        .collect();

    links.sort_by(|a, b| {
        b.strength
            .partial_cmp(&a.strength)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    links.truncate(limit);
    Ok(links)
}

// ---------------------------------------------------------------------------
// Concept clustering
// ---------------------------------------------------------------------------

/// A cluster of files that share common imported/called/referenced symbols.
#[derive(Debug, Clone)]
pub struct FileConcept {
    /// Files that share `shared_symbols`.
    pub files: Vec<String>,
    /// Qualified names of the symbols shared among all files in the cluster.
    pub shared_symbols: Vec<String>,
    /// `shared_symbols.len() / total_unique_targets` for the seed file.
    /// Ranges from 0.0 to 1.0; higher means tighter coupling.
    pub density: f64,
}

/// Cluster files related to each seed in `seed_files` by shared references.
///
/// For every seed file, finds other files that import/call/reference at least
/// one of the same targets using [`Store::files_sharing_references_with`].
/// Files appearing across multiple seeds are merged into one concept.
///
/// Returns up to `limit` [`FileConcept`]s ordered by density × size.
pub fn cluster_by_shared_symbols(
    store: &Store,
    seed_files: &[&str],
    limit: usize,
) -> Result<Vec<FileConcept>> {
    const EDGE_BUDGET: usize = 500;

    // Map: file_path → set of shared symbol qnames.
    let mut file_to_symbols: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    let mut total_targets_per_seed: HashMap<&str, usize> = HashMap::new();

    for &seed in seed_files {
        let pairs = store.files_sharing_references_with(seed, EDGE_BUDGET)?;

        // Count the distinct targets from the seed side for density.
        let seed_targets: std::collections::HashSet<String> =
            pairs.iter().map(|(_, sym)| sym.clone()).collect();
        total_targets_per_seed.insert(seed, seed_targets.len().max(1));

        for (co_file, sym) in pairs {
            if !seed_files.contains(&co_file.as_str()) {
                file_to_symbols.entry(co_file).or_default().insert(sym);
            }
        }
    }

    // Build concept records.
    let total_targets = total_targets_per_seed.values().sum::<usize>().max(1);

    let mut concepts: Vec<FileConcept> = file_to_symbols
        .into_iter()
        .map(|(file, symbol_set)| {
            let shared_count = symbol_set.len();
            let mut shared_symbols: Vec<String> = symbol_set.into_iter().collect();
            shared_symbols.sort();
            let density = shared_count as f64 / total_targets as f64;
            FileConcept {
                files: vec![file],
                shared_symbols,
                density,
            }
        })
        .collect();

    // Sort by density × shared count then truncate.
    concepts.sort_by(|a, b| {
        let score_a = a.density * a.shared_symbols.len() as f64;
        let score_b = b.density * b.shared_symbols.len() as f64;
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    concepts.truncate(limit);
    Ok(concepts)
}

// ---------------------------------------------------------------------------
// Context-based query expansion
// ---------------------------------------------------------------------------

/// Search using prior-context files and symbols to boost results.
///
/// `context_files` and `context_symbols` come from the caller's session
/// history (e.g. recently accessed files or symbols from prior events).
/// They are used to:
/// - add them to `changed_files` boosting so nearby results rank higher,
/// - expand the graph around seed hits to include context-adjacent nodes.
///
/// Falls back to a plain FTS search when both context lists are empty.
pub fn context_boosted_search(
    store: &Store,
    query: &SearchQuery,
    context_files: &[String],
    context_symbols: &[String],
) -> Result<Vec<ScoredNode>> {
    // Merge context_files into the changed_files boost set.
    let mut effective_cf: Vec<String> = query.changed_files.clone();
    for f in context_files {
        if !effective_cf.contains(f) {
            effective_cf.push(f.clone());
        }
    }

    // Build an augmented query that includes the context in its boosts.
    let effective_query = SearchQuery {
        changed_files: effective_cf,
        graph_expand: true,
        ..query.clone()
    };

    // Run base FTS + graph expansion (phase 1 seeds).
    let fts_text = build_fts_query(&effective_query.text);
    let fts_q = SearchQuery {
        text: fts_text,
        ..effective_query.clone()
    };
    let mut fts_seeds = store.search(&fts_q)?;

    // If caller supplied known context symbols, fetch them as synthetic seeds.
    if !context_symbols.is_empty() {
        let ctx_refs: Vec<&str> = context_symbols.iter().map(String::as_str).collect();
        let ctx_nodes = store.nodes_by_qualified_names(&ctx_refs)?;
        for n in ctx_nodes {
            let qn = n.qualified_name.clone();
            // Score context nodes lower than FTS hits so they don't dominate.
            if !fts_seeds.iter().any(|s| s.node.qualified_name == qn) {
                fts_seeds.push(ScoredNode {
                    node: n,
                    score: 1.0,
                });
            }
        }
    }

    // Apply ranking boosts including context_files as the changed-file set.
    let changed_set: std::collections::HashSet<String> =
        effective_query.changed_files.iter().cloned().collect();
    let recent_set: std::collections::HashSet<String> = if effective_query.recent_file_boost {
        store.recently_indexed_files(50)?.into_iter().collect()
    } else {
        Default::default()
    };
    let boosted = apply_ranking_boosts(
        fts_seeds,
        &query.text,
        query.reference_file.as_deref(),
        query.reference_language.as_deref(),
        query.fuzzy_match,
        &recent_set,
        &changed_set,
    );

    // Graph-expand final seeds.
    if effective_query.graph_expand && !boosted.is_empty() {
        let expanded =
            crate::graph_expand(store, boosted, effective_query.graph_max_hops, query.limit)?;
        Ok(maybe_exclude_file_nodes(
            expanded,
            query.include_files,
            query.limit,
        ))
    } else {
        Ok(maybe_exclude_file_nodes(
            boosted,
            query.include_files,
            query.limit,
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_neighborhood_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = Store::open(db.to_str().unwrap()).unwrap();
        let nbhd = symbol_neighborhood(&store, "pkg::nonexistent", 10).unwrap();
        assert!(nbhd.callers.is_empty());
        assert!(nbhd.callees.is_empty());
        assert!(nbhd.tests.is_empty());
        assert!(nbhd.siblings.is_empty());
        assert!(nbhd.import_neighbors.is_empty());
    }

    #[test]
    fn graph_query_expansion_empty_seeds() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = Store::open(db.to_str().unwrap()).unwrap();
        let terms = graph_query_expansion(&store, &[], 10).unwrap();
        assert!(terms.is_empty());
    }

    #[test]
    fn cross_file_links_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = Store::open(db.to_str().unwrap()).unwrap();
        let links = cross_file_links(&store, "src/lib.rs", 10).unwrap();
        assert!(links.is_empty());
    }

    #[test]
    fn cluster_by_shared_symbols_empty() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = Store::open(db.to_str().unwrap()).unwrap();
        let concepts = cluster_by_shared_symbols(&store, &[], 10).unwrap();
        assert!(concepts.is_empty());
    }

    #[test]
    fn context_boosted_search_empty_context() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = Store::open(db.to_str().unwrap()).unwrap();
        let q = SearchQuery {
            text: "search".to_string(),
            ..Default::default()
        };
        let results = context_boosted_search(&store, &q, &[], &[]).unwrap();
        assert!(results.is_empty());
    }
}
