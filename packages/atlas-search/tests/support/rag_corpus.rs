//! Deterministic RAG evaluation fixture corpus (VQ1).
//!
//! Builds fixed code, memory, memoir, overview, and experience-edge rows so the
//! four VQ1 measurement families can run without a live repository, embedding
//! backend, or LLM.
//!
//! Every measurement is a count or ratio: the same corpus always produces the
//! same [`RagEvaluationReport`], on any host, in any order.

use atlas_core::{AtlasError, Edge, Node, NodeId, NodeKind};
use atlas_search::eval::{approx_tokens, recall_at_k};
use atlas_search::rag_eval::{
    DsqPruning, DsqScopeKind, RAG_AUX_RECALL_K, RagEvaluationReport, TokenEfficiency,
};
use atlas_session::{
    MemoryImportance, MemoryListFilter, MemoryRecallPolicy, MemoryScope, MemoryViewer,
    NewExperienceEdge, NewMemoirConcept, NewMemoirGraph, NewMemoirRelation, NewMemory,
    OverviewFreshness, OverviewLevel, OverviewRecord, OverviewSourceKind, OverviewSubjectKind,
    SessionStore, render_level,
};
use atlas_store_sqlite::{Store, VectorScope};

/// Indexed repo id used by the code-store fixture.
pub const RAG_EVAL_REPO_ID: &str = "rag_eval_repo";
/// Repo root shared by the session-memory fixture.
pub const RAG_EVAL_REPO_ROOT: &str = "/workspace/atlas-rag-eval";
/// Corpus identifier written into the JSON report.
pub const RAG_EVAL_CORPUS: &str = "rag-eval-fixture-v1";

/// Directory fan-out of the code fixture (`src/dir{0..10}`).
const CODE_DIRS: usize = 10;
/// Files per directory in the code fixture.
const CODE_FILES: usize = 10;

/// Baseline escalation rounds when a fresh evidence gap exists.
const ROUNDS_WITHOUT_EXPERIENCE: usize = 2;
/// Escalation rounds when an experience edge short-circuits evidence lookup.
const ROUNDS_WITH_EXPERIENCE: usize = 1;

struct MemoryCase {
    query: String,
    expected_ids: Vec<String>,
}

struct RagCorpus {
    code: Store,
    session: SessionStore,
    overviews: Vec<OverviewRecord>,
    memory_cases: Vec<MemoryCase>,
    shortcut_queries: Vec<String>,
}

/// Build the fixture corpus, measure all four families, and return the report.
pub fn evaluate_rag_corpus() -> atlas_core::Result<RagEvaluationReport> {
    let corpus = RagCorpus::build()?;
    corpus.evaluate()
}

impl RagCorpus {
    fn build() -> atlas_core::Result<Self> {
        let code = build_code_corpus()?;
        let mut session = SessionStore::open(":memory:")?;
        let memory_cases = seed_memories(&mut session)?;
        seed_memoir(&mut session)?;
        seed_experience_edges(&mut session)?;
        let overviews = overview_fixture();
        let shortcut_queries = vec![
            "deployment rollback".to_owned(),
            "cache invalidation".to_owned(),
        ];

        Ok(Self {
            code,
            session,
            overviews,
            memory_cases,
            shortcut_queries,
        })
    }

    fn evaluate(&self) -> atlas_core::Result<RagEvaluationReport> {
        let (l0_tokens, l1_tokens, l2_tokens) = overview_tokens(&self.overviews);
        let token_efficiency = TokenEfficiency::from_counts(l0_tokens, l1_tokens, l2_tokens);
        let dsq_pruning = dsq_pruning(&self.code)?;
        let (primary_recall_at_k, aux_recall_at_k) =
            memory_recall(&self.session, &self.memory_cases)?;

        let rounds_used = if self
            .shortcut_queries
            .iter()
            .all(|query| self.edge_activates(query))
        {
            ROUNDS_WITH_EXPERIENCE
        } else {
            ROUNDS_WITHOUT_EXPERIENCE
        };

        Ok(RagEvaluationReport::new(
            RAG_EVAL_CORPUS,
            RAG_AUX_RECALL_K,
            token_efficiency,
            l0_tokens,
            l1_tokens,
            l2_tokens,
            dsq_pruning,
            primary_recall_at_k,
            aux_recall_at_k,
            ROUNDS_WITHOUT_EXPERIENCE,
            rounds_used,
        ))
    }

