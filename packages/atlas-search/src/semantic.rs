//! Semantic retrieval: graph-aware symbol lookup, query expansion, concept
//! clustering, and cross-file linking (Phase CM9).
//!
//! These functions sit on top of the lexical FTS layer and use graph
//! relationships to find conceptually related artifacts that keyword search
//! alone would miss.

use std::collections::{BTreeSet, HashMap};

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
    /// Uniform resolution confidence tier across contributing edges.
    ///
    /// `None` when the link mixes tiers or the contributing edges carry no
    /// tier.
    pub confidence_tier: Option<String>,
    /// Uniform persisted reachability provenance across contributing edges.
    pub reachability: Option<String>,
}

/// Per-edge evidence retained while grouping links by referencing file.
struct LinkEvidence {
    target_qn: String,
    confidence_tier: Option<String>,
    reachability: Option<String>,
}

/// Find files that reference symbols defined in `file_path`.
///
/// Returns up to `limit` [`SemanticLink`]s ordered by strength (reference
/// count) descending.  An internal edge budget of `max_edges` (default 500)
/// prevents runaway queries on densely-connected files.
///
/// Weak `same_package` fallback edges are excluded by default; use
/// [`cross_file_links_with_options`] with `include_weak = true` to restore
/// them.
pub fn cross_file_links(store: &Store, file_path: &str, limit: usize) -> Result<Vec<SemanticLink>> {
    cross_file_links_with_options(store, file_path, limit, false)
}

