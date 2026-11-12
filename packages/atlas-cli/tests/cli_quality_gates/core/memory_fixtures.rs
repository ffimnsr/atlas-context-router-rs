//! ICM-H3 — reusable memory-layer fixtures.
//!
//! One call seeds every memory-layer shape the roadmap requires for snapshot
//! and parity coverage: a critical decision memory, a low-priority stale memory,
//! dead-code false-positive feedback, a memoir dependency graph, overview
//! records, a saved artifact referenced by `source_id`, and a frontend-private
//! memory. Fixtures write only to the session and content stores.

use super::*;
use atlas_adapters::derive_session_db_path;
use atlas_contentstore::{ContentStore, SourceMeta};
use atlas_session::{
    MemoryImportance, MemoryScope, NewFeedback, NewMemoirConcept, NewMemoirGraph,
    NewMemoirRelation, NewMemory, NewOverviewRecord, OverviewFreshness, OverviewSourceKind,
    OverviewSubject, OverviewSubjectKind, SessionStore,
};

/// Ids and paths produced by [`seed_memory_layer`].
///
/// Not every snapshot uses every id; keeping the full set makes the fixture a
/// shared corpus for parity and leakage assertions.
#[allow(dead_code)]
pub(super) struct MemoryLayerFixture {
    pub repo_root: String,
    pub critical_memory_id: String,
    pub stale_memory_id: String,
    pub private_memory_id: String,
    pub feedback_id: String,
    pub overview_id: String,
    pub overview_subject_id: String,
    pub artifact_source_id: String,
}

fn session_store(repo_root: &Path) -> SessionStore {
    let graph_db = repo_root.join(".atlas").join("worldtree.db");
    let session_db = derive_session_db_path(graph_db.to_str().expect("graph db path"));
    SessionStore::open(&session_db).expect("open session store")
}

