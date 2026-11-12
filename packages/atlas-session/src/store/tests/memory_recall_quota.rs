use super::*;

use crate::store::memory_recall::time_score;

fn add_memoir_concept(store: &mut SessionStore, name: &str, source_id: &str) {
    store
        .create_memoir_graph(&NewMemoirGraph {
            repo_root: "/repo".to_owned(),
            scope: MemoryScope::Project,
            name: "graph".to_owned(),
            description: String::new(),
            metadata: serde_json::json!({}),
        })
        .ok();
    store
        .add_memoir_concept(&NewMemoirConcept {
            repo_root: "/repo".to_owned(),
            scope: MemoryScope::Project,
            graph: "graph".to_owned(),
            name: name.to_owned(),
            description: String::new(),
            kind: "concept".to_owned(),
            source_id: Some(source_id.to_owned()),
            metadata: serde_json::json!({}),
        })
        .unwrap();
}

fn recall(
    store: &SessionStore,
    query: &str,
    viewer: &MemoryViewer,
    limit: usize,
) -> Vec<MemorySearchHit> {
    store
        .recall_memories(
            "/repo",
            query,
            &MemoryListFilter::default(),
            false,
            viewer,
            limit,
        )
        .unwrap()
}

#[test]
fn fts_matches_non_adjacent_tokens() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "pool",
        "2026-01-01T00:00:00Z",
        "postgres connection pool tuning",
        "notes",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "other",
        "2026-01-01T00:00:00Z",
        "postgres only",
        "notes",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    let hits = recall(&store, "postgres pool", &cli_viewer(), 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.id, "pool");
    assert_eq!(hits[0].ranking.recall_path, "primary");
    assert!(hits[0].recall_score > 0.0 && hits[0].recall_score <= 1.0);
}

#[test]
fn freshness_and_composite_scores_are_bounded() {
    assert_eq!(time_score(1, 1), 1.0);
    assert!(time_score(31, 1) < 1.0);

    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m",
        "2026-01-01T00:00:00Z",
        "deploy body",
        "deploy",
        MemoryImportance::Critical,
        MemoryScope::Global,
    );
    let hits = recall(&store, "deploy", &cli_viewer(), 10);
    let hit = &hits[0];
    assert!(
        (0.0..=1.0).contains(&hit.ranking.origin_score),
        "origin score out of range"
    );
    assert!((0.0..=1.0).contains(&hit.recall_score));
    assert_eq!(hit.ranking.importance_score, 1.0);
    assert_eq!(hit.ranking.topic_match, "exact_topic");
}

#[test]
fn quota_split_matches_limit_and_never_duplicates_ids() {
    let (_dir, mut store) = open_store(16, 1024);
    for index in 0..10 {
        seed_memory_full(
            &store,
            &format!("p{index}"),
            "2026-01-01T00:00:00Z",
            &format!("alpha target {index}"),
            "notes",
            MemoryImportance::Normal,
            MemoryScope::Project,
            None,
            None,
            Some(&format!("src-p{index}")),
        );
    }
    for index in 0..3 {
        seed_memory_full(
            &store,
            &format!("a{index}"),
            "2026-01-01T00:00:00Z",
            &format!("unrelated aux fact {index}"),
            "aux",
            MemoryImportance::Normal,
            MemoryScope::Project,
            None,
            None,
            Some(&format!("src-aux{index}")),
        );
        add_memoir_concept(
            &mut store,
            &format!("beta concept {index}"),
            &format!("src-aux{index}"),
        );
    }

    let hits = recall(&store, "alpha beta", &cli_viewer(), 4);
    let primary = hits
        .iter()
        .filter(|hit| hit.ranking.recall_path == "primary")
        .count();
    let auxiliary = hits
        .iter()
        .filter(|hit| hit.ranking.recall_path == "auxiliary")
        .count();
    assert_eq!(hits.len(), 4);
    assert_eq!(primary, 3, "ceil(0.75 * 4) = 3 primary");
    assert_eq!(auxiliary, 1);

    let hits = recall(&store, "alpha beta", &cli_viewer(), 10);
    let primary = hits
        .iter()
        .filter(|hit| hit.ranking.recall_path == "primary")
        .count();
    let auxiliary = hits
        .iter()
        .filter(|hit| hit.ranking.recall_path == "auxiliary")
        .count();
    assert_eq!(primary, 8, "ceil(0.75 * 10) = 8 primary");
    assert_eq!(auxiliary, 2, "remaining quota is 2");

    let mut ids = std::collections::BTreeSet::new();
    for hit in &hits {
        assert!(ids.insert(hit.memory.id.clone()), "duplicate recall id");
    }
}

#[test]
fn auxiliary_recall_respects_visibility() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_memory_full(
        &store,
        "shared",
        "2026-01-01T00:00:00Z",
        "alpha primary",
        "notes",
        MemoryImportance::Normal,
        MemoryScope::Project,
        None,
        None,
        None,
    );
    seed_memory_full(
        &store,
        "private",
        "2026-01-01T00:00:00Z",
        "opaque private note",
        "aux",
        MemoryImportance::Normal,
        MemoryScope::Frontend,
        None,
        Some("codex"),
        Some("src-private"),
    );
    add_memoir_concept(&mut store, "beta marker", "src-private");

    let cli_hits = recall(&store, "alpha beta", &cli_viewer(), 10);
    assert!(
        cli_hits.iter().all(|hit| hit.memory.id != "private"),
        "cli viewer must not see codex frontend memories"
    );

    let codex_hits = recall(
        &store,
        "alpha beta",
        &MemoryViewer {
            frontend: "codex".to_owned(),
            session_id: "s1".to_owned(),
        },
        10,
    );
    assert!(
        codex_hits.iter().any(|hit| hit.memory.id == "private"),
        "matching frontend sees the auxiliary row"
    );
}

#[test]
fn recall_without_memoir_returns_primary_only() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "m",
        "2026-01-01T00:00:00Z",
        "alpha beta fact",
        "notes",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    let hits = recall(&store, "alpha beta", &cli_viewer(), 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].ranking.recall_path, "primary");
}

#[test]
fn memoir_relation_hops_fill_auxiliary_quota() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_memory_full(
        &store,
        "p",
        "2026-01-01T00:00:00Z",
        "gamma primary",
        "notes",
        MemoryImportance::Normal,
        MemoryScope::Project,
        None,
        None,
        None,
    );
    seed_memory_full(
        &store,
        "via-hop",
        "2026-01-01T00:00:00Z",
        "opaque hop fact",
        "aux",
        MemoryImportance::Normal,
        MemoryScope::Project,
        None,
        None,
        Some("src-hop"),
    );
    add_memoir_concept(&mut store, "beta seed", "src-seed");
    add_memoir_concept(&mut store, "hop target", "src-hop");
    store
        .link_memoir_concepts(&NewMemoirRelation {
            repo_root: "/repo".to_owned(),
            scope: MemoryScope::Project,
            graph: "graph".to_owned(),
            source: "beta seed".to_owned(),
            target: "hop target".to_owned(),
            relation: "depends_on".to_owned(),
            confidence: 1.0,
            source_id: None,
            metadata: serde_json::json!({}),
            create_missing: false,
        })
        .unwrap();

    let hits = recall(&store, "gamma beta", &cli_viewer(), 10);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].ranking.recall_path, "primary");
    assert_eq!(hits[1].memory.id, "via-hop");
    assert_eq!(hits[1].ranking.recall_path, "auxiliary");
    assert!(hits[1].recall_score <= 1.0);
}