/// Find files that reference symbols defined in `file_path`, with control over
/// weak `same_package` fallback edges.
///
/// Edges whose `confidence_tier` is `same_package` are filtered individually
/// before grouping unless `include_weak` is true. Every other tier is always
/// retained.
pub fn cross_file_links_with_options(
    store: &Store,
    file_path: &str,
    limit: usize,
    include_weak: bool,
) -> Result<Vec<SemanticLink>> {
    const EDGE_BUDGET: usize = 500;

    let edges = store.semantic_reference_edges(file_path, EDGE_BUDGET)?;

    // Group by referencing file, retaining per-edge provenance.
    let mut file_map: HashMap<String, Vec<LinkEvidence>> = HashMap::new();
    for edge in edges {
        if !include_weak && edge.confidence_tier.as_deref() == Some("same_package") {
            continue;
        }
        let reachability = edge
            .extra_json
            .get("reachability")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        file_map
            .entry(edge.source_file)
            .or_default()
            .push(LinkEvidence {
                target_qn: edge.target_qn,
                confidence_tier: edge.confidence_tier,
                reachability,
            });
    }

    let mut links: Vec<SemanticLink> = file_map
        .into_iter()
        .map(|(to_file, entries)| {
            let mut symbols: Vec<String> = entries
                .iter()
                .map(|entry| entry.target_qn.clone())
                .collect();
            symbols.sort();
            symbols.dedup();
            let strength = symbols.len() as f64;

            let tiers: BTreeSet<Option<String>> = entries
                .iter()
                .map(|entry| entry.confidence_tier.clone())
                .collect();
            let confidence_tier = if tiers.len() == 1 {
                tiers.into_iter().next().flatten()
            } else {
                None
            };

            let reachability: BTreeSet<Option<String>> = entries
                .iter()
                .map(|entry| entry.reachability.clone())
                .collect();
            let reachability = if reachability.len() == 1 {
                reachability.into_iter().next().flatten()
            } else {
                None
            };

            SemanticLink {
                from_file: file_path.to_string(),
                to_file,
                via_symbols: symbols,
                strength,
                confidence_tier,
                reachability,
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
    // Map: (seed_a, seed_b) → shared symbols; keeps seed↔seed coupling visible
    // instead of dropping the other seed from the result set.
    let mut seed_pair_symbols: std::collections::BTreeMap<
        (String, String),
        std::collections::HashSet<String>,
    > = std::collections::BTreeMap::new();
    let mut total_targets_per_seed: HashMap<&str, usize> = HashMap::new();

    for &seed in seed_files {
        let pairs = store.files_sharing_references_with(seed, EDGE_BUDGET)?;

        // Count the distinct targets from the seed side for density.
        let seed_targets: std::collections::HashSet<String> =
            pairs.iter().map(|(_, sym)| sym.clone()).collect();
        total_targets_per_seed.insert(seed, seed_targets.len().max(1));

        for (co_file, sym) in pairs {
            if co_file == seed {
                continue;
            }
            if seed_files.contains(&co_file.as_str()) {
                let key = if seed < co_file.as_str() {
                    (seed.to_owned(), co_file)
                } else {
                    (co_file, seed.to_owned())
                };
                seed_pair_symbols.entry(key).or_default().insert(sym);
            } else {
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

    // Seed↔seed coupling: emit one merged concept per seed pair so multi-seed
    // queries surface cross-seed relationships instead of hiding them.
    for ((left, right), symbol_set) in seed_pair_symbols {
        if symbol_set.is_empty() {
            continue;
        }
        let shared_count = symbol_set.len();
        let mut shared_symbols: Vec<String> = symbol_set.into_iter().collect();
        shared_symbols.sort();
        let density = shared_count as f64 / total_targets as f64;
        concepts.push(FileConcept {
            files: vec![left, right],
            shared_symbols,
            density,
        });
    }

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
                fts_seeds.push(ScoredNode::new(n, 1.0));
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

    const TEST_REPO_ID: &str = "cross-file-links-repo";

    fn test_node(file_path: &str, qualified_name: &str) -> Node {
        Node {
            id: atlas_core::NodeId::UNSET,
            kind: atlas_core::NodeKind::Function,
            name: qualified_name
                .rsplit("::")
                .next()
                .unwrap_or("fn")
                .to_owned(),
            qualified_name: qualified_name.to_owned(),
            file_path: file_path.to_owned(),
            line_start: 1,
            line_end: 2,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("()".to_owned()),
            return_type: Some("()".to_owned()),
            modifiers: None,
            is_test: false,
            file_hash: format!("hash-{file_path}"),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    fn test_edge(
        file_path: &str,
        source_qn: &str,
        target_qn: &str,
        tier: &str,
        reachability: &str,
    ) -> atlas_core::Edge {
        atlas_core::Edge {
            id: 0,
            kind: atlas_core::EdgeKind::Calls,
            source_qn: source_qn.to_owned(),
            target_qn: target_qn.to_owned(),
            file_path: file_path.to_owned(),
            line: Some(1),
            confidence: 0.65,
            confidence_tier: Some(tier.to_owned()),
            extra_json: serde_json::json!({ "reachability": reachability }),
            repo_provenance: None,
        }
    }

    fn seed_cross_file_links_store() -> Store {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();

        let target = "src/lib.rs::fn::shared";
        store
            .replace_file_graph_for_repo(
                TEST_REPO_ID,
                "src/lib.rs",
                "hash-lib",
                Some("rust"),
                None,
                &[test_node("src/lib.rs", target)],
                &[],
            )
            .unwrap();

        let strong_qn = "src/strong.rs::fn::strong_caller";
        store
            .replace_file_graph_for_repo(
                TEST_REPO_ID,
                "src/strong.rs",
                "hash-strong",
                Some("rust"),
                None,
                &[test_node("src/strong.rs", strong_qn)],
                &[test_edge(
                    "src/strong.rs",
                    strong_qn,
                    target,
                    "imports",
                    "verified",
                )],
            )
            .unwrap();

        let weak_qn = "src/weak.rs::fn::weak_caller";
        store
            .replace_file_graph_for_repo(
                TEST_REPO_ID,
                "src/weak.rs",
                "hash-weak",
                Some("rust"),
                None,
                &[test_node("src/weak.rs", weak_qn)],
                &[test_edge(
                    "src/weak.rs",
                    weak_qn,
                    target,
                    "same_package",
                    "unknown",
                )],
            )
            .unwrap();
        store
    }

    #[test]
    fn cross_file_links_excludes_weak_edges_by_default() {
        let store = seed_cross_file_links_store();

        let links = cross_file_links(&store, "src/lib.rs", 10).unwrap();
        assert_eq!(links.len(), 1, "weak edge must be filtered: {links:?}");
        assert_eq!(links[0].to_file, "src/strong.rs");
        assert_eq!(links[0].confidence_tier.as_deref(), Some("imports"));
        assert_eq!(links[0].reachability.as_deref(), Some("verified"));
    }

    #[test]
    fn cross_file_links_include_weak_restores_previous_output() {
        let store = seed_cross_file_links_store();

        let links = cross_file_links_with_options(&store, "src/lib.rs", 10, true).unwrap();
        assert_eq!(
            links.len(),
            2,
            "include_weak must keep weak edges: {links:?}"
        );

        let weak = links
            .iter()
            .find(|link| link.to_file == "src/weak.rs")
            .expect("weak link present");
        assert_eq!(weak.confidence_tier.as_deref(), Some("same_package"));
        assert_eq!(weak.reachability.as_deref(), Some("unknown"));
    }

    #[test]
    fn cross_file_links_mixed_tiers_report_no_uniform_provenance() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();

        let target = "src/lib.rs::fn::shared";
        store
            .replace_file_graph_for_repo(
                TEST_REPO_ID,
                "src/lib.rs",
                "hash-lib",
                Some("rust"),
                None,
                &[test_node("src/lib.rs", target)],
                &[],
            )
            .unwrap();

        let caller = "src/mixed.rs::fn::caller";
        store
            .replace_file_graph_for_repo(
                TEST_REPO_ID,
                "src/mixed.rs",
                "hash-mixed",
                Some("rust"),
                None,
                &[test_node("src/mixed.rs", caller)],
                &[
                    test_edge("src/mixed.rs", caller, target, "imports", "verified"),
                    test_edge("src/mixed.rs", caller, target, "same_package", "unknown"),
                ],
            )
            .unwrap();

        let links = cross_file_links_with_options(&store, "src/lib.rs", 10, true).unwrap();
        assert_eq!(links.len(), 1, "same file groups into one link");
        assert_eq!(links[0].confidence_tier, None);
        assert_eq!(links[0].reachability, None);
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
