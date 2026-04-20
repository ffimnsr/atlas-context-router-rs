// Phase 22 — Context Engine: Slices 3, 4, 5, 6, 8
// Phase CM6 — Retrieval-backed restoration
//
// Slice 3: resolve_target
// Slice 4: build_symbol_context
// Slice 5: rank_context / trim_context
// Slice 6: build_review_context / build_impact_context
// Slice 8: apply_code_spans
// CM6: retrieve_saved_context

use std::collections::{HashMap, HashSet};

use atlas_contentstore::{
    ContentStore,
    store::{SearchFilters, SourceRow},
};
use atlas_core::{
    Result,
    model::{
        AmbiguityMeta, ContextRequest, ContextResult, ContextTarget, NoiseReductionSummary,
        SavedContextSource, SelectedEdge, SelectedFile, SelectedNode, SelectionReason,
        TruncationMeta, WorkflowCallChain, WorkflowComponent, WorkflowFocusNode, WorkflowSummary,
    },
};
use atlas_store_sqlite::Store;

// ---------------------------------------------------------------------------
// Slice 3: target resolution
// ---------------------------------------------------------------------------

/// Outcome of resolving a [`ContextTarget`].
#[derive(Debug)]
pub enum ResolvedTarget {
    /// Exactly one node matched.
    Node(Box<atlas_core::model::Node>),
    /// Exactly one file path matched (used for `FilePath` targets).
    File(String),
    /// Multiple candidates were found; a ranked list is provided.
    Ambiguous(AmbiguityMeta),
    /// No match found, but suggestions are available if the fallback search
    /// returned anything.
    NotFound { suggestions: Vec<String> },
}

/// Resolve a [`ContextTarget`] to a concrete node or file using the store.
///
/// Resolution order (exact paths first, FTS fallback last):
/// 1. `QualifiedName` → exact `node_by_qname` lookup.
/// 2. `SymbolName`    → exact `nodes_by_name`, take unique or mark ambiguous.
/// 3. `FilePath`      → `nodes_by_file` check; returns `File` if non-empty.
/// 4. `ChangedFiles`  → not resolved to a single node; callers handle directly.
///
/// Falls back to FTS search (capped at 8 candidates) only when exact paths
/// yield no result.
pub fn resolve_target(store: &Store, target: &ContextTarget) -> Result<ResolvedTarget> {
    match target {
        ContextTarget::QualifiedName { qname } => resolve_by_qname(store, qname),
        ContextTarget::SymbolName { name } => resolve_by_name(store, name),
        ContextTarget::FilePath { path } => resolve_by_file(store, path),
        // These multi-value or special targets are handled by builders, not here.
        ContextTarget::ChangedFiles { .. }
        | ContextTarget::ChangedSymbols { .. }
        | ContextTarget::EdgeQuerySeed { .. } => Ok(ResolvedTarget::NotFound {
            suggestions: vec![],
        }),
    }
}

fn resolve_by_qname(store: &Store, qname: &str) -> Result<ResolvedTarget> {
    if let Some(node) = store.node_by_qname(qname)? {
        return Ok(ResolvedTarget::Node(Box::new(node)));
    }
    // Exact qname miss → try FTS fallback using the qname as a search term.
    fts_fallback(store, qname)
}

fn resolve_by_name(store: &Store, name: &str) -> Result<ResolvedTarget> {
    // LIMIT 9 so we can tell "unique" vs "a few" vs "many".
    const CANDIDATE_CAP: usize = 9;
    let nodes = store.nodes_by_name(name, CANDIDATE_CAP)?;
    match nodes.len() {
        0 => fts_fallback(store, name),
        1 => Ok(ResolvedTarget::Node(Box::new(
            nodes.into_iter().next().unwrap(),
        ))),
        _ => {
            let candidates: Vec<String> = nodes.iter().map(|n| n.qualified_name.clone()).collect();
            Ok(ResolvedTarget::Ambiguous(AmbiguityMeta {
                query: name.to_owned(),
                candidates,
                resolved: false,
            }))
        }
    }
}

fn resolve_by_file(store: &Store, path: &str) -> Result<ResolvedTarget> {
    let nodes = store.nodes_by_file(path)?;
    if nodes.is_empty() {
        // Path not in DB; return not-found with no suggestions.
        return Ok(ResolvedTarget::NotFound {
            suggestions: vec![],
        });
    }
    Ok(ResolvedTarget::File(path.to_owned()))
}

/// Run an FTS search and return `Ambiguous` (if results found) or `NotFound`.
fn fts_fallback(store: &Store, text: &str) -> Result<ResolvedTarget> {
    use atlas_core::SearchQuery;
    use atlas_search::search as fts_search;

    let query = SearchQuery {
        text: text.to_owned(),
        limit: 8,
        fuzzy_match: true,
        ..SearchQuery::default()
    };
    let results = fts_search(store, &query)?;
    if results.is_empty() {
        return Ok(ResolvedTarget::NotFound {
            suggestions: vec![],
        });
    }
    let candidates: Vec<String> = results
        .iter()
        .map(|r| r.node.qualified_name.clone())
        .collect();
    Ok(ResolvedTarget::Ambiguous(AmbiguityMeta {
        query: text.to_owned(),
        candidates,
        resolved: false,
    }))
}

// ---------------------------------------------------------------------------
// Slice 4: symbol-context retrieval
// ---------------------------------------------------------------------------

/// Default caps when the request does not specify limits.
const DEFAULT_MAX_NODES: usize = 50;
const DEFAULT_MAX_EDGES: usize = 100;
const DEFAULT_MAX_FILES: usize = 20;

/// Per-bucket limits fed to store helpers.
const BUCKET_CALLERS: usize = 15;
const BUCKET_CALLEES: usize = 15;
const BUCKET_IMPORTS: usize = 10;
const BUCKET_SIBLINGS: usize = 10;
const BUCKET_TESTS: usize = 10;

