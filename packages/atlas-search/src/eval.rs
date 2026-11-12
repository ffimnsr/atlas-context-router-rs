//! Retrieval quality evaluation and token-efficiency metrics (Patch R6).
//!
//! Provides `recall_at_k`, `mrr`, `exact_hit_rate`, and token-efficiency
//! metrics for comparing retrieval modes under fixed context budgets.
//!
//! # Usage
//!
//! Build a labeled `Vec<RetrievalCase>` and call [`evaluate`] for each
//! [`RetrievalMode`] of interest.  Use [`hybrid_passes_acceptance`] to decide
//! whether hybrid retrieval meets the quality bar required to enable it by
//! default.
//!
//! # Acceptance thresholds
//!
//! Before enabling hybrid retrieval by default, all quality metrics must meet
//! [`HYBRID_ACCEPTANCE_THRESHOLDS`].  The thresholds are const so they can be
//! checked in CI without a running embedding backend.

use std::collections::HashSet;

use atlas_core::{Result, SearchQuery};
use atlas_store_sqlite::Store;
use serde::{Deserialize, Serialize};

use crate::execute_query_with_embedding;

// ---------------------------------------------------------------------------
// Retrieval mode
// ---------------------------------------------------------------------------

/// Retrieval modes evaluated by this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum RetrievalMode {
    /// Graph expansion only: empty text + `graph_expand=true`.
    /// Useful as an upper-bound signal for graph-reachable targets.
    GraphOnly,
    /// FTS lexical retrieval without graph expansion or vector search.
    LexicalOnly,
    /// Hybrid FTS + vector retrieval with Reciprocal Rank Fusion.
    /// Falls back to FTS-only when no embedding backend is configured.
    Hybrid,
    /// Hybrid retrieval followed by graph expansion hops.
    HybridGraphExpand,
}

