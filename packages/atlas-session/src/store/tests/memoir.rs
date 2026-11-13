use super::*;

fn graph_input(name: &str, scope: MemoryScope) -> NewMemoirGraph {
    NewMemoirGraph {
        repo_root: "/repo".to_owned(),
        scope,
        name: name.to_owned(),
        description: format!("{name} description"),
        metadata: serde_json::json!({}),
    }
}

fn concept_input(graph: &str, name: &str) -> NewMemoirConcept {
    NewMemoirConcept {
        repo_root: "/repo".to_owned(),
        scope: MemoryScope::Project,
        graph: graph.to_owned(),
        name: name.to_owned(),
        description: format!("{name} description"),
        kind: "concept".to_owned(),
        source_id: Some(format!("src-{name}")),
        metadata: serde_json::json!({}),
    }
}

fn link_input(graph: &str, source: &str, target: &str, relation: &str) -> NewMemoirRelation {
    NewMemoirRelation {
        repo_root: "/repo".to_owned(),
        scope: MemoryScope::Project,
        graph: graph.to_owned(),
        source: source.to_owned(),
        target: target.to_owned(),
        relation: relation.to_owned(),
        confidence: 0.9,
        source_id: Some("evidence-1".to_owned()),
        metadata: serde_json::json!({}),
        create_missing: false,
    }
}

#[test]
fn memoir_tables_match_required_schema() {
    let (_dir, store) = open_store(16, 1024);

    assert_eq!(
        table_columns(&store.conn, "memoir_graphs"),
        vec![
            "id",
            "repo_root",
            "scope",
            "name",
            "description",
            "created_at",
            "updated_at",
            "metadata_json",
        ]
    );
    assert_eq!(
        table_columns(&store.conn, "memoir_concepts"),
        vec![
            "id",
            "graph_id",
            "repo_root",
            "name",
            "description",
            "kind",
            "source_id",
            "created_at",
            "updated_at",
            "metadata_json",
        ]
    );
    assert_eq!(
        table_columns(&store.conn, "memoir_relations"),
        vec![
            "id",
            "graph_id",
            "repo_root",
            "source_concept_id",
            "target_concept_id",
            "relation_type",
            "confidence",
            "source_id",
            "created_at",
            "metadata_json",
        ]
    );
    assert!(store.memoir_schema_issues().is_empty());
}

#[test]
fn duplicate_memoir_graph_names_fail_in_same_repo_and_scope() {
    let (_dir, mut store) = open_store(16, 1024);

    let first = store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Project))
        .unwrap();
    assert_eq!(first.scope, MemoryScope::Project);

    let error = store
        .create_memoir_graph(&graph_input("architecture", MemoryScope::Project))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("already exists in repo scope 'project'"),
        "duplicate graph must fail deterministically: {error}"
    );

    // Same name in another scope is a different graph identity.
    let global = store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Global))
        .unwrap();
    assert_ne!(first.id, global.id);
}

#[test]
fn memoir_link_accepts_controlled_vocabulary_and_rejects_unknown_types() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Project))
        .unwrap();
    let a = store
        .add_memoir_concept(&concept_input("Architecture", "parser"))
        .unwrap();
    let b = store
        .add_memoir_concept(&concept_input("Architecture", "lexer"))
        .unwrap();

    let relation = store
        .link_memoir_concepts(&link_input("Architecture", "parser", "lexer", "depends_on"))
        .unwrap();
    assert_eq!(relation.relation_type, "depends_on");
    assert_eq!(relation.source_concept_id, a.id);
    assert_eq!(relation.target_concept_id, b.id);
    assert_eq!(relation.confidence, 0.9);
    assert_eq!(relation.source_id.as_deref(), Some("evidence-1"));

    let error = store
        .link_memoir_concepts(&link_input(
            "Architecture",
            "parser",
            "lexer",
            "unrelated_to",
        ))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unknown memoir relation type: unrelated_to"),
        "unknown relation must fail clearly: {error}"
    );
    assert!(error.contains("depends_on"), "error must list vocabulary");
}

#[test]
fn memoir_relation_aliases_normalize_direction_and_tagging() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Project))
        .unwrap();
    let a = store
        .add_memoir_concept(&concept_input("Architecture", "old_parser"))
        .unwrap();
    let b = store
        .add_memoir_concept(&concept_input("Architecture", "new_parser"))
        .unwrap();

    // `A replaced_by B` means B replaces A: endpoints are swapped.
    let replaced = store
        .link_memoir_concepts(&link_input(
            "Architecture",
            "old_parser",
            "new_parser",
            "replaced_by",
        ))
        .unwrap();
    assert_eq!(replaced.relation_type, "replaces");
    assert_eq!(replaced.source_concept_id, b.id);
    assert_eq!(replaced.target_concept_id, a.id);
    assert_eq!(replaced.metadata["alias"], "replaced_by");
    assert_eq!(replaced.metadata["direction"], "reversed");

    // `separate_from` tags the alias but keeps direction.
    let separate = store
        .link_memoir_concepts(&link_input(
            "Architecture",
            "old_parser",
            "new_parser",
            "separate_from",
        ))
        .unwrap();
    assert_eq!(separate.relation_type, "related_to");
    assert_eq!(separate.source_concept_id, a.id);
    assert_eq!(separate.target_concept_id, b.id);
    assert_eq!(separate.metadata["alias"], "separate_from");
}