/// Seed all memory-layer fixtures into `repo_root` and return their ids.
pub(super) fn seed_memory_layer(repo_root: &Path) -> MemoryLayerFixture {
    let repo = canonical_path(repo_root).to_string_lossy().into_owned();
    let mut store = session_store(repo_root);

    // 1. Critical decision memory.
    let critical = store
        .store_memory(&NewMemory {
            repo_root: repo.clone(),
            session_id: None,
            frontend: None,
            scope: MemoryScope::Project,
            topic: "decisions".to_owned(),
            title: "memory recall policy".to_owned(),
            body: "Decision: keep memory recall lexical-first and make embeddings opt-in."
                .to_owned(),
            importance: MemoryImportance::Critical,
            source_id: None,
            metadata: json!({ "kind": "decision" }),
        })
        .expect("seed critical memory");

    // 2. Low-priority stale memory (backdated past the low retention window).
    let stale = store
        .store_memory(&NewMemory {
            repo_root: repo.clone(),
            session_id: None,
            frontend: None,
            scope: MemoryScope::Project,
            topic: "cleanup".to_owned(),
            title: "old probe notes".to_owned(),
            body: "Temporary probe notes from an expired investigation.".to_owned(),
            importance: MemoryImportance::Low,
            source_id: None,
            metadata: json!({}),
        })
        .expect("seed stale memory");
    backdate_memory(repo_root, &stale.id, 400);

    // 3. Frontend-private memory that must never leak to other frontends.
    let private = store
        .store_memory(&NewMemory {
            repo_root: repo.clone(),
            session_id: None,
            frontend: Some("codex".to_owned()),
            scope: MemoryScope::Frontend,
            topic: "preferences".to_owned(),
            title: "codex private preference".to_owned(),
            body: "codex-only preference: keep output terse.".to_owned(),
            importance: MemoryImportance::Normal,
            source_id: None,
            metadata: json!({}),
        })
        .expect("seed private memory");

    // 4. Dead-code false-positive feedback.
    let feedback = store
        .store_feedback(&NewFeedback {
            repo_root: repo.clone(),
            session_id: None,
            tool_name: "analyze_dead_code".to_owned(),
            analysis_kind: "dead_code".to_owned(),
            predicted: "helper is dead code".to_owned(),
            actual: "helper is called by greet_twice".to_owned(),
            correction: "helper stays; it is reachable from greet_twice".to_owned(),
            related_symbol: Some("src/lib.rs::fn::helper".to_owned()),
            related_file: Some("src/lib.rs".to_owned()),
            source_id: None,
            metadata: json!({}),
        })
        .expect("seed feedback");

    // 5. Memoir dependency graph.
    store
        .create_memoir_graph(&NewMemoirGraph {
            repo_root: repo.clone(),
            scope: MemoryScope::Project,
            name: "memory-arch".to_owned(),
            description: "Memory layer relations".to_owned(),
            metadata: json!({}),
        })
        .expect("seed memoir graph");
    for (name, description) in [
        ("wake_up_pack", "Bounded session-start context pack."),
        ("memory_recall", "Lexical-first memory retrieval."),
    ] {
        store
            .add_memoir_concept(&NewMemoirConcept {
                repo_root: repo.clone(),
                scope: MemoryScope::Project,
                graph: "memory-arch".to_owned(),
                name: name.to_owned(),
                description: description.to_owned(),
                kind: "concept".to_owned(),
                source_id: None,
                metadata: json!({}),
            })
            .expect("seed memoir concept");
    }
    store
        .link_memoir_concepts(&NewMemoirRelation {
            repo_root: repo.clone(),
            scope: MemoryScope::Project,
            graph: "memory-arch".to_owned(),
            source: "wake_up_pack".to_owned(),
            target: "memory_recall".to_owned(),
            relation: "depends_on".to_owned(),
            confidence: 0.9,
            source_id: None,
            metadata: json!({}),
            create_missing: false,
        })
        .expect("seed memoir relation");

    // 6. Overview memory record for a code subject.
    let subject = OverviewSubject {
        kind: OverviewSubjectKind::Function,
        repo_root: repo.clone(),
        commit_sha: "fixture-sha".to_owned(),
        package_name: Some("sample".to_owned()),
        module_path: Some("src".to_owned()),
        canonical_file_path: Some("src/lib.rs".to_owned()),
        qualified_name: Some("src/lib.rs::fn::helper".to_owned()),
        node_kind: Some("function".to_owned()),
        line_start: Some(1),
        line_end: Some(3),
        content_hash: None,
    };
    let subject_id = atlas_session::subject_id(&subject);
    let overview = store
        .insert_overview_record(&NewOverviewRecord {
            subject,
            title: "helper".to_owned(),
            summary: "Formats a greeting with a suffix.".to_owned(),
            description_preview: "Builds the greeting through Greeter::greet_twice.".to_owned(),
            responsibilities: vec!["format greetings".to_owned()],
            flow: vec![],
            inputs: vec!["name: &str".to_owned()],
            outputs: vec!["String".to_owned()],
            invariants: vec![],
            gotchas: vec![],
            examples: vec![],
            tags: vec!["fixture".to_owned()],
            source_kind: OverviewSourceKind::Manual,
            analyzer_name: None,
            analyzer_version: None,
            model_name: None,
            graph_last_indexed_at: None,
            freshness: Some(OverviewFreshness::Fresh),
            confidence: 0.8,
            source_id: None,
            supersedes_id: None,
            metadata: json!({}),
        })
        .expect("seed overview record");

    // 7. Saved artifact referenced by `source_id` from the wake-up pack.
    let content_db = atlas_engine::paths::content_db_path(
        repo_root
            .join(".atlas")
            .join("worldtree.db")
            .to_str()
            .expect("graph db path"),
    );
    let artifact_source_id = "fixture-artifact".to_owned();
    let mut content_store = ContentStore::open(&content_db).expect("open content store");
    content_store
        .migrate()
        .expect("migrate content store");
    content_store
        .index_artifact(
            SourceMeta {
                id: artifact_source_id.clone(),
                session_id: None,
                agent_id: None,
                source_type: "review_context".to_owned(),
                label: "memory layer review context".to_owned(),
                repo_root: Some(repo.clone()),
                repo_roots: vec![repo.clone()],
                repo_id: None,
                repo_ids: Vec::new(),
                identity_kind: "fixture".to_owned(),
                identity_value: repo.clone(),
            },
            &"review context payload for the memory layer fixture ".repeat(40),
            "text/plain",
        )
        .expect("index artifact");

    MemoryLayerFixture {
        repo_root: repo,
        critical_memory_id: critical.id,
        stale_memory_id: stale.id,
        private_memory_id: private.id,
        feedback_id: feedback.id,
        overview_id: overview.id,
        overview_subject_id: subject_id,
        artifact_source_id,
    }
}

/// Move a memory's timestamps into the past by `days`.
fn backdate_memory(repo_root: &Path, memory_id: &str, days: i64) {
    let graph_db = repo_root.join(".atlas").join("worldtree.db");
    let session_db = derive_session_db_path(graph_db.to_str().expect("graph db path"));
    let conn = rusqlite::Connection::open(&session_db).expect("open session db");
    let cutoff = (time::OffsetDateTime::now_utc() - time::Duration::days(days))
        .replace_nanosecond(0)
        .expect("valid timestamp")
        .format(&time::format_description::well_known::Rfc3339)
        .expect("format timestamp");
    conn.execute(
        "UPDATE memories SET created_at = ?2, updated_at = ?2, last_accessed_at = ?2 WHERE id = ?1",
        rusqlite::params![memory_id, cutoff],
    )
    .expect("backdate memory");
}