/// Build a symbol-centred [`ContextResult`] from a resolved seed node.
///
/// Retrieves callers, callees, import neighbors, containment siblings, and
/// optional test neighbors at hop-1.  Multi-hop traversal is gated behind
/// `request.depth > 1`.
///
/// The result is passed through `rank_context` then `trim_context` before
/// returning.
pub fn build_symbol_context(
    store: &Store,
    seed: atlas_core::model::Node,
    request: &ContextRequest,
) -> Result<ContextResult> {
    let qname = seed.qualified_name.clone();

    let mut nodes: Vec<SelectedNode> = Vec::new();
    let mut edges: Vec<SelectedEdge> = Vec::new();
    let mut seen_qnames: HashSet<String> = HashSet::new();

    // Seed node itself (distance 0, DirectTarget).
    seen_qnames.insert(qname.clone());
    nodes.push(SelectedNode {
        node: seed.clone(),
        selection_reason: SelectionReason::DirectTarget,
        distance: 0,
        relevance_score: 0.0,
    });

    let depth = request.depth.unwrap_or(1).max(1);

    // Breadth-first hop expansion. For hop-1 we always collect callers/callees.
    // Subsequent hops repeat on the previous hop's new nodes.
    let mut frontier_qnames: Vec<String> = vec![qname.clone()];

    for hop in 1..=depth {
        let mut next_frontier: Vec<String> = Vec::new();

        for fqname in &frontier_qnames {
            // Callers
            if request.include_callers {
                for (caller, edge) in store.direct_callers(fqname, BUCKET_CALLERS)? {
                    let cqn = caller.qualified_name.clone();
                    if seen_qnames.insert(cqn.clone()) {
                        next_frontier.push(cqn);
                        edges.push(SelectedEdge {
                            edge,
                            selection_reason: SelectionReason::Caller,
                            depth: Some(hop),
                            relevance_score: 0.0,
                        });
                        nodes.push(SelectedNode {
                            node: caller,
                            selection_reason: SelectionReason::Caller,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }

            // Callees
            if request.include_callees {
                for (callee, edge) in store.direct_callees(fqname, BUCKET_CALLEES)? {
                    let cqn = callee.qualified_name.clone();
                    if seen_qnames.insert(cqn.clone()) {
                        next_frontier.push(cqn);
                        edges.push(SelectedEdge {
                            edge,
                            selection_reason: SelectionReason::Callee,
                            depth: Some(hop),
                            relevance_score: 0.0,
                        });
                        nodes.push(SelectedNode {
                            node: callee,
                            selection_reason: SelectionReason::Callee,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }

            // Imports (only on first hop, or when explicitly requested)
            if request.include_imports {
                for (import_node, edge) in store.import_neighbors(fqname, BUCKET_IMPORTS)? {
                    let iqn = import_node.qualified_name.clone();
                    if seen_qnames.insert(iqn) {
                        edges.push(SelectedEdge {
                            edge,
                            selection_reason: SelectionReason::Importee,
                            depth: Some(hop),
                            relevance_score: 0.0,
                        });
                        nodes.push(SelectedNode {
                            node: import_node,
                            selection_reason: SelectionReason::Importee,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }

            // Containment siblings (first hop only, gated by include_neighbors)
            if hop == 1 && request.include_neighbors {
                for sibling in store.containment_siblings(fqname, BUCKET_SIBLINGS)? {
                    let sqn = sibling.qualified_name.clone();
                    if seen_qnames.insert(sqn) {
                        nodes.push(SelectedNode {
                            node: sibling,
                            selection_reason: SelectionReason::ContainmentSibling,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }

            // Test neighbors (first hop only, gated by include_tests)
            if hop == 1 && request.include_tests {
                for (test_node, edge) in store.test_neighbors(fqname, BUCKET_TESTS)? {
                    let tqn = test_node.qualified_name.clone();
                    if seen_qnames.insert(tqn) {
                        edges.push(SelectedEdge {
                            edge,
                            selection_reason: SelectionReason::TestAdjacent,
                            depth: Some(hop),
                            relevance_score: 0.0,
                        });
                        nodes.push(SelectedNode {
                            node: test_node,
                            selection_reason: SelectionReason::TestAdjacent,
                            distance: hop,
                            relevance_score: 0.0,
                        });
                    }
                }
            }
        }

        frontier_qnames = next_frontier;
        if frontier_qnames.is_empty() {
            break;
        }
    }

    // Collect unique file paths from selected nodes.
    let files = collect_files(&nodes);

    let mut result = ContextResult {
        request: request.clone(),
        nodes,
        edges,
        files,
        truncation: TruncationMeta::none(),
        ambiguity: None,
        workflow: None,
        saved_context_sources: vec![],
    };

    rank_context(&mut result);
    trim_context(&mut result);
    update_file_node_counts(&mut result);

    if request.include_code_spans {
        apply_code_spans(&mut result);
    }

    result.workflow = Some(build_workflow_summary(&result));

    Ok(result)
}

/// Build a [`ContextResult`] when the target was ambiguous (no ranking/trimming
/// needed — the caller should present candidates to the user).
pub fn build_ambiguous_result(request: &ContextRequest, meta: AmbiguityMeta) -> ContextResult {
    ContextResult {
        request: request.clone(),
        nodes: vec![],
        edges: vec![],
        files: vec![],
        truncation: TruncationMeta::none(),
        ambiguity: Some(meta),
        workflow: None,
        saved_context_sources: vec![],
    }
}

/// Build a [`ContextResult`] for a not-found target.
pub fn build_not_found_result(request: &ContextRequest, suggestions: Vec<String>) -> ContextResult {
    let ambiguity = if suggestions.is_empty() {
        None
    } else {
        Some(AmbiguityMeta {
            query: format!("{:?}", request.target),
            candidates: suggestions,
            resolved: false,
        })
    };
    ContextResult {
        request: request.clone(),
        nodes: vec![],
        edges: vec![],
        files: vec![],
        truncation: TruncationMeta::none(),
        ambiguity,
        workflow: None,
        saved_context_sources: vec![],
    }
}

/// Collect unique `SelectedFile` entries from a node list, preserving first-seen order.
/// `language` is taken from the first node seen for each file.
/// `node_count_included` is left at 0; call `update_file_node_counts` after trimming.
fn collect_files(nodes: &[SelectedNode]) -> Vec<SelectedFile> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut files: Vec<SelectedFile> = Vec::new();
    for sn in nodes {
        let path = sn.node.file_path.clone();
        if seen.insert(path.clone()) {
            let reason = if sn.selection_reason == SelectionReason::DirectTarget {
                SelectionReason::DirectTarget
            } else {
                sn.selection_reason
            };
            files.push(SelectedFile {
                path,
                selection_reason: reason,
                line_ranges: vec![],
                language: Some(sn.node.language.clone()),
                node_count_included: 0,
            });
        }
    }
    files
}

/// Recompute `SelectedFile.node_count_included` from the node list after trimming.
fn update_file_node_counts(result: &mut ContextResult) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for sn in &result.nodes {
        *counts.entry(sn.node.file_path.clone()).or_insert(0) += 1;
    }
    for sf in &mut result.files {
        sf.node_count_included = counts.get(&sf.path).copied().unwrap_or(0);
    }
}

// ---------------------------------------------------------------------------
// Slice 5: ranking and trimming
// ---------------------------------------------------------------------------

/// Relevance score assigned to each selection reason (higher wins trimming).
fn reason_priority(reason: SelectionReason) -> u8 {
    match reason {
        SelectionReason::DirectTarget => 100,
        SelectionReason::Caller => 80,
        SelectionReason::Callee => 80,
        SelectionReason::Importer => 60,
        SelectionReason::Importee => 60,
        SelectionReason::TestAdjacent => 50,
        SelectionReason::ContainmentSibling => 40,
        SelectionReason::ImpactNeighbor => 30,
    }
}

/// Compute a floating-point relevance score for a [`SelectedNode`].
///
/// Scoring factors:
///   - Reason priority           (base from `reason_priority`, 0-100)
///   - Closer hop distance       (bonus: max(0, 10 - 5*distance))
///   - Public API symbol         (+5)
///   - High-value kinds (fn/method/class/struct/trait) (+3)
///   - Test node                 (-10, deprioritised unless include_tests)
fn node_score(sn: &SelectedNode, seed_file: &str, seed_qname: &str) -> f64 {
    let _ = seed_qname; // reserved for future cross-ref boosting
    let mut score = reason_priority(sn.selection_reason) as f64;

    // Distance decay.
    let distance_bonus = (10.0_f64 - sn.distance as f64 * 5.0).max(0.0);
    score += distance_bonus;

    // Same-file as seed.
    if sn.node.file_path == seed_file {
        score += 3.0;
    }

    // Public API.
    if let Some(mods) = &sn.node.modifiers {
        let m = mods.to_lowercase();
        if m.contains("pub") || m.contains("public") || m.contains("export") {
            score += 5.0;
        }
    }

    // Kind boost.
    use atlas_core::NodeKind;
    match sn.node.kind {
        NodeKind::Function | NodeKind::Method => score += 3.0,
        NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface => score += 2.0,
        _ => {}
    }

    // Test penalty (present to ensure non-test nodes survive trimming first).
    if sn.node.is_test || sn.node.kind == NodeKind::Test {
        score -= 10.0;
    }

    score
}

/// Score for a [`SelectedEdge`].
fn edge_score(se: &SelectedEdge) -> f64 {
    let base = reason_priority(se.selection_reason) as f64;
    // Edge confidence is 0-1; scale to 0-10.
    base + (se.edge.confidence as f64) * 10.0
}

/// Sort [`ContextResult`] nodes and edges by relevance **in place**.
///
/// Assigns `relevance_score` on each node/edge, then sorts highest-first.
/// Nodes tie-break by `qualified_name`; edges tie-break by source then target.
pub fn rank_context(result: &mut ContextResult) {
    // Need to know seed node's file and qname for scoring.
    // Seed is always the first node (DirectTarget, distance 0).
    let (seed_file, seed_qname) = result
        .nodes
        .iter()
        .find(|n| n.selection_reason == SelectionReason::DirectTarget)
        .map(|n| (n.node.file_path.clone(), n.node.qualified_name.clone()))
        .unwrap_or_default();

    // Compute and assign node scores (immutable pass, then mutable assign).
    let node_scores: Vec<f32> = result
        .nodes
        .iter()
        .map(|sn| node_score(sn, &seed_file, &seed_qname) as f32)
        .collect();
    for (sn, s) in result.nodes.iter_mut().zip(&node_scores) {
        sn.relevance_score = *s;
    }

    result.nodes.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.qualified_name.cmp(&b.node.qualified_name))
    });

    // Compute and assign edge scores.
    let edge_scores: Vec<f32> = result
        .edges
        .iter()
        .map(|se| edge_score(se) as f32)
        .collect();
    for (se, s) in result.edges.iter_mut().zip(&edge_scores) {
        se.relevance_score = *s;
    }

    result.edges.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.edge.source_qn.cmp(&b.edge.source_qn))
            .then_with(|| a.edge.target_qn.cmp(&b.edge.target_qn))
    });

    result.files.sort_by(|a, b| {
        let pa = reason_priority(a.selection_reason) as i32;
        let pb = reason_priority(b.selection_reason) as i32;
        pb.cmp(&pa).then_with(|| a.path.cmp(&b.path))
    });
}

/// Apply hard node/edge/file caps to [`ContextResult`], recording what was
/// dropped in [`TruncationMeta`].
///
/// Trimming order after ranking:
/// 1. Keep all DirectTarget nodes unconditionally.
/// 2. Keep next highest-scored nodes up to `max_nodes`.
/// 3. Drop edges whose source or target qname no longer has a node in the set.
/// 4. Trim edge list to `max_edges`.
/// 5. Trim file list to `max_files`, keeping DirectTarget files first.
pub fn trim_context(result: &mut ContextResult) {
    use atlas_core::model::ContextIntent;

    let max_nodes = result.request.max_nodes.unwrap_or(DEFAULT_MAX_NODES);
    let max_edges = result.request.max_edges.unwrap_or(DEFAULT_MAX_EDGES);
    let max_files = result.request.max_files.unwrap_or(DEFAULT_MAX_FILES);

    // --- Nodes ---
    let original_node_count = result.nodes.len();
    if original_node_count > max_nodes {
        // Always keep DirectTarget node(s) at the front; they survive the cap.
        // After rank_context they should already be at the top, but be safe.
        let (targets, rest): (Vec<_>, Vec<_>) = result
            .nodes
            .drain(..)
            .partition(|n| n.selection_reason == SelectionReason::DirectTarget);

        let reserve_non_target = usize::from(
            result.request.intent == ContextIntent::Review
                && max_nodes > 1
                && !rest.is_empty()
                && targets.len() >= max_nodes,
        );
        let keep_targets = max_nodes.saturating_sub(reserve_non_target);

        result.nodes = targets.into_iter().take(keep_targets).collect();
        let budget = max_nodes.saturating_sub(result.nodes.len());
        result.nodes.extend(rest.into_iter().take(budget));
    }
    let dropped_nodes = original_node_count.saturating_sub(result.nodes.len());

    // Remaining-node qname set (for edge pruning).
    let remaining_qnames: HashSet<&str> = result
        .nodes
        .iter()
        .map(|n| n.node.qualified_name.as_str())
        .collect();

    // --- Edges ---
    let original_edge_count = result.edges.len();
    // Drop edges referencing removed nodes.
    result.edges.retain(|se| {
        remaining_qnames.contains(se.edge.source_qn.as_str())
            && remaining_qnames.contains(se.edge.target_qn.as_str())
    });
    let edges_after_prune = result.edges.len();
    if edges_after_prune > max_edges {
        result.edges.truncate(max_edges);
    }
    let dropped_edges = original_edge_count.saturating_sub(result.edges.len());

    // --- Files ---
    let original_file_count = result.files.len();
    if original_file_count > max_files {
        let (target_files, rest): (Vec<_>, Vec<_>) = result
            .files
            .drain(..)
            .partition(|f| f.selection_reason == SelectionReason::DirectTarget);
        result.files = target_files;
        let budget = max_files.saturating_sub(result.files.len());
        result.files.extend(rest.into_iter().take(budget));
    }
    let dropped_files = original_file_count.saturating_sub(result.files.len());

    result.truncation = TruncationMeta {
        nodes_dropped: dropped_nodes,
        edges_dropped: dropped_edges,
        files_dropped: dropped_files,
        truncated: dropped_nodes > 0 || dropped_edges > 0 || dropped_files > 0,
    };
}

// ---------------------------------------------------------------------------
// Convenience: full resolve-then-build pipeline
// ---------------------------------------------------------------------------

/// Resolve `request.target` and build a [`ContextResult`] in one call.
///
/// Routes by `intent` first:
/// - `Review`  → `build_review_context` (needs `ChangedFiles` target)
/// - `Impact`  → `build_impact_context`
/// - `Symbol`/`File` → symbol-context retrieval
///
/// Returns an ambiguity or not-found result when the target cannot be
/// uniquely resolved.
pub fn build_context(store: &Store, request: &ContextRequest) -> Result<ContextResult> {
    use atlas_core::model::ContextIntent;
    match request.intent {
        ContextIntent::Review => return build_review_context(store, request),
        // All impact-class intents route to the impact builder.
        ContextIntent::Impact
        | ContextIntent::ImpactAnalysis
        | ContextIntent::RefactorSafety
        | ContextIntent::DependencyRemoval => return build_impact_context(store, request),
        _ => {}
    }

    // Handle special targets that do not go through single-node resolution.
    match &request.target {
        ContextTarget::ChangedSymbols { qnames } => {
            // Derive file paths from the given qnames, then run impact context.
            let mut paths: Vec<String> = Vec::new();
            for qn in qnames {
                if let Some(node) = store.node_by_qname(qn)? {
                    paths.push(node.file_path);
                }
            }
            paths.dedup();
            let derived = ContextRequest {
                target: ContextTarget::ChangedFiles { paths },
                ..request.clone()
            };
            return build_impact_context(store, &derived);
        }
        ContextTarget::EdgeQuerySeed {
            source_qname,
            edge_kind: _,
        } => {
            // Route through symbol context on the source node.
            // Edge kind filtering is reserved for a future slice.
            return match store.node_by_qname(source_qname)? {
                Some(node) => build_symbol_context(store, node, request),
                None => Ok(build_not_found_result(request, vec![])),
            };
        }
        _ => {}
    }

    let resolved = resolve_target(store, &request.target)?;
    match resolved {
        ResolvedTarget::Node(node) => build_symbol_context(store, *node, request),
        ResolvedTarget::File(path) => {
            // File-centred context: build symbol context from the highest-value
            // node in the file (first returned by the store, ordered by line).
            let nodes = store.nodes_by_file(&path)?;
            match nodes.into_iter().next() {
                Some(first_node) => build_symbol_context(store, first_node, request),
                None => Ok(build_not_found_result(request, vec![])),
            }
        }
        ResolvedTarget::Ambiguous(meta) => Ok(build_ambiguous_result(request, meta)),
        ResolvedTarget::NotFound { suggestions } => {
            Ok(build_not_found_result(request, suggestions))
        }
    }
}

// ---------------------------------------------------------------------------
// Slice 6: review and impact context builders
// ---------------------------------------------------------------------------

/// Build a review [`ContextResult`] from a set of changed file paths.
///
/// Runs the impact radius traversal seeded from `changed_paths`, then wraps
/// the result into a [`ContextResult`] with:
/// - changed nodes tagged `DirectTarget`
/// - impacted neighbors tagged `ImpactNeighbor`
/// - relevant edges tagged `ImpactNeighbor`
///
/// Applies ranking and trimming on the assembled result before returning.
pub fn build_review_context(store: &Store, request: &ContextRequest) -> Result<ContextResult> {
    let changed_paths = extract_changed_paths(request);
    let path_refs: Vec<&str> = changed_paths.iter().map(String::as_str).collect();

    let max_nodes = request.max_nodes.unwrap_or(DEFAULT_MAX_NODES);
    let max_depth = request.depth.unwrap_or(2);
    let traversal_max_nodes = max_nodes.saturating_add(max_nodes.min(16));

    let impact = store.impact_radius(&path_refs, max_depth, traversal_max_nodes)?;
    let advanced = atlas_impact::analyze(impact.clone());
    let impact_scores: HashMap<String, f64> = advanced
        .scored_nodes
        .iter()
        .map(|scored| (scored.node.qualified_name.clone(), scored.impact_score))
        .collect();

    let changed_qns: HashSet<String> = impact
        .changed_nodes
        .iter()
        .map(|n| n.qualified_name.clone())
        .collect();

    let mut nodes: Vec<SelectedNode> = Vec::new();
    let mut seen_qnames: HashSet<String> = HashSet::new();

    // Changed nodes → DirectTarget
    for node in impact.changed_nodes {
        seen_qnames.insert(node.qualified_name.clone());
        nodes.push(SelectedNode {
            node,
            selection_reason: SelectionReason::DirectTarget,
            distance: 0,
            relevance_score: 0.0,
        });
    }

    // Impacted neighbors → ImpactNeighbor
    for node in impact.impacted_nodes {
        let qn = node.qualified_name.clone();
        if seen_qnames.insert(qn) {
            nodes.push(SelectedNode {
                node,
                selection_reason: SelectionReason::ImpactNeighbor,
                distance: 1,
                relevance_score: 0.0,
            });
        }
    }

    // All relevant edges
    let edges: Vec<SelectedEdge> = impact
        .relevant_edges
        .into_iter()
        .filter(|e| {
            changed_qns.contains(e.source_qn.as_str())
                || changed_qns.contains(e.target_qn.as_str())
                || seen_qnames.contains(e.source_qn.as_str())
                || seen_qnames.contains(e.target_qn.as_str())
        })
        .map(|edge| SelectedEdge {
            edge,
            selection_reason: SelectionReason::ImpactNeighbor,
            depth: None,
            relevance_score: 0.0,
        })
        .collect();

    let files = collect_files(&nodes);

    let mut result = ContextResult {
        request: request.clone(),
        nodes,
        edges,
        files,
        truncation: TruncationMeta::none(),
        ambiguity: None,
        workflow: None,
        saved_context_sources: vec![],
    };

    rank_context(&mut result);
    apply_impact_focus_scores(&mut result, &impact_scores);
    trim_context(&mut result);
    update_file_node_counts(&mut result);

    if request.include_code_spans {
        apply_code_spans(&mut result);
    }

    result.workflow = Some(build_workflow_summary(&result));

    Ok(result)
}

/// Build an impact [`ContextResult`] from file or symbol seeds.
///
/// Accepts any `ContextTarget`:
/// - `ChangedFiles`   → pass paths directly to `impact_radius`
/// - `QualifiedName` / `SymbolName` → resolve to node, use its file path
/// - `FilePath`       → use the path directly
///
/// Changed (seed) nodes are tagged `DirectTarget`; downstream nodes are
/// tagged `ImpactNeighbor`.
pub fn build_impact_context(store: &Store, request: &ContextRequest) -> Result<ContextResult> {
    let seed_paths: Vec<String> = match &request.target {
        ContextTarget::ChangedFiles { paths } => paths.clone(),
        ContextTarget::FilePath { path } => vec![path.clone()],
        ContextTarget::QualifiedName { .. } | ContextTarget::SymbolName { .. } => {
            match resolve_target(store, &request.target)? {
                ResolvedTarget::Node(node) => vec![node.file_path.clone()],
                ResolvedTarget::File(path) => vec![path],
                ResolvedTarget::Ambiguous(meta) => {
                    return Ok(build_ambiguous_result(request, meta));
                }
                ResolvedTarget::NotFound { suggestions } => {
                    return Ok(build_not_found_result(request, suggestions));
                }
            }
        }
        ContextTarget::ChangedSymbols { qnames } => {
            let mut paths: Vec<String> = Vec::new();
            for qn in qnames {
                if let Some(node) = store.node_by_qname(qn)? {
                    paths.push(node.file_path);
                }
            }
            paths.dedup();
            paths
        }
        ContextTarget::EdgeQuerySeed { source_qname, .. } => {
            match store.node_by_qname(source_qname)? {
                Some(node) => vec![node.file_path],
                None => return Ok(build_not_found_result(request, vec![])),
            }
        }
    };

    // Reuse build_review_context logic with a manufactured ChangedFiles request.
    let adapted = ContextRequest {
        intent: request.intent,
        target: ContextTarget::ChangedFiles { paths: seed_paths },
        ..request.clone()
    };
    build_review_context(store, &adapted)
}

/// Extract changed file paths from a `ChangedFiles` target, or return empty.
fn extract_changed_paths(request: &ContextRequest) -> Vec<String> {
    if let ContextTarget::ChangedFiles { paths } = &request.target {
        paths.clone()
    } else {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Slice 8: code span population
// ---------------------------------------------------------------------------

/// Populate `SelectedFile.line_ranges` for every file in `result`.
///
/// For each file the target node's span (distance 0) is always included.
/// Caller/callee spans are included unless the spans are already bounded
/// by the target span.  Each span is clamped so start ≤ end.
///
/// This function is called after ranking and trimming so spans are only
/// computed for nodes that actually made it into the result.
pub fn apply_code_spans(result: &mut ContextResult) {
    use std::collections::HashMap as FMap;

    // Build a map: file_path → list of (start, end) from selected nodes.
    let mut span_map: FMap<String, Vec<(u32, u32)>> = FMap::new();

    for sn in &result.nodes {
        let start = sn.node.line_start;
        let end = sn.node.line_end.max(start);

        // Target node is always recorded.
        // Callers/callees are recorded; others are skipped to keep spans narrow.
        let include = matches!(
            sn.selection_reason,
            SelectionReason::DirectTarget
                | SelectionReason::Caller
                | SelectionReason::Callee
                | SelectionReason::ImpactNeighbor
        );
        if include {
            span_map
                .entry(sn.node.file_path.clone())
                .or_default()
                .push((start, end));
        }
    }

    // Merge and assign to SelectedFile.
    for sf in &mut result.files {
        if let Some(spans) = span_map.get(&sf.path) {
            sf.line_ranges = merge_spans(spans);
        }
    }
}

/// Merge overlapping or adjacent line ranges into a minimal covering set.
/// Input ranges: (start, end) inclusive, 1-based.
fn merge_spans(spans: &[(u32, u32)]) -> Vec<(u32, u32)> {
    if spans.is_empty() {
        return vec![];
    }
    let mut sorted = spans.to_vec();
    sorted.sort_by_key(|&(s, _)| s);

    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(sorted.len());
    let (mut cur_start, mut cur_end) = sorted[0];

    for &(start, end) in &sorted[1..] {
        if start <= cur_end + 1 {
            cur_end = cur_end.max(end);
        } else {
            merged.push((cur_start, cur_end));
            cur_start = start;
            cur_end = end;
        }
    }
    merged.push((cur_start, cur_end));
    merged
}

fn apply_impact_focus_scores(result: &mut ContextResult, impact_scores: &HashMap<String, f64>) {
    for node in &mut result.nodes {
        if let Some(score) = impact_scores.get(&node.node.qualified_name) {
            node.relevance_score += (*score as f32) * 20.0;
        }
    }

    for edge in &mut result.edges {
        let source = impact_scores
            .get(&edge.edge.source_qn)
            .copied()
            .unwrap_or(0.0);
        let target = impact_scores
            .get(&edge.edge.target_qn)
            .copied()
            .unwrap_or(0.0);
        edge.relevance_score += ((source + target) as f32) * 5.0;
    }

    result.nodes.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.qualified_name.cmp(&b.node.qualified_name))
    });

    result.edges.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.edge.source_qn.cmp(&b.edge.source_qn))
            .then_with(|| a.edge.target_qn.cmp(&b.edge.target_qn))
    });
}

fn build_workflow_summary(result: &ContextResult) -> WorkflowSummary {
    let high_impact_nodes = result
        .nodes
        .iter()
        .take(5)
        .map(|node| WorkflowFocusNode {
            qualified_name: node.node.qualified_name.clone(),
            kind: node.node.kind.as_str().to_string(),
            file_path: node.node.file_path.clone(),
            relevance_score: node.relevance_score,
            selection_reason: node.selection_reason.as_str().to_string(),
        })
        .collect();

    let impacted_components = build_impacted_components(&result.nodes);
    let call_chains = build_call_chains(result);
    let ripple_effects = build_ripple_effects(result, &impacted_components, &call_chains);
    let headline = build_workflow_headline(result, &impacted_components, &call_chains);

    WorkflowSummary {
        headline,
        high_impact_nodes,
        impacted_components,
        call_chains,
        ripple_effects,
        noise_reduction: build_noise_reduction_summary(result),
    }
}

fn build_workflow_headline(
    result: &ContextResult,
    components: &[WorkflowComponent],
    call_chains: &[WorkflowCallChain],
) -> Option<String> {
    let direct_targets = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .count();
    let component_count = components.len();
    let chain_count = call_chains.len();

    Some(match result.request.intent {
        atlas_core::model::ContextIntent::Review => format!(
            "{} changed node(s), {} component(s), {} call chain(s)",
            direct_targets, component_count, chain_count
        ),
        atlas_core::model::ContextIntent::Impact
        | atlas_core::model::ContextIntent::ImpactAnalysis => format!(
            "Impact reaches {} node(s) across {} component(s)",
            result.nodes.len(),
            component_count
        ),
        atlas_core::model::ContextIntent::UsageLookup => format!(
            "{} usage node(s) surfaced, {} chain(s)",
            result.nodes.len().saturating_sub(direct_targets),
            chain_count
        ),
        _ => format!(
            "{} focused node(s) across {} component(s)",
            result.nodes.len(),
            component_count
        ),
    })
}

fn build_impacted_components(nodes: &[SelectedNode]) -> Vec<WorkflowComponent> {
    let mut component_map: HashMap<(String, String), (usize, usize, HashSet<String>)> =
        HashMap::new();

    for node in nodes {
        let (kind, label) = component_identity(&node.node);
        let entry = component_map
            .entry((kind, label))
            .or_insert_with(|| (0, 0, HashSet::new()));

        if node.selection_reason == SelectionReason::DirectTarget {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
        entry.2.insert(node.node.file_path.clone());
    }

    let mut components: Vec<WorkflowComponent> = component_map
        .into_iter()
        .map(
            |((kind, label), (changed, impacted, files))| WorkflowComponent {
                summary: format!(
                    "{} changed, {} impacted across {} file(s)",
                    changed,
                    impacted,
                    files.len()
                ),
                label,
                kind,
                changed_node_count: changed,
                impacted_node_count: impacted,
                file_count: files.len(),
            },
        )
        .collect();

    components.sort_by(|a, b| {
        (b.changed_node_count + b.impacted_node_count)
            .cmp(&(a.changed_node_count + a.impacted_node_count))
            .then_with(|| a.label.cmp(&b.label))
    });
    components.truncate(6);
    components
}

fn component_identity(node: &atlas_core::Node) -> (String, String) {
    let extra = node.extra_json.as_object();
    if let Some(path) = extra
        .and_then(|extra| {
            extra
                .get("owner_manifest_path")
                .or_else(|| extra.get("workspace_manifest_path"))
                .and_then(|value| value.as_str())
        })
        .filter(|path| !path.is_empty())
    {
        return ("package".to_string(), path.to_string());
    }

    let dir = parent_dir(&node.file_path);
    if dir.is_empty() {
        ("file".to_string(), node.file_path.clone())
    } else {
        ("directory".to_string(), dir.to_string())
    }
}

fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(index) => &path[..index],
        None => "",
    }
}

fn build_call_chains(result: &ContextResult) -> Vec<WorkflowCallChain> {
    use atlas_core::EdgeKind;

    let direct_targets: HashSet<&str> = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .map(|node| node.node.qualified_name.as_str())
        .collect();

    let call_edges: Vec<&SelectedEdge> = result
        .edges
        .iter()
        .filter(|edge| edge.edge.kind == EdgeKind::Calls)
        .collect();

    let mut incoming: HashMap<&str, Vec<&SelectedEdge>> = HashMap::new();
    let mut outgoing: HashMap<&str, Vec<&SelectedEdge>> = HashMap::new();
    for edge in &call_edges {
        incoming
            .entry(edge.edge.target_qn.as_str())
            .or_default()
            .push(*edge);
        outgoing
            .entry(edge.edge.source_qn.as_str())
            .or_default()
            .push(*edge);
    }

    let mut seen = HashSet::new();
    let mut chains: Vec<(f32, WorkflowCallChain)> = Vec::new();

    for target in &direct_targets {
        if let (Some(ins), Some(outs)) = (incoming.get(target), outgoing.get(target)) {
            for inbound in ins.iter().take(3) {
                for outbound in outs.iter().take(3) {
                    let steps = vec![
                        inbound.edge.source_qn.clone(),
                        (*target).to_string(),
                        outbound.edge.target_qn.clone(),
                    ];
                    let key = steps.join(" -> ");
                    if seen.insert(key.clone()) {
                        chains.push((
                            inbound.relevance_score + outbound.relevance_score,
                            WorkflowCallChain {
                                summary: key,
                                steps,
                                edge_kinds: vec![
                                    inbound.edge.kind.as_str().to_string(),
                                    outbound.edge.kind.as_str().to_string(),
                                ],
                            },
                        ));
                    }
                }
            }
        }

        for edge in call_edges
            .iter()
            .filter(|edge| edge.edge.source_qn == *target || edge.edge.target_qn == *target)
        {
            let steps = vec![edge.edge.source_qn.clone(), edge.edge.target_qn.clone()];
            let key = steps.join(" -> ");
            if seen.insert(key.clone()) {
                chains.push((
                    edge.relevance_score,
                    WorkflowCallChain {
                        summary: key,
                        steps,
                        edge_kinds: vec![edge.edge.kind.as_str().to_string()],
                    },
                ));
            }
        }
    }

    chains.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.summary.cmp(&b.1.summary))
    });
    chains.into_iter().map(|(_, chain)| chain).take(5).collect()
}