    fn edge_activates(&self, query: &str) -> bool {
        self.session
            .activate_experience_edges(RAG_EVAL_REPO_ROOT, query, "review", 3)
            .map(|edges| !edges.is_empty())
            .unwrap_or(false)
    }
}

fn build_code_corpus() -> atlas_core::Result<Store> {
    let mut store = Store::open(":memory:")?;
    store.migrate()?;

    for dir in 0..CODE_DIRS {
        for file in 0..CODE_FILES {
            let path = format!("src/dir{dir}/file{file}.rs");
            let qn = format!("{path}::fn::scoped_symbol");
            let nodes = vec![make_fn_node(&path, &qn)];
            store.replace_file_graph_for_repo(
                RAG_EVAL_REPO_ID,
                &path,
                "rag-eval-hash",
                Some("rust"),
                None,
                &nodes,
                &[] as &[Edge],
            )?;
            store.upsert_chunk(RAG_EVAL_REPO_ID, &qn, 0, "scoped chunk text")?;
            let chunk_id = store
                .chunks_missing_embeddings(1)?
                .first()
                .map(|row| row.0)
                .ok_or_else(|| AtlasError::Other("missing chunk id".to_owned()))?;
            store.set_chunk_embedding(chunk_id, &[1.0, 0.0])?;
        }
    }

    Ok(store)
}