impl RetrievalMode {
    /// Short label used in benchmark output and serialized reports.
    pub fn label(self) -> &'static str {
        match self {
            Self::GraphOnly => "graph_only",
            Self::LexicalOnly => "lexical_only",
            Self::Hybrid => "hybrid",
            Self::HybridGraphExpand => "hybrid_graph_expand",
        }
    }

    /// Build a [`SearchQuery`] for this mode from a base text query.
    pub fn build_query(self, text: &str, limit: usize) -> SearchQuery {
        SearchQuery {
            text: if self == Self::GraphOnly {
                String::new()
            } else {
                text.to_owned()
            },
            kind: None,
            language: None,
            include_files: false,
            file_path: None,
            subpath: None,
            exact_dir: None,
            exclude_subpaths: Vec::new(),
            is_test: None,
            limit,
            graph_expand: matches!(self, Self::GraphOnly | Self::HybridGraphExpand),
            graph_max_hops: 1,
            reference_file: None,
            reference_language: None,
            fuzzy_match: false,
            recent_file_boost: false,
            changed_files: Vec::new(),
            hybrid: matches!(self, Self::Hybrid | Self::HybridGraphExpand),
            top_k_fts: 60,
            top_k_vector: 60,
            rrf_k: 60,
            regex_pattern: None,
            repo_ids: Vec::new(),
            triehi_min_chunks: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Labeled test case
// ---------------------------------------------------------------------------

/// Single labeled retrieval case used for offline evaluation.
#[derive(Debug, Clone)]
pub struct RetrievalCase {
    /// Query text (symbol name, identifier, or natural-language phrase).
    pub query: String,
    /// Expected target qualified names (ground truth).
    pub expected_targets: Vec<String>,
}

// ---------------------------------------------------------------------------
// Budget classes
// ---------------------------------------------------------------------------

/// Fixed context budget for quality-under-budget evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BudgetClass {
    /// Small budget: ~2 000 approximate tokens (≈ 8 000 bytes).
    Small,
    /// Medium budget: ~8 000 approximate tokens (≈ 32 000 bytes).
    Medium,
}

impl BudgetClass {
    /// Approximate token limit for this budget class.
    pub fn token_limit(self) -> usize {
        match self {
            Self::Small => 2_000,
            Self::Medium => 8_000,
        }
    }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Retrieval quality and token-efficiency metrics for one mode over a labeled
/// test set.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievalMetrics {
    /// Retrieval mode these metrics were collected for.
    pub mode: RetrievalMode,
    /// Recall@k: fraction of expected targets found in the top-k results,
    /// averaged across all cases in the test set.
    pub recall_at_k: f64,
    /// The k used for recall computation.
    pub recall_k: usize,
    /// Mean Reciprocal Rank: average of `1 / rank` of the first expected
    /// target found in each result list.  0.0 when no target is found.
    pub mrr: f64,
    /// Exact hit rate: fraction of queries where ≥1 expected target appears
    /// in the top-k results.
    pub exact_hit_rate: f64,
    /// Average approximate tokens across all retrieved result payloads
    /// (before budget trimming).
    pub retrieved_tokens_per_query: f64,
    /// Average approximate tokens of the emitted context payload (after
    /// budget trimming when a [`BudgetClass`] is supplied).
    pub emitted_tokens_per_query: f64,
    /// Query executions per task.  Always 1.0 in offline evaluation; should
    /// be populated from session event logs in live evaluation.
    pub tool_calls_per_task: f64,
    /// Average raw payload bytes per query (sum of formatted result strings).
    pub payload_bytes_per_query: f64,
    /// Fraction of top-k result slots occupied by non-target nodes (context
    /// noise).  Lower is better.
    pub context_noise: f64,
    /// Number of cases whose result list was identical to the immediately
    /// preceding case — a proxy for redundant/repeated search calls.
    pub repeated_result_count: usize,
    /// Number of labeled cases evaluated.
    pub case_count: usize,
    /// Approximation of the tokens emitted by the L0 overview view for the
    /// same subject set (VQ1).
    pub l0_tokens: usize,
    /// Approximation of the tokens emitted by the L1 overview view.
    pub l1_tokens: usize,
    /// Approximation of the tokens emitted by the L2 overview view.
    pub l2_tokens: usize,
    /// Vector-candidate count after DSQ scope pre-filtering (VQ1).
    pub scope_candidate_count: usize,
    /// Vector-candidate count for the equivalent unscoped query.
    pub unscoped_candidate_count: usize,
    /// Recall@k of the two-path (primary + auxiliary) memory recall.
    pub aux_recall_at_k: f64,
    /// Recall@k of the primary-only memory recall path.
    pub primary_recall_at_k: f64,
    /// Context escalation rounds actually used for the measured case set.
    pub rounds_used: usize,
    /// Escalation rounds that would be used without experience edges.
    pub rounds_without_experience: usize,
}

impl RetrievalMetrics {
    /// L0/L1/L2 token ratios against the L2 baseline.
    ///
    /// `l2_ratio` is always `1.0`; the ratios are meaningless when no overview
    /// rows were measured, in which case every ratio collapses to `0.0`.
    pub fn token_efficiency(&self) -> TokenEfficiency {
        TokenEfficiency::from_counts(self.l0_tokens, self.l1_tokens, self.l2_tokens)
    }

    /// Fraction of unscoped vector candidates removed by DSQ pre-filtering.
    ///
    /// Returns `0.0` when no unscoped candidate baseline was recorded.
    pub fn dsq_pruning_ratio(&self) -> f64 {
        if self.unscoped_candidate_count == 0 {
            return 0.0;
        }
        let retained = self.scope_candidate_count as f64 / self.unscoped_candidate_count as f64;
        (1.0 - retained).clamp(0.0, 1.0)
    }

    /// Recall delta between primary-only and two-path memory recall.
    pub fn auxiliary_lift(&self) -> f64 {
        self.aux_recall_at_k - self.primary_recall_at_k
    }

    /// Escalation rounds saved by activating experience edges.
    pub fn escalation_round_reduction(&self) -> usize {
        self.rounds_without_experience
            .saturating_sub(self.rounds_used)
    }
}

/// Token ratios of the progressive-disclosure levels against the L2 baseline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TokenEfficiency {
    /// L0 tokens divided by L2 tokens.
    pub l0_ratio: f64,
    /// L1 tokens divided by L2 tokens.
    pub l1_ratio: f64,
    /// Always `1.0` when L2 tokens were measured.
    pub l2_ratio: f64,
}