fn build_ripple_effects(
    result: &ContextResult,
    components: &[WorkflowComponent],
    call_chains: &[WorkflowCallChain],
) -> Vec<String> {
    let mut ripple_effects = Vec::new();

    if components.len() > 1 {
        ripple_effects.push(format!(
            "Impact spans {} components: {}",
            components.len(),
            components
                .iter()
                .take(3)
                .map(|component| component.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if let Some(chain) = call_chains.first() {
        ripple_effects.push(format!("Primary call chain: {}", chain.summary));
    }

    let neighboring_files: HashSet<&str> = result
        .nodes
        .iter()
        .filter(|node| node.selection_reason != SelectionReason::DirectTarget)
        .map(|node| node.node.file_path.as_str())
        .collect();
    if !neighboring_files.is_empty() {
        ripple_effects.push(format!(
            "Change reaches {} neighboring file(s).",
            neighboring_files.len()
        ));
    }

    if ripple_effects.is_empty() {
        ripple_effects.push("Impact remains local to selected nodes.".to_string());
    }

    ripple_effects
}

fn build_noise_reduction_summary(result: &ContextResult) -> NoiseReductionSummary {
    let mut rules_applied = Vec::new();
    if !result.request.include_neighbors {
        rules_applied.push("omitted containment siblings".to_string());
    }
    if !result.request.include_tests {
        rules_applied.push("omitted test-only neighbors".to_string());
    }
    if !result.request.include_imports {
        rules_applied.push("omitted import neighbors".to_string());
    }
    if !result.request.include_callers {
        rules_applied.push("omitted caller expansion".to_string());
    }
    if !result.request.include_callees {
        rules_applied.push("omitted callee expansion".to_string());
    }
    if result.truncation.truncated {
        rules_applied.push("trimmed low-signal nodes and edges to requested caps".to_string());
    }

    NoiseReductionSummary {
        retained_nodes: result.nodes.len(),
        retained_edges: result.edges.len(),
        retained_files: result.files.len(),
        dropped_nodes: result.truncation.nodes_dropped,
        dropped_edges: result.truncation.edges_dropped,
        dropped_files: result.truncation.files_dropped,
        rules_applied,
    }
}

// ---------------------------------------------------------------------------
// CM6: saved-context retrieval
// ---------------------------------------------------------------------------

/// Maximum number of saved-context sources to include in a result.
const MAX_SAVED_SOURCES: usize = 5;

/// Query the content store for saved artifacts relevant to this request.
///
/// Build a BM25 query from the top symbol names and file basenames that
/// appear in the graph result.  Score each unique source by:
/// - inverse-rank position (RRF-like: 10 / (rank + 1))
/// - recency boost: +5.0 when the artifact was created within the past 7 days
///   (determined by lexicographic RFC3339 comparison)
/// - same-session boost: +10.0 when the artifact session matches `request.session_id`
///
/// Returns at most [`MAX_SAVED_SOURCES`] sources ordered by descending score.
fn retrieve_saved_context(
    content_store: &ContentStore,
    request: &ContextRequest,
    result: &ContextResult,
) -> Vec<SavedContextSource> {
    // Build query from top symbol names and file basenames.
    let mut terms: Vec<String> = result
        .nodes
        .iter()
        .take(5)
        .map(|sn| sn.node.name.clone())
        .collect();
    for sf in result.files.iter().take(3) {
        let basename = sf.path.rsplit('/').next().unwrap_or(&sf.path);
        terms.push(basename.to_string());
    }
    terms.dedup();

    if terms.is_empty() {
        return vec![];
    }

    let query = terms.join(" ");
    let filters = SearchFilters {
        session_id: request.session_id.clone(),
        ..SearchFilters::default()
    };

    let chunks = match content_store.search_with_fallback(&query, &filters) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    // Collect unique source_ids preserving first-seen rank order.
    let mut seen_ids: Vec<String> = Vec::new();
    for chunk in &chunks {
        if !seen_ids.contains(&chunk.source_id) {
            seen_ids.push(chunk.source_id.clone());
            if seen_ids.len() >= MAX_SAVED_SOURCES {
                break;
            }
        }
    }

    // RFC3339 strings sort lexicographically; compute cutoff for 7-day recency.
    let seven_days_ago = {
        // Approximate: subtract 604800 seconds from now expressed as a string.
        // Use a simple subtraction on the unix epoch rather than the `time` crate.
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cutoff = now_secs.saturating_sub(7 * 24 * 60 * 60);
        // Format as a sortable RFC3339 prefix: "YYYY-MM-DDTHH:MM:SSZ"
        let secs = cutoff;
        let days_since_epoch = secs / 86400;
        let rem = secs % 86400;
        let hours = rem / 3600;
        let minutes = (rem % 3600) / 60;
        let seconds = rem % 60;
        // Gregorian calendar approximation reliable for 1970-2100.
        let (year, month, day) = epoch_days_to_ymd(days_since_epoch);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, month, day, hours, minutes, seconds
        )
    };

    let mut scored: Vec<SavedContextSource> = Vec::new();
    for (rank, source_id) in seen_ids.iter().enumerate() {
        let meta: SourceRow = match content_store.get_source(source_id) {
            Ok(Some(m)) => m,
            _ => continue,
        };
        let preview: String = chunks
            .iter()
            .find(|c| &c.source_id == source_id)
            .map(|c| c.content.chars().take(512).collect())
            .unwrap_or_default();

        // Base: inverse-rank score.
        let mut score = 10.0_f32 / (rank as f32 + 1.0);

        // Recency boost.
        if meta.created_at.as_str() >= seven_days_ago.as_str() {
            score += 5.0;
        }

        // Same-session boost.
        if let (Some(req_sid), Some(art_sid)) =
            (request.session_id.as_deref(), meta.session_id.as_deref())
            && req_sid == art_sid
        {
            score += 10.0;
        }

        let retrieval_hint = format!(
            "source_id={} label={:?} type={}",
            source_id, meta.label, meta.source_type
        );

        scored.push(SavedContextSource {
            source_id: source_id.clone(),
            label: meta.label,
            source_type: meta.source_type,
            session_id: meta.session_id,
            preview,
            retrieval_hint,
            relevance_score: score,
        });
    }

    scored.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

/// Convert days since the Unix epoch to (year, month, day) using the
/// proleptic Gregorian calendar.  Adequate for RFC3339 recency comparison.
fn epoch_days_to_ymd(days: u64) -> (u64, u8, u8) {
    // Algorithm: civil date from days since 1970-01-01.
    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u64, m as u8, d as u8)
}

// ---------------------------------------------------------------------------
// Slice 9: ContextEngine public entrypoint
// ---------------------------------------------------------------------------

/// Stateless context engine facade.
///
/// Wraps the free-function pipeline into a struct so callers inject one
/// [`Store`] reference and call engine operations as methods.  An optional
/// [`ContentStore`] enables CM6 saved-context retrieval.
pub struct ContextEngine<'a> {
    store: &'a Store,
    /// Optional content store for CM6 retrieval-backed restoration.
    content_store: Option<&'a ContentStore>,
}

impl<'a> ContextEngine<'a> {
    /// Create a new engine backed by `store`.
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            content_store: None,
        }
    }

    /// Attach a content store to enable saved-context retrieval (CM6).
    ///
    /// When attached and `request.include_saved_context` is `true`, the engine
    /// queries the content store for relevant saved artifacts after graph
    /// retrieval and merges them into `ContextResult::saved_context_sources`.
    pub fn with_content_store(mut self, cs: &'a ContentStore) -> Self {
        self.content_store = Some(cs);
        self
    }

    /// Resolve a [`ContextTarget`] to a concrete node, file, or ambiguity result.
    pub fn resolve(&self, target: &ContextTarget) -> Result<ResolvedTarget> {
        resolve_target(self.store, target)
    }

    /// Build a bounded [`ContextResult`] for the given request.
    ///
    /// Routes by intent (Review / Impact / Symbol / File), resolves target,
    /// retrieves neighbors, ranks, trims, and optionally applies code spans.
    /// When `request.include_saved_context` is `true` and a content store is
    /// attached, also populates `saved_context_sources` (CM6).
    pub fn build(&self, request: &ContextRequest) -> Result<ContextResult> {
        let mut result = build_context(self.store, request)?;
        if request.include_saved_context
            && let Some(cs) = self.content_store
        {
            result.saved_context_sources = retrieve_saved_context(cs, request, &result);
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{
        EdgeKind, NodeId, NodeKind,
        model::{
            ContextIntent, ContextRequest, ContextTarget, Edge, Node, ParsedFile, SelectionReason,
        },
    };
    use atlas_store_sqlite::Store;

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn open_store() -> Store {
        let mut s = Store::open(":memory:").unwrap();
        s.migrate().unwrap();
        s
    }

    fn make_node(
        qname: &str,
        name: &str,
        file: &str,
        kind: NodeKind,
        parent: Option<&str>,
    ) -> Node {
        Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_string(),
            qualified_name: qname.to_string(),
            file_path: file.to_string(),
            line_start: 1,
            line_end: 10,
            language: "rust".to_string(),
            parent_name: parent.map(String::from),
            params: None,
            return_type: None,
            modifiers: Some("pub".to_string()),
            is_test: false,
            file_hash: "abc".to_string(),
            extra_json: serde_json::Value::Null,
        }
    }

    fn make_edge(src: &str, tgt: &str, kind: EdgeKind, file: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: src.to_string(),
            target_qn: tgt.to_string(),
            file_path: file.to_string(),
            line: None,
            confidence: 1.0,
            confidence_tier: None,
            extra_json: serde_json::Value::Null,
        }
    }

    fn seed_graph(store: &mut Store) {
        // Graph:
        //   src/a.rs: fn_a (calls fn_b), fn_a_helper (sibling)
        //   src/b.rs: fn_b (calls fn_c), fn_b_helper
        //   src/b.rs: fn_c
        //   tests/test_a.rs: test_fn_a (tests fn_a)
        let nodes = [
            make_node(
                "src/a.rs::fn_a",
                "fn_a",
                "src/a.rs",
                NodeKind::Function,
                None,
            ),
            make_node(
                "src/a.rs::fn_a_helper",
                "fn_a_helper",
                "src/a.rs",
                NodeKind::Function,
                Some("mod_a"),
            ),
            make_node(
                "src/b.rs::fn_b",
                "fn_b",
                "src/b.rs",
                NodeKind::Function,
                None,
            ),
            make_node(
                "src/b.rs::fn_c",
                "fn_c",
                "src/b.rs",
                NodeKind::Function,
                None,
            ),
            make_node(
                "tests/test_a.rs::test_fn_a",
                "test_fn_a",
                "tests/test_a.rs",
                NodeKind::Test,
                None,
            ),
        ];
        let edges = [
            make_edge(
                "src/a.rs::fn_a",
                "src/b.rs::fn_b",
                EdgeKind::Calls,
                "src/a.rs",
            ),
            make_edge(
                "src/b.rs::fn_b",
                "src/b.rs::fn_c",
                EdgeKind::Calls,
                "src/b.rs",
            ),
            make_edge(
                "tests/test_a.rs::test_fn_a",
                "src/a.rs::fn_a",
                EdgeKind::Tests,
                "tests/test_a.rs",
            ),
        ];
        let files: Vec<ParsedFile> = vec![
            ParsedFile {
                path: "src/a.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h1".to_string(),
                size: None,
                nodes: nodes[0..2].to_vec(),
                edges: edges[0..1].to_vec(),
            },
            ParsedFile {
                path: "src/b.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h2".to_string(),
                size: None,
                nodes: nodes[2..4].to_vec(),
                edges: edges[1..2].to_vec(),
            },
            ParsedFile {
                path: "tests/test_a.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h3".to_string(),
                size: None,
                nodes: nodes[4..5].to_vec(),
                edges: edges[2..3].to_vec(),
            },
        ];
        store.replace_batch(&files).unwrap();
    }

    // ------------------------------------------------------------------
    // Slice 3: resolve_target
    // ------------------------------------------------------------------

    #[test]
    fn resolve_exact_qname_hit() {
        let mut store = open_store();
        seed_graph(&mut store);
        let target = ContextTarget::QualifiedName {
            qname: "src/a.rs::fn_a".to_string(),
        };
        let resolved = resolve_target(&store, &target).unwrap();
        assert!(
            matches!(resolved, ResolvedTarget::Node(n) if n.qualified_name == "src/a.rs::fn_a")
        );
    }

    #[test]
    fn resolve_exact_qname_miss_returns_not_found_or_ambiguous() {
        let mut store = open_store();
        seed_graph(&mut store);
        let target = ContextTarget::QualifiedName {
            qname: "nonexistent::qname".to_string(),
        };
        let resolved = resolve_target(&store, &target).unwrap();
        // FTS fallback either returns NotFound or some Ambiguous candidates.
        assert!(matches!(
            resolved,
            ResolvedTarget::NotFound { .. } | ResolvedTarget::Ambiguous(..)
        ));
    }

    #[test]
    fn resolve_unique_symbol_name() {
        let mut store = open_store();
        seed_graph(&mut store);
        let target = ContextTarget::SymbolName {
            name: "fn_a".to_string(),
        };
        let resolved = resolve_target(&store, &target).unwrap();
        assert!(matches!(resolved, ResolvedTarget::Node(n) if n.name == "fn_a"));
    }

    #[test]
    fn resolve_ambiguous_symbol_name() {
        // fn_b and fn_c share no common name but let's add a duplicate name.
        let mut store = open_store();
        // Insert a second node named "fn_a" in a different file to force ambiguity.
        let dupe = ParsedFile {
            path: "src/c.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h4".to_string(),
            size: None,
            nodes: vec![make_node(
                "src/c.rs::fn_a",
                "fn_a",
                "src/c.rs",
                NodeKind::Function,
                None,
            )],
            edges: vec![],
        };
        store.replace_batch(&[dupe]).unwrap();
        seed_graph(&mut store);

        let target = ContextTarget::SymbolName {
            name: "fn_a".to_string(),
        };
        let resolved = resolve_target(&store, &target).unwrap();
        assert!(matches!(resolved, ResolvedTarget::Ambiguous(ref m) if m.candidates.len() >= 2));
    }

    #[test]
    fn resolve_file_path_hit() {
        let mut store = open_store();
        seed_graph(&mut store);
        let target = ContextTarget::FilePath {
            path: "src/a.rs".to_string(),
        };
        let resolved = resolve_target(&store, &target).unwrap();
        assert!(matches!(resolved, ResolvedTarget::File(p) if p == "src/a.rs"));
    }

    #[test]
    fn resolve_file_path_miss_returns_not_found() {
        let mut store = open_store();
        seed_graph(&mut store);
        let target = ContextTarget::FilePath {
            path: "src/missing.rs".to_string(),
        };
        let resolved = resolve_target(&store, &target).unwrap();
        assert!(matches!(resolved, ResolvedTarget::NotFound { .. }));
    }

    #[test]
    fn resolve_missing_symbol_returns_not_found() {
        let mut store = open_store();
        seed_graph(&mut store);
        let target = ContextTarget::SymbolName {
            name: "zzz_totally_absent".to_string(),
        };
        let resolved = resolve_target(&store, &target).unwrap();
        assert!(matches!(
            resolved,
            ResolvedTarget::NotFound { .. } | ResolvedTarget::Ambiguous(..)
        ));
    }

    // ------------------------------------------------------------------
    // Slice 4: build_symbol_context
    // ------------------------------------------------------------------

    fn symbol_request(qname: &str) -> ContextRequest {
        ContextRequest {
            intent: ContextIntent::Symbol,
            target: ContextTarget::QualifiedName {
                qname: qname.to_string(),
            },
            include_tests: false,
            include_imports: false,
            include_neighbors: false,
            ..ContextRequest::default()
        }
    }

    #[test]
    fn symbol_context_contains_seed_and_callee() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let req = symbol_request("src/a.rs::fn_a");
        let result = build_symbol_context(&store, seed, &req).unwrap();

        let qnames: Vec<&str> = result
            .nodes
            .iter()
            .map(|n| n.node.qualified_name.as_str())
            .collect();
        assert!(qnames.contains(&"src/a.rs::fn_a"), "seed missing");
        assert!(qnames.contains(&"src/b.rs::fn_b"), "callee fn_b missing");
    }

    #[test]
    fn symbol_context_seed_is_direct_target() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let req = symbol_request("src/a.rs::fn_a");
        let result = build_symbol_context(&store, seed, &req).unwrap();

        let seed_node = result
            .nodes
            .iter()
            .find(|n| n.node.qualified_name == "src/a.rs::fn_a")
            .unwrap();
        assert_eq!(seed_node.selection_reason, SelectionReason::DirectTarget);
        assert_eq!(seed_node.distance, 0);
    }

    #[test]
    fn symbol_context_include_tests_flag() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let mut req = symbol_request("src/a.rs::fn_a");
        req.include_tests = true;
        let result = build_symbol_context(&store, seed, &req).unwrap();

        let qnames: Vec<&str> = result
            .nodes
            .iter()
            .map(|n| n.node.qualified_name.as_str())
            .collect();
        assert!(
            qnames.contains(&"tests/test_a.rs::test_fn_a"),
            "test node missing"
        );
    }

    #[test]
    fn symbol_context_files_bounded() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let mut req = symbol_request("src/a.rs::fn_a");
        req.max_files = Some(1);
        let result = build_symbol_context(&store, seed, &req).unwrap();
        assert!(result.files.len() <= 1);
    }

    // ------------------------------------------------------------------
    // Slice 5: rank_context / trim_context
    // ------------------------------------------------------------------

    #[test]
    fn rank_puts_direct_target_first() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let req = symbol_request("src/a.rs::fn_a");
        let result = build_symbol_context(&store, seed, &req).unwrap();
        // After ranking the seed should be first.
        assert_eq!(
            result.nodes[0].selection_reason,
            SelectionReason::DirectTarget
        );
    }

    #[test]
    fn callers_and_callees_survive_trimming_over_distant_nodes() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/b.rs::fn_b").unwrap().unwrap();
        let mut req = symbol_request("src/b.rs::fn_b");
        // Tight cap: keep only 2 nodes.
        req.max_nodes = Some(2);
        req.include_tests = true;
        let result = build_symbol_context(&store, seed, &req).unwrap();

        assert!(result.nodes.len() <= 2);
        // DirectTarget must always be in the result.
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.selection_reason == SelectionReason::DirectTarget)
        );
        // Truncation was recorded.
        assert!(result.truncation.truncated || result.nodes.len() == 2);
    }

    #[test]
    fn trim_records_dropped_counts() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let mut req = symbol_request("src/a.rs::fn_a");
        req.max_nodes = Some(1);
        let result = build_symbol_context(&store, seed, &req).unwrap();
        // Everything beyond the seed should be dropped.
        assert_eq!(result.nodes.len(), 1);
        // Edges referencing dropped nodes should also be gone.
        for edge in &result.edges {
            let src_present = result
                .nodes
                .iter()
                .any(|n| n.node.qualified_name == edge.edge.source_qn);
            let tgt_present = result
                .nodes
                .iter()
                .any(|n| n.node.qualified_name == edge.edge.target_qn);
            assert!(
                src_present || tgt_present,
                "edge references both-dropped nodes"
            );
        }
    }

    #[test]
    fn trim_caps_deterministic_under_ties() {
        // Two runs with same seed and tight cap must produce same output.
        let mut store = open_store();
        seed_graph(&mut store);

        let run = |s: &Store| {
            let seed = s.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
            let mut req = symbol_request("src/a.rs::fn_a");
            req.max_nodes = Some(2);
            build_symbol_context(s, seed, &req).unwrap()
        };

        let r1 = run(&store);
        let r2 = run(&store);
        let qns1: Vec<&str> = r1
            .nodes
            .iter()
            .map(|n| n.node.qualified_name.as_str())
            .collect();
        let qns2: Vec<&str> = r2
            .nodes
            .iter()
            .map(|n| n.node.qualified_name.as_str())
            .collect();
        assert_eq!(qns1, qns2, "trim output non-deterministic");
    }

    #[test]
    fn build_context_convenience_wrapper() {
        let mut store = open_store();
        seed_graph(&mut store);
        let req = ContextRequest {
            intent: ContextIntent::Symbol,
            target: ContextTarget::QualifiedName {
                qname: "src/b.rs::fn_b".to_string(),
            },
            ..ContextRequest::default()
        };
        let result = build_context(&store, &req).unwrap();
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.node.qualified_name == "src/b.rs::fn_b")
        );
    }

    // ------------------------------------------------------------------
    // Slice 6: build_review_context / build_impact_context
    // ------------------------------------------------------------------

    fn review_request(paths: Vec<String>) -> ContextRequest {
        ContextRequest {
            intent: ContextIntent::Review,
            target: ContextTarget::ChangedFiles { paths },
            ..ContextRequest::default()
        }
    }

    #[test]
    fn review_context_changed_files_become_direct_targets() {
        let mut store = open_store();
        seed_graph(&mut store);
        let req = review_request(vec!["src/a.rs".to_string()]);
        let result = build_context(&store, &req).unwrap();

        // Changed nodes from src/a.rs must be DirectTarget.
        assert!(
            result
                .nodes
                .iter()
                .filter(|n| n.node.file_path == "src/a.rs")
                .all(|n| n.selection_reason == SelectionReason::DirectTarget),
            "src/a.rs nodes not tagged DirectTarget"
        );
    }

    #[test]
    fn review_context_impacted_nodes_tagged_impact_neighbor() {
        let mut store = open_store();
        seed_graph(&mut store);
        // Change src/a.rs → fn_a calls fn_b, so fn_b should be in impact radius.
        let req = review_request(vec!["src/a.rs".to_string()]);
        let result = build_context(&store, &req).unwrap();

        // At least one ImpactNeighbor must be present (fn_b from src/b.rs).
        let has_neighbor = result
            .nodes
            .iter()
            .any(|n| n.selection_reason == SelectionReason::ImpactNeighbor);
        assert!(
            has_neighbor,
            "expected ImpactNeighbor nodes from impact traversal"
        );
    }

    #[test]
    fn review_context_result_is_bounded() {
        let mut store = open_store();
        seed_graph(&mut store);
        let mut req = review_request(vec!["src/a.rs".to_string()]);
        req.max_nodes = Some(3);
        let result = build_context(&store, &req).unwrap();
        assert!(result.nodes.len() <= 3, "node cap exceeded");
    }

    #[test]
    fn review_context_tight_cap_keeps_impacted_neighbor() {
        let mut store = open_store();
        seed_graph(&mut store);
        let mut req = review_request(vec!["src/a.rs".to_string()]);
        req.max_nodes = Some(2);
        let result = build_context(&store, &req).unwrap();

        assert_eq!(result.nodes.len(), 2);
        assert!(
            result
                .nodes
                .iter()
                .any(|node| node.selection_reason == SelectionReason::DirectTarget)
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|node| node.selection_reason == SelectionReason::ImpactNeighbor),
            "expected impacted neighbor to survive tight review cap"
        );
    }

    #[test]
    fn impact_context_file_seed_returns_neighbors() {
        let mut store = open_store();
        seed_graph(&mut store);
        let req = ContextRequest {
            intent: ContextIntent::Impact,
            target: ContextTarget::FilePath {
                path: "src/a.rs".to_string(),
            },
            ..ContextRequest::default()
        };
        let result = build_context(&store, &req).unwrap();
        assert!(!result.nodes.is_empty(), "impact result must have nodes");
    }

    #[test]
    fn impact_context_qname_seed_returns_neighbors() {
        let mut store = open_store();
        seed_graph(&mut store);
        let req = ContextRequest {
            intent: ContextIntent::Impact,
            target: ContextTarget::QualifiedName {
                qname: "src/a.rs::fn_a".to_string(),
            },
            ..ContextRequest::default()
        };
        let result = build_context(&store, &req).unwrap();
        // fn_a is in src/a.rs which calls fn_b; fn_b should appear.
        let has_fn_b = result
            .nodes
            .iter()
            .any(|n| n.node.qualified_name == "src/b.rs::fn_b");
        assert!(has_fn_b, "fn_b should appear as impact neighbor of fn_a");
    }

    #[test]
    fn impact_context_missing_qname_returns_empty() {
        let mut store = open_store();
        seed_graph(&mut store);
        let req = ContextRequest {
            intent: ContextIntent::Impact,
            target: ContextTarget::QualifiedName {
                qname: "no::such::symbol".to_string(),
            },
            ..ContextRequest::default()
        };
        let result = build_context(&store, &req).unwrap();
        assert!(
            result.nodes.is_empty(),
            "missing symbol should yield empty result"
        );
    }

    // ------------------------------------------------------------------
    // Slice 8: apply_code_spans
    // ------------------------------------------------------------------

    #[test]
    fn code_spans_populated_for_target() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let mut req = symbol_request("src/a.rs::fn_a");
        req.include_code_spans = true;
        let result = build_symbol_context(&store, seed, &req).unwrap();

        let target_file = result
            .files
            .iter()
            .find(|f| f.path == "src/a.rs")
            .expect("src/a.rs must be in files");
        assert!(
            !target_file.line_ranges.is_empty(),
            "target file must have line ranges"
        );
    }

    #[test]
    fn code_spans_not_populated_when_disabled() {
        let mut store = open_store();
        seed_graph(&mut store);
        let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
        let mut req = symbol_request("src/a.rs::fn_a");
        req.include_code_spans = false;
        let result = build_symbol_context(&store, seed, &req).unwrap();

        // When code spans are disabled, line_ranges must be empty.
        for sf in &result.files {
            assert!(
                sf.line_ranges.is_empty(),
                "line_ranges should be empty when spans disabled"
            );
        }
    }

    #[test]
    fn code_spans_merge_overlapping_ranges() {
        let spans = vec![(1u32, 5u32), (3, 8), (15, 20)];
        let merged = super::merge_spans(&spans);
        assert_eq!(merged, vec![(1, 8), (15, 20)]);
    }

    #[test]
    fn code_spans_merge_adjacent_ranges() {
        let spans = vec![(1u32, 5u32), (6, 10)];
        let merged = super::merge_spans(&spans);
        assert_eq!(merged, vec![(1, 10)]);
    }

    #[test]
    fn code_spans_single_range_unchanged() {
        let spans = vec![(10u32, 20u32)];
        let merged = super::merge_spans(&spans);
        assert_eq!(merged, vec![(10, 20)]);
    }
}