fn make_fn_node(file_path: &str, qualified_name: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "scoped_symbol".to_owned(),
        qualified_name: qualified_name.to_owned(),
        file_path: file_path.to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: Some("()".to_owned()),
        modifiers: None,
        is_test: false,
        file_hash: "rag-eval-hash".to_owned(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn dsq_pruning(store: &Store) -> atlas_core::Result<Vec<DsqPruning>> {
    let unscoped = VectorScope {
        repo_ids: vec![RAG_EVAL_REPO_ID.to_owned()],
        ..Default::default()
    };
    let (_, unscoped_candidate_count) =
        store.nodes_by_vector_similarity_with_stats(&[1.0, 0.0], &unscoped, 10)?;

    let predicates = [
        (
            DsqScopeKind::Recursive,
            VectorScope {
                subpath_prefix: Some("src/dir3".to_owned()),
                repo_ids: vec![RAG_EVAL_REPO_ID.to_owned()],
                ..Default::default()
            },
        ),
        (
            DsqScopeKind::ExactDir,
            VectorScope {
                exact_dir: Some("src/dir3".to_owned()),
                repo_ids: vec![RAG_EVAL_REPO_ID.to_owned()],
                ..Default::default()
            },
        ),
        (
            DsqScopeKind::Exclusion,
            VectorScope {
                subpath_prefix: Some("src".to_owned()),
                exclude_prefixes: (0..CODE_DIRS - 1)
                    .map(|dir| format!("src/dir{dir}"))
                    .collect(),
                repo_ids: vec![RAG_EVAL_REPO_ID.to_owned()],
                ..Default::default()
            },
        ),
    ];

    let mut rows = Vec::with_capacity(predicates.len());
    for (scope, vector_scope) in predicates {
        let (_, scoped_candidate_count) =
            store.nodes_by_vector_similarity_with_stats(&[1.0, 0.0], &vector_scope, 10)?;
        rows.push(DsqPruning {
            scope,
            scoped_candidate_count,
            unscoped_candidate_count,
        });
    }
    Ok(rows)
}

fn seed_memories(session: &mut SessionStore) -> atlas_core::Result<Vec<MemoryCase>> {
    let lexical_queries = [
        "alpha glide",
        "bravo ledger",
        "charlie harbor",
        "delta meadow",
        "echo lantern",
        "foxtrot canyon",
        "golf tundra",
        "hotel orchard",
    ];

    let mut cases = Vec::with_capacity(lexical_queries.len() + 2);
    for (index, query) in lexical_queries.iter().enumerate() {
        let memory = session.store_memory(&NewMemory {
            repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
            session_id: None,
            frontend: None,
            scope: MemoryScope::Project,
            topic: (*query).to_owned(),
            title: format!("fixture {index}"),
            body: format!("{query} rollout notes"),
            importance: MemoryImportance::Normal,
            source_id: Some(format!("rag-src-lexical-{index}")),
            metadata: serde_json::json!({}),
        })?;
        cases.push(MemoryCase {
            query: (*query).to_owned(),
            expected_ids: vec![memory.id],
        });
    }

    // Auxiliary-only facts: bodies never contain the query tokens, so the
    // primary lexical path cannot find them. Memoir concepts carry the link.
    let direct = session.store_memory(&NewMemory {
        repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: "release restore".to_owned(),
        title: "release restore steps".to_owned(),
        body: "restore the previous release artifact".to_owned(),
        importance: MemoryImportance::High,
        source_id: Some("rag-src-deploy".to_owned()),
        metadata: serde_json::json!({}),
    })?;
    let neighbour = session.store_memory(&NewMemory {
        repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: "freeze window".to_owned(),
        title: "freeze window rule".to_owned(),
        body: "do not ship during the freeze window".to_owned(),
        importance: MemoryImportance::Normal,
        source_id: Some("rag-src-freeze".to_owned()),
        metadata: serde_json::json!({}),
    })?;
    let cache = session.store_memory(&NewMemory {
        repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: "freshness routine".to_owned(),
        title: "freshness routine steps".to_owned(),
        body: "evict stale entries before serving".to_owned(),
        importance: MemoryImportance::Normal,
        source_id: Some("rag-src-cache".to_owned()),
        metadata: serde_json::json!({}),
    })?;

    cases.push(MemoryCase {
        query: "deployment rollback".to_owned(),
        expected_ids: vec![direct.id, neighbour.id],
    });
    cases.push(MemoryCase {
        query: "cache invalidation".to_owned(),
        expected_ids: vec![cache.id],
    });

    Ok(cases)
}

fn seed_memoir(session: &mut SessionStore) -> atlas_core::Result<()> {
    let graph = "rag-eval";
    session.create_memoir_graph(&NewMemoirGraph {
        repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
        scope: MemoryScope::Project,
        name: graph.to_owned(),
        description: "VQ1 deterministic memoir fixture".to_owned(),
        metadata: serde_json::json!({}),
    })?;

    for (name, description, source_id) in [
        (
            "Deployment Rollback",
            "restore a release after a failed rollout",
            "rag-src-deploy",
        ),
        ("Release Freeze", "shipping freeze window", "rag-src-freeze"),
        (
            "Cache Invalidation",
            "eviction policy for stale entries",
            "rag-src-cache",
        ),
    ] {
        session.add_memoir_concept(&NewMemoirConcept {
            repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
            scope: MemoryScope::Project,
            graph: graph.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            kind: "concept".to_owned(),
            source_id: Some(source_id.to_owned()),
            metadata: serde_json::json!({}),
        })?;
    }

    // One hop keeps the auxiliary BFS path exercised for the deploy case.
    session.link_memoir_concepts(&NewMemoirRelation {
        repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
        scope: MemoryScope::Project,
        graph: graph.to_owned(),
        source: "Deployment Rollback".to_owned(),
        target: "Release Freeze".to_owned(),
        relation: "related_to".to_owned(),
        confidence: 0.9,
        source_id: None,
        metadata: serde_json::json!({}),
        create_missing: false,
    })?;

    Ok(())
}

fn seed_experience_edges(session: &mut SessionStore) -> atlas_core::Result<()> {
    for (query, target) in [
        (
            "deployment rollback",
            "atlas://repo/rag_eval_repo/symbol/deployment_rollback",
        ),
        (
            "cache invalidation",
            "atlas://repo/rag_eval_repo/symbol/cache_invalidation",
        ),
    ] {
        session.record_experience_edge(&NewExperienceEdge {
            repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
            source_uri: "atlas://repo/rag_eval_repo/file/src/lib.rs".to_owned(),
            target_uri: target.to_owned(),
            query_text: query.to_owned(),
            trace_summary: "deterministic VQ1 shortcut".to_owned(),
            commit_sha: "rag-eval-commit".to_owned(),
            source_file_hash: Some("rag-eval-hash".to_owned()),
            target_file_hash: Some("rag-eval-hash".to_owned()),
            confidence: 0.9,
            query_embedding: None,
        })?;
    }
    Ok(())
}

fn memory_recall(session: &SessionStore, cases: &[MemoryCase]) -> atlas_core::Result<(f64, f64)> {
    let viewer = MemoryViewer {
        frontend: "cli".to_owned(),
        session_id: "rag-eval-session".to_owned(),
    };
    let filter = MemoryListFilter::default();
    let policy = MemoryRecallPolicy::default();

    let mut primary_sum = 0.0_f64;
    let mut two_path_sum = 0.0_f64;
    for case in cases {
        let hits = session.recall_memories_with_policy(
            RAG_EVAL_REPO_ROOT,
            &case.query,
            &filter,
            false,
            &viewer,
            RAG_AUX_RECALL_K,
            &policy,
        )?;

        let expected: Vec<&str> = case.expected_ids.iter().map(String::as_str).collect();
        let primary_ids: Vec<&str> = hits
            .iter()
            .filter(|hit| hit.ranking.recall_path == "primary")
            .map(|hit| hit.memory.id.as_str())
            .collect();
        let all_ids: Vec<&str> = hits.iter().map(|hit| hit.memory.id.as_str()).collect();

        primary_sum += recall_at_k(&primary_ids, &expected, RAG_AUX_RECALL_K);
        two_path_sum += recall_at_k(&all_ids, &expected, RAG_AUX_RECALL_K);
    }

    let n = cases.len().max(1) as f64;
    Ok((primary_sum / n, two_path_sum / n))
}

fn overview_fixture() -> Vec<OverviewRecord> {
    (0..3)
        .map(|index| OverviewRecord {
            id: format!("rag-overview-{index}"),
            repo_root: RAG_EVAL_REPO_ROOT.to_owned(),
            subject_id: format!("rag-subject-{index}"),
            subject_kind: OverviewSubjectKind::File,
            package_name: Some("atlas-rag-eval".to_owned()),
            module_path: Some(format!("src/module_{index}")),
            canonical_file_path: Some(format!("src/module_{index}.rs")),
            qualified_name: None,
            node_kind: Some("file".to_owned()),
            line_start: Some(1),
            line_end: Some(240),
            content_hash: Some("rag-eval-hash".to_owned()),
            title: format!("Module {index}"),
            summary: "Coordinates request routing, validation, and persistence for one module."
                .to_owned(),
            description_preview: "This module owns the request lifecycle for its feature slice, \
                including input validation, retry accounting, persistence boundaries, and the \
                deterministic error mapping shared by every handler in the module."
                .to_owned(),
            responsibilities: vec![
                "validate inbound requests before any state mutation".to_owned(),
                "route validated work to the correct service function".to_owned(),
                "persist results through the repository boundary".to_owned(),
                "map failures into stable error codes".to_owned(),
                "record retry accounting for transient failures".to_owned(),
                "emit deterministic diagnostics for operators".to_owned(),
                "propagate cancellation signals to every awaited task".to_owned(),
                "keep writes inside one transaction boundary".to_owned(),
            ],
            flow: vec![
                "receive the request payload and normalize identifiers".to_owned(),
                "validate shape, scope, and permission constraints".to_owned(),
                "delegate to the service layer with a bounded retry budget".to_owned(),
                "persist the outcome and return the canonical response envelope".to_owned(),
            ],
            inputs: vec![
                "canonical request identifiers".to_owned(),
                "validated request payload".to_owned(),
                "caller visibility scope".to_owned(),
            ],
            outputs: vec![
                "canonical response envelope".to_owned(),
                "persisted state change".to_owned(),
                "structured diagnostics record".to_owned(),
            ],
            invariants: vec![
                "state mutations happen only after validation succeeds".to_owned(),
                "retry budgets never exceed the configured ceiling".to_owned(),
                "error codes remain stable across releases".to_owned(),
            ],
            gotchas: vec![
                "visibility checks must run before any cache lookup".to_owned(),
                "retry accounting resets on a new request identifier".to_owned(),
                "diagnostics never include request payload bodies".to_owned(),
                "a rolled back transaction still releases its connection".to_owned(),
                "cancellation after commit returns the persisted outcome".to_owned(),
            ],
            examples: Vec::new(),
            tags: vec!["rag-eval".to_owned(), "fixture".to_owned()],
            source_kind: OverviewSourceKind::Manual,
            analyzer_name: Some("rag-eval-fixture".to_owned()),
            analyzer_version: Some("1".to_owned()),
            model_name: None,
            commit_sha: "rag-eval-commit".to_owned(),
            graph_last_indexed_at: Some("2026-01-01T00:00:00Z".to_owned()),
            freshness: OverviewFreshness::Fresh,
            confidence: 0.95,
            source_id: Some(format!("rag-overview-source-{index}")),
            supersedes_id: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:00Z".to_owned(),
            metadata: serde_json::json!({}),
        })
        .collect()
}

fn overview_tokens(records: &[OverviewRecord]) -> (usize, usize, usize) {
    let count = |level: OverviewLevel| {
        records
            .iter()
            .map(|record| {
                let view = render_level(record, level);
                let rendered = serde_json::to_string(&view).unwrap_or_default();
                approx_tokens(&rendered)
            })
            .sum()
    };
    (
        count(OverviewLevel::L0),
        count(OverviewLevel::L1),
        count(OverviewLevel::L2),
    )
}