#[test]
fn memoir_link_does_not_create_missing_concepts_unless_opted_in() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Project))
        .unwrap();
    store
        .add_memoir_concept(&concept_input("Architecture", "parser"))
        .unwrap();

    let error = store
        .link_memoir_concepts(&link_input(
            "Architecture",
            "parser",
            "missing_tokenizer",
            "depends_on",
        ))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("not found in graph 'Architecture'"),
        "missing endpoint must fail by default: {error}"
    );
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memoir_concepts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "no concept may be auto-created by default");

    let mut input = link_input("Architecture", "parser", "missing_tokenizer", "depends_on");
    input.create_missing = true;
    let relation = store.link_memoir_concepts(&input).unwrap();
    assert_eq!(relation.relation_type, "depends_on");
    let count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memoir_concepts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2, "explicit opt-in creates the endpoint");
}

#[test]
fn memoir_inspect_is_bounded_and_reports_direction_and_evidence() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Project))
        .unwrap();
    for name in ["parser", "lexer", "buffer"] {
        store
            .add_memoir_concept(&concept_input("Architecture", name))
            .unwrap();
    }
    store
        .link_memoir_concepts(&link_input("Architecture", "parser", "lexer", "depends_on"))
        .unwrap();
    store
        .link_memoir_concepts(&link_input("Architecture", "buffer", "parser", "part_of"))
        .unwrap();
    store
        .link_memoir_concepts(&link_input("Architecture", "lexer", "buffer", "refines"))
        .unwrap();

    let inspection = store
        .inspect_memoir_concept(&MemoirInspectQuery {
            repo_root: "/repo".to_owned(),
            graph: Some("Architecture".to_owned()),
            concept: "parser".to_owned(),
            scope: None,
            depth: 1,
            relation: None,
        })
        .unwrap();
    assert_eq!(inspection.root.name, "parser");
    assert_eq!(inspection.edges.len(), 2, "depth 1 stays bounded");
    assert!(!inspection.truncated);

    let outgoing = inspection
        .edges
        .iter()
        .find(|edge| edge.relation_type == "depends_on")
        .unwrap();
    assert_eq!(outgoing.direction, MemoirRelationDirection::Outgoing);
    assert_eq!(outgoing.from_concept, "parser");
    assert_eq!(outgoing.to_concept, "lexer");
    assert_eq!(outgoing.source_id.as_deref(), Some("evidence-1"));

    let incoming = inspection
        .edges
        .iter()
        .find(|edge| edge.relation_type == "part_of")
        .unwrap();
    assert_eq!(incoming.direction, MemoirRelationDirection::Incoming);
    assert_eq!(incoming.from_concept, "buffer");
    assert_eq!(incoming.to_concept, "parser");

    let filtered = store
        .inspect_memoir_concept(&MemoirInspectQuery {
            repo_root: "/repo".to_owned(),
            graph: Some("Architecture".to_owned()),
            concept: "parser".to_owned(),
            scope: None,
            depth: 2,
            relation: Some("depends_on".to_owned()),
        })
        .unwrap();
    assert_eq!(filtered.relation.as_deref(), Some("depends_on"));
    assert!(
        filtered
            .edges
            .iter()
            .all(|edge| edge.relation_type == "depends_on"),
        "relation filter must bound the traversal"
    );

    let deep = store
        .inspect_memoir_concept(&MemoirInspectQuery {
            repo_root: "/repo".to_owned(),
            graph: Some("Architecture".to_owned()),
            concept: "parser".to_owned(),
            scope: None,
            depth: 2,
            relation: None,
        })
        .unwrap();
    assert!(deep.edges.len() > inspection.edges.len());
}

#[test]
fn memoir_inspect_resolves_by_concept_and_rejects_ambiguity() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Project))
        .unwrap();
    store
        .add_memoir_concept(&concept_input("Architecture", "parser"))
        .unwrap();

    let inspection = store
        .inspect_memoir_concept(&MemoirInspectQuery {
            repo_root: "/repo".to_owned(),
            graph: None,
            concept: "parser".to_owned(),
            scope: None,
            depth: 1,
            relation: None,
        })
        .unwrap();
    assert_eq!(inspection.graph.name, "Architecture");

    store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Global))
        .unwrap();
    let mut global_concept = concept_input("Architecture", "parser");
    global_concept.scope = MemoryScope::Global;
    store.add_memoir_concept(&global_concept).unwrap();

    let error = store
        .inspect_memoir_concept(&MemoirInspectQuery {
            repo_root: "/repo".to_owned(),
            graph: None,
            concept: "parser".to_owned(),
            scope: None,
            depth: 1,
            relation: None,
        })
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("ambiguous across graphs"),
        "ambiguous concept must not silently pick a graph: {error}"
    );
}

#[test]
fn delete_memoir_graph_removes_concepts_and_relations() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .create_memoir_graph(&graph_input("Architecture", MemoryScope::Project))
        .unwrap();
    store
        .add_memoir_concept(&concept_input("Architecture", "parser"))
        .unwrap();
    store
        .add_memoir_concept(&concept_input("Architecture", "lexer"))
        .unwrap();
    store
        .link_memoir_concepts(&link_input("Architecture", "parser", "lexer", "depends_on"))
        .unwrap();

    let dry_run = store
        .delete_memoir_graph("/repo", MemoryScope::Project, "Architecture", true)
        .unwrap();
    assert!(dry_run.found);
    assert!(!dry_run.deleted);
    assert_eq!(dry_run.concepts_deleted, 2);
    assert_eq!(dry_run.relations_deleted, 1);

    let deleted = store
        .delete_memoir_graph("/repo", MemoryScope::Project, "Architecture", false)
        .unwrap();
    assert!(deleted.found);
    assert!(deleted.deleted);
    assert_eq!(deleted.concepts_deleted, 2);
    assert_eq!(deleted.relations_deleted, 1);

    let remaining: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM memoir_graphs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, 0);

    let missing = store
        .delete_memoir_graph("/repo", MemoryScope::Project, "Architecture", false)
        .unwrap();
    assert!(!missing.found);
    assert!(!missing.deleted);
}