impl TokenEfficiency {
    /// Derive L0/L1/L2 ratios from absolute token counts.
    ///
    /// Returns all zeros when `l2_tokens` is zero so callers never divide by
    /// an empty baseline.
    pub fn from_counts(l0_tokens: usize, l1_tokens: usize, l2_tokens: usize) -> Self {
        if l2_tokens == 0 {
            return Self {
                l0_ratio: 0.0,
                l1_ratio: 0.0,
                l2_ratio: 0.0,
            };
        }
        let baseline = l2_tokens as f64;
        Self {
            l0_ratio: l0_tokens as f64 / baseline,
            l1_ratio: l1_tokens as f64 / baseline,
            l2_ratio: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Acceptance thresholds
// ---------------------------------------------------------------------------

/// Acceptance thresholds that must all be satisfied before hybrid retrieval
/// is enabled by default.
#[derive(Debug, Clone, Serialize)]
pub struct HybridAcceptanceThresholds {
    /// Minimum recall@5 required.
    pub min_recall_at_5: f64,
    /// Minimum MRR required.
    pub min_mrr: f64,
    /// Minimum exact hit rate required.
    pub min_exact_hit_rate: f64,
    /// Maximum tolerated payload-bytes regression versus the lexical baseline
    /// expressed as a fraction.  E.g. `0.10` means hybrid may emit at most
    /// 10 % more bytes than lexical-only before failing the check.
    pub max_payload_bytes_regression: f64,
}

/// Default acceptance thresholds for enabling hybrid retrieval.
pub const HYBRID_ACCEPTANCE_THRESHOLDS: HybridAcceptanceThresholds = HybridAcceptanceThresholds {
    min_recall_at_5: 0.70,
    min_mrr: 0.50,
    min_exact_hit_rate: 0.80,
    max_payload_bytes_regression: 0.10,
};

// ---------------------------------------------------------------------------
// Token approximation
// ---------------------------------------------------------------------------

/// Approximate token count for a text string.
///
/// Splits on whitespace and common code punctuation.  This is a deterministic
/// local approximation — not tied to any LLM tokenizer — sufficient for
/// relative efficiency comparisons across retrieval modes.
pub fn approx_tokens(text: &str) -> usize {
    text.split(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ';' | ':' | '(' | ')' | '{' | '}' | '[' | ']')
    })
    .filter(|t| !t.is_empty())
    .count()
}

// ---------------------------------------------------------------------------
// Per-case metrics
// ---------------------------------------------------------------------------

/// Recall@k for a single labeled case.
///
/// `results` must be ordered by rank (index 0 = rank 1).
/// Returns 0.0 when `expected` is empty.
pub fn recall_at_k(results: &[&str], expected: &[&str], k: usize) -> f64 {
    if expected.is_empty() {
        return 0.0;
    }
    let expected_set: HashSet<&str> = expected.iter().copied().collect();
    let hits = results
        .iter()
        .take(k)
        .filter(|r| expected_set.contains(*r))
        .count();
    hits as f64 / expected.len() as f64
}

/// Reciprocal rank for a single labeled case.
///
/// Returns `1 / rank` of the first expected target found in `results`, or
/// 0.0 when no expected target appears.
pub fn reciprocal_rank(results: &[&str], expected: &[&str]) -> f64 {
    let expected_set: HashSet<&str> = expected.iter().copied().collect();
    for (i, r) in results.iter().enumerate() {
        if expected_set.contains(*r) {
            return 1.0 / (i + 1) as f64;
        }
    }
    0.0
}

// ---------------------------------------------------------------------------
// Full evaluation
// ---------------------------------------------------------------------------

/// Evaluate a single [`RetrievalMode`] over a labeled test set.
///
/// Each case in `cases` is run independently.  `limit` controls the result
/// cut-off returned by the search layer; `recall_k ≤ limit` is the k used for
/// recall and hit-rate computation.  When `budget` is `Some`, emitted-token
/// counts are trimmed to that budget.
pub fn evaluate(
    store: &Store,
    cases: &[RetrievalCase],
    mode: RetrievalMode,
    limit: usize,
    recall_k: usize,
    budget: Option<BudgetClass>,
) -> Result<RetrievalMetrics> {
    evaluate_with_embedding(store, cases, mode, limit, recall_k, budget, None)
}

/// Evaluate a single [`RetrievalMode`] using an optional embedding backend.
///
/// Pass an embedding configuration when evaluating real hybrid retrieval. Passing
/// `None` preserves [`evaluate`]'s FTS-fallback behavior for offline checks.
pub fn evaluate_with_embedding(
    store: &Store,
    cases: &[RetrievalCase],
    mode: RetrievalMode,
    limit: usize,
    recall_k: usize,
    budget: Option<BudgetClass>,
    embed_cfg: Option<&crate::embed::EmbeddingConfig>,
) -> Result<RetrievalMetrics> {
    // GraphOnly uses graph-expansion path; other modes use standard FTS path.
    let semantic = matches!(
        mode,
        RetrievalMode::GraphOnly | RetrievalMode::HybridGraphExpand
    );

    let mut sum_recall = 0.0_f64;
    let mut sum_rr = 0.0_f64;
    let mut hit_count = 0_usize;
    let mut sum_retrieved_tokens = 0.0_f64;
    let mut sum_emitted_tokens = 0.0_f64;
    let mut sum_payload_bytes = 0.0_f64;
    let mut sum_noise = 0.0_f64;
    let mut repeated_result_count = 0_usize;
    let mut prev_result_qns: Option<Vec<String>> = None;

    for case in cases {
        let query = mode.build_query(&case.query, limit);
        let results =
            execute_query_with_embedding(store, &query, semantic, embed_cfg).unwrap_or_default();

        let ranked_qns: Vec<String> = results
            .iter()
            .map(|r| r.node.qualified_name.clone())
            .collect();
        let ranked_refs: Vec<&str> = ranked_qns.iter().map(String::as_str).collect();
        let expected_refs: Vec<&str> = case.expected_targets.iter().map(String::as_str).collect();

        let k = recall_k.min(ranked_refs.len().max(1));
        sum_recall += recall_at_k(&ranked_refs, &expected_refs, k);
        sum_rr += reciprocal_rank(&ranked_refs, &expected_refs);

        if ranked_refs
            .iter()
            .take(k)
            .any(|qn| expected_refs.contains(qn))
        {
            hit_count += 1;
        }

        // Build a representative payload string from all returned nodes.
        let payload: String = results
            .iter()
            .map(|r| {
                format!(
                    "{} {} {}",
                    r.node.qualified_name,
                    r.node.file_path,
                    r.node.kind.as_str()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let payload_bytes = payload.len();
        let retrieved_tokens = approx_tokens(&payload);

        let emitted_tokens = if let Some(budget_class) = budget {
            let token_limit = budget_class.token_limit();
            let mut emitted = 0_usize;
            for r in &results {
                let item_tokens = approx_tokens(&format!(
                    "{} {} {}",
                    r.node.qualified_name,
                    r.node.file_path,
                    r.node.kind.as_str()
                ));
                if emitted + item_tokens > token_limit {
                    break;
                }
                emitted += item_tokens;
            }
            emitted
        } else {
            retrieved_tokens
        };

        sum_retrieved_tokens += retrieved_tokens as f64;
        sum_emitted_tokens += emitted_tokens as f64;
        sum_payload_bytes += payload_bytes as f64;

        // Context noise: fraction of top-k result slots not in expected set.
        let expected_set: HashSet<&str> = expected_refs.iter().copied().collect();
        let noise_count = ranked_refs
            .iter()
            .take(k)
            .filter(|qn| !expected_set.contains(*qn))
            .count();
        sum_noise += noise_count as f64 / k.max(1) as f64;

        // Repeated-result proxy: identical ranked list to previous case.
        if let Some(ref prev) = prev_result_qns
            && *prev == ranked_qns
        {
            repeated_result_count += 1;
        }
        prev_result_qns = Some(ranked_qns);
    }

    let n = cases.len().max(1) as f64;
    let recall = sum_recall / n;
    Ok(RetrievalMetrics {
        mode,
        recall_at_k: recall,
        recall_k,
        mrr: sum_rr / n,
        exact_hit_rate: hit_count as f64 / n,
        retrieved_tokens_per_query: sum_retrieved_tokens / n,
        emitted_tokens_per_query: sum_emitted_tokens / n,
        tool_calls_per_task: 1.0,
        payload_bytes_per_query: sum_payload_bytes / n,
        context_noise: sum_noise / n,
        repeated_result_count,
        case_count: cases.len(),
        // Graph search measures no overview levels, scope candidates, or memory
        // paths; the RAG corpus populates these through `rag_eval`.
        l0_tokens: 0,
        l1_tokens: 0,
        l2_tokens: 0,
        scope_candidate_count: 0,
        unscoped_candidate_count: 0,
        primary_recall_at_k: recall,
        aux_recall_at_k: recall,
        rounds_used: 1,
        rounds_without_experience: 1,
    })
}

// ---------------------------------------------------------------------------
// Acceptance check
// ---------------------------------------------------------------------------

/// Returns `true` when `hybrid` meets all [`HYBRID_ACCEPTANCE_THRESHOLDS`]
/// relative to `lexical_baseline`.
///
/// Call this after running [`evaluate`] for both [`RetrievalMode::Hybrid`] and
/// [`RetrievalMode::LexicalOnly`] over the same labeled test set.
pub fn hybrid_passes_acceptance(
    hybrid: &RetrievalMetrics,
    lexical_baseline: &RetrievalMetrics,
) -> bool {
    let t = &HYBRID_ACCEPTANCE_THRESHOLDS;
    let baseline_bytes = lexical_baseline.payload_bytes_per_query.max(1.0);
    let payload_regression = (hybrid.payload_bytes_per_query - baseline_bytes) / baseline_bytes;

    hybrid.recall_at_k >= t.min_recall_at_5
        && hybrid.mrr >= t.min_mrr
        && hybrid.exact_hit_rate >= t.min_exact_hit_rate
        && payload_regression <= t.max_payload_bytes_regression
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
    };

    use atlas_core::{Node, NodeId, NodeKind};

    // recall_at_k ---------------------------------------------------------

    #[test]
    fn recall_at_k_all_present() {
        assert_eq!(recall_at_k(&["a", "b", "c"], &["a", "b"], 3), 1.0);
    }

    #[test]
    fn recall_at_k_partial() {
        assert_eq!(recall_at_k(&["a", "x", "y"], &["a", "b"], 3), 0.5);
    }

    #[test]
    fn recall_at_k_none_in_top_k() {
        // "b" is at rank 3 but k=2, so it does not count.
        assert_eq!(recall_at_k(&["x", "y", "b"], &["a", "b"], 2), 0.0);
    }

    #[test]
    fn recall_at_k_empty_expected_returns_zero() {
        assert_eq!(recall_at_k(&["a", "b"], &[], 2), 0.0);
    }

    // reciprocal_rank -----------------------------------------------------

    #[test]
    fn reciprocal_rank_first_hit() {
        let rr = reciprocal_rank(&["a", "b", "c"], &["a"]);
        assert!((rr - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reciprocal_rank_second_hit() {
        let rr = reciprocal_rank(&["x", "a", "c"], &["a"]);
        assert!((rr - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn reciprocal_rank_no_hit() {
        assert_eq!(reciprocal_rank(&["x", "y", "z"], &["a"]), 0.0);
    }

    // approx_tokens -------------------------------------------------------

    #[test]
    fn approx_tokens_simple() {
        assert_eq!(approx_tokens("hello world"), 2);
    }

    #[test]
    fn approx_tokens_code_like() {
        // Splits on ':' and '(' ')' — at minimum yields "foo", "bar", "baz", "fn"
        let count = approx_tokens("foo::bar::baz fn()");
        assert!(count >= 2, "expected ≥2 tokens, got {count}");
    }

    #[test]
    fn approx_tokens_empty() {
        assert_eq!(approx_tokens(""), 0);
    }

    // hybrid_passes_acceptance -------------------------------------------

    #[test]
    fn hybrid_passes_acceptance_all_met() {
        let hybrid = make_metrics(RetrievalMode::Hybrid, 0.75, 0.55, 0.85, 1000.0);
        let baseline = make_metrics(RetrievalMode::LexicalOnly, 0.60, 0.45, 0.70, 950.0);
        assert!(hybrid_passes_acceptance(&hybrid, &baseline));
    }

    #[test]
    fn hybrid_fails_recall() {
        // recall_at_k below threshold (0.65 < 0.70)
        let hybrid = make_metrics(RetrievalMode::Hybrid, 0.65, 0.55, 0.85, 1000.0);
        let baseline = make_metrics(RetrievalMode::LexicalOnly, 0.60, 0.45, 0.70, 950.0);
        assert!(!hybrid_passes_acceptance(&hybrid, &baseline));
    }

    #[test]
    fn hybrid_fails_mrr() {
        // mrr below threshold (0.45 < 0.50)
        let hybrid = make_metrics(RetrievalMode::Hybrid, 0.75, 0.45, 0.85, 1000.0);
        let baseline = make_metrics(RetrievalMode::LexicalOnly, 0.60, 0.45, 0.70, 950.0);
        assert!(!hybrid_passes_acceptance(&hybrid, &baseline));
    }

    #[test]
    fn hybrid_fails_hit_rate() {
        // exact_hit_rate below threshold (0.75 < 0.80)
        let hybrid = make_metrics(RetrievalMode::Hybrid, 0.75, 0.55, 0.75, 1000.0);
        let baseline = make_metrics(RetrievalMode::LexicalOnly, 0.60, 0.45, 0.70, 950.0);
        assert!(!hybrid_passes_acceptance(&hybrid, &baseline));
    }

    #[test]
    fn hybrid_fails_payload_regression() {
        // 20 % payload regression exceeds 10 % threshold
        let hybrid = make_metrics(RetrievalMode::Hybrid, 0.80, 0.60, 0.90, 1200.0);
        let baseline = make_metrics(RetrievalMode::LexicalOnly, 0.60, 0.45, 0.70, 1000.0);
        assert!(!hybrid_passes_acceptance(&hybrid, &baseline));
    }

    #[test]
    fn configured_hybrid_evaluation_passes_acceptance() {
        let mut store = Store::open(":memory:").expect("open in-memory store");
        store.migrate().expect("migrate store");

        let qn = "src/search.rs::fn::lexical_anchor";
        store
            .replace_file_graph_for_repo(
                "repo_test",
                "src/search.rs",
                "hash",
                Some("rust"),
                None,
                &[Node {
                    id: NodeId::UNSET,
                    kind: NodeKind::Function,
                    name: "lexical_anchor".to_owned(),
                    qualified_name: qn.to_owned(),
                    file_path: "src/search.rs".to_owned(),
                    line_start: 1,
                    line_end: 2,
                    language: "rust".to_owned(),
                    parent_name: None,
                    params: Some("()".to_owned()),
                    return_type: None,
                    modifiers: None,
                    is_test: false,
                    file_hash: "hash".to_owned(),
                    extra_json: serde_json::Value::Null,
                    repo_provenance: None,
                }],
                &[],
            )
            .expect("seed graph");
        store
            .upsert_chunk("repo_test", qn, 0, "lexical anchor")
            .expect("insert chunk");
        let chunks = store
            .chunks_missing_embeddings(1)
            .expect("load missing chunk");
        store
            .set_chunk_embedding(chunks[0].0, &[1.0, 0.0])
            .expect("store embedding");

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock embedding server");
        let address = listener.local_addr().expect("read server address");
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_in_server = Arc::clone(&requests);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept embedding request");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).expect("read embedding request");
            requests_in_server.fetch_add(1, Ordering::SeqCst);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 26\r\nConnection: close\r\n\r\n{\"embeddings\":[[1.0,0.0]]}",
                )
                .expect("write embedding response");
        });
        let config =
            crate::embed::EmbeddingConfig::new(format!("http://{address}"), "test-model", 1, 0, 0);
        let cases = vec![RetrievalCase {
            query: "lexical_anchor".to_owned(),
            expected_targets: vec![qn.to_owned()],
        }];

        let baseline = evaluate(&store, &cases, RetrievalMode::LexicalOnly, 5, 5, None)
            .expect("evaluate lexical baseline");
        let hybrid = evaluate_with_embedding(
            &store,
            &cases,
            RetrievalMode::Hybrid,
            5,
            5,
            None,
            Some(&config),
        )
        .expect("evaluate configured hybrid");
        server.join().expect("join mock embedding server");

        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(hybrid.mode, RetrievalMode::Hybrid);
        assert!(hybrid_passes_acceptance(&hybrid, &baseline));
    }

    #[test]
    fn token_efficiency_uses_l2_baseline() {
        let mut metrics = make_metrics(RetrievalMode::LexicalOnly, 1.0, 1.0, 1.0, 10.0);
        metrics.l0_tokens = 20;
        metrics.l1_tokens = 100;
        metrics.l2_tokens = 200;
        let efficiency = metrics.token_efficiency();
        assert!((efficiency.l0_ratio - 0.10).abs() < f64::EPSILON);
        assert!((efficiency.l1_ratio - 0.50).abs() < f64::EPSILON);
        assert!((efficiency.l2_ratio - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn token_efficiency_without_overviews_is_zero() {
        let metrics = make_metrics(RetrievalMode::LexicalOnly, 1.0, 1.0, 1.0, 10.0);
        let efficiency = metrics.token_efficiency();
        assert_eq!(efficiency.l0_ratio, 0.0);
        assert_eq!(efficiency.l1_ratio, 0.0);
        assert_eq!(efficiency.l2_ratio, 0.0);
    }

    #[test]
    fn dsq_pruning_ratio_reports_removed_candidates() {
        let mut metrics = make_metrics(RetrievalMode::LexicalOnly, 1.0, 1.0, 1.0, 10.0);
        metrics.scope_candidate_count = 10;
        metrics.unscoped_candidate_count = 100;
        assert!((metrics.dsq_pruning_ratio() - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn dsq_pruning_ratio_without_candidates_is_zero() {
        let metrics = make_metrics(RetrievalMode::LexicalOnly, 1.0, 1.0, 1.0, 10.0);
        assert_eq!(metrics.dsq_pruning_ratio(), 0.0);
    }

    #[test]
    fn auxiliary_lift_is_two_path_minus_primary() {
        let mut metrics = make_metrics(RetrievalMode::LexicalOnly, 1.0, 1.0, 1.0, 10.0);
        metrics.primary_recall_at_k = 0.75;
        metrics.aux_recall_at_k = 1.0;
        assert!((metrics.auxiliary_lift() - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn escalation_round_reduction_is_baseline_minus_used() {
        let mut metrics = make_metrics(RetrievalMode::LexicalOnly, 1.0, 1.0, 1.0, 10.0);
        metrics.rounds_without_experience = 2;
        metrics.rounds_used = 1;
        assert_eq!(metrics.escalation_round_reduction(), 1);
    }

    fn make_metrics(
        mode: RetrievalMode,
        recall: f64,
        mrr: f64,
        hit_rate: f64,
        payload_bytes: f64,
    ) -> RetrievalMetrics {
        RetrievalMetrics {
            mode,
            recall_at_k: recall,
            recall_k: 5,
            mrr,
            exact_hit_rate: hit_rate,
            retrieved_tokens_per_query: 100.0,
            emitted_tokens_per_query: 80.0,
            tool_calls_per_task: 1.0,
            payload_bytes_per_query: payload_bytes,
            context_noise: 0.2,
            repeated_result_count: 0,
            case_count: 10,
            l0_tokens: 0,
            l1_tokens: 0,
            l2_tokens: 0,
            scope_candidate_count: 0,
            unscoped_candidate_count: 0,
            primary_recall_at_k: recall,
            aux_recall_at_k: recall,
            rounds_used: 1,
            rounds_without_experience: 1,
        }
    }
}
