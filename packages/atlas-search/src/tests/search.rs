use super::*;

#[test]
fn search_excludes_file_nodes_by_default() {
    use atlas_core::{Node, NodeId};

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let file_node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: "Architecture Notes".to_string(),
        qualified_name: "docs/architecture.md".to_string(),
        file_path: "docs/architecture.md".to_string(),
        line_start: 1,
        line_end: 10,
        language: "markdown".to_string(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "h".to_string(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph(
            "docs/architecture.md",
            "h",
            Some("markdown"),
            Some(10),
            &[file_node],
            &[],
        )
        .expect("replace file graph");

    let no_files = search(
        &store,
        &SearchQuery {
            text: "Architecture Notes".to_string(),
            limit: 5,
            ..Default::default()
        },
    )
    .expect("search without files");
    assert!(
        no_files.is_empty(),
        "file nodes should be hidden by default"
    );

    let with_files = search(
        &store,
        &SearchQuery {
            text: "Architecture Notes".to_string(),
            include_files: true,
            limit: 5,
            ..Default::default()
        },
    )
    .expect("search with files");
    assert_eq!(with_files[0].node.kind, NodeKind::File);
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

#[test]
fn apply_ranking_boosts_records_evidence_fields() {
    let mut boosted_node = make_test_node(
        "search",
        "src/search.rs::fn::search",
        "src/search.rs",
        "rust",
    );
    boosted_node.node.modifiers = Some("pub".to_string());

    let recent_set: HashSet<String> = ["src/search.rs".to_string()].into();
    let changed_set: HashSet<String> = ["src/search.rs".to_string()].into();
    let boosted = apply_ranking_boosts(
        vec![boosted_node],
        "search",
        Some("src/main.rs"),
        Some("rust"),
        false,
        &recent_set,
        &changed_set,
    );

    let evidence = boosted[0]
        .ranking_evidence
        .as_ref()
        .expect("ranking evidence");
    assert!(evidence.exact_name_match);
    assert_eq!(evidence.kind_boost, Some(3.0));
    assert_eq!(evidence.public_exported_boost, Some(2.0));
    assert_eq!(evidence.same_directory_boost, Some(3.0));
    assert_eq!(evidence.same_language_boost, Some(2.0));
    assert_eq!(evidence.recent_file_boost, Some(4.0));
    assert_eq!(evidence.changed_file_boost, Some(5.0));
    assert!(evidence.matched_fields.contains(&SearchMatchedField::Name));
}

#[test]
fn search_relaxed_fuzzy_records_term_distance_and_threshold() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "search".to_string(),
        qualified_name: "src/lib.rs::fn::search".to_string(),
        file_path: "src/lib.rs".to_string(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_string(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "h".to_string(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    };
    store
        .replace_file_graph("src/lib.rs", "h", Some("rust"), Some(5), &[node], &[])
        .expect("replace file graph");

    let results = search(
        &store,
        &SearchQuery {
            text: "serch".to_string(),
            fuzzy_match: true,
            ..Default::default()
        },
    )
    .expect("search results");

    let evidence = results[0]
        .ranking_evidence
        .as_ref()
        .expect("ranking evidence");
    let fuzzy = evidence.fuzzy.as_ref().expect("fuzzy evidence");
    assert_eq!(fuzzy.corrected_term.as_deref(), Some("search"));
    assert_eq!(fuzzy.edit_distance, Some(1));
    assert_eq!(fuzzy.fuzzy_threshold, Some(1));
}
