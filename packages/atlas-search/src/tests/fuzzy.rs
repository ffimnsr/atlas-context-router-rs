use super::*;

#[test]
fn edit_distance_basic() {
    assert_eq!(edit_distance("kitten", "sitting", 10), 3);
    assert_eq!(edit_distance("abc", "abc", 5), 0);
    assert_eq!(edit_distance("abc", "xyz", 10), 3);
    // Cap early-exit: length diff > cap
    assert_eq!(edit_distance("short", "muchlongerstring", 2), 3);
}

#[test]
fn fuzzy_match_boost_applied() {
    // "sarch" is 1 edit away from "search" → should get fuzzy boost.
    let close = make_test_node("search", "src/lib.rs::fn::search", "src/lib.rs", "rust");
    let distant = make_test_node(
        "transform",
        "src/lib.rs::fn::transform",
        "src/lib.rs",
        "rust",
    );

    let input = vec![distant.clone(), close.clone()];
    let boosted = apply_ranking_boosts(
        input,
        "sarch",
        None,
        None,
        true,
        &HashSet::new(),
        &HashSet::new(),
    );

    let close_score = boosted
        .iter()
        .find(|r| r.node.name == "search")
        .unwrap()
        .score;
    let distant_score = boosted
        .iter()
        .find(|r| r.node.name == "transform")
        .unwrap()
        .score;
    assert!(
        close_score > distant_score,
        "fuzzy-close name should score higher; close={close_score} distant={distant_score}"
    );
}

#[test]
fn fuzzy_match_off_no_boost() {
    // Same setup but fuzzy_match=false → no extra boost for "sarch".
    let close = make_test_node("search", "src/lib.rs::fn::search", "src/lib.rs", "rust");
    let input = vec![close];
    let no_fuzzy = apply_ranking_boosts(
        input.clone(),
        "sarch",
        None,
        None,
        false,
        &HashSet::new(),
        &HashSet::new(),
    );
    let with_fuzzy = apply_ranking_boosts(
        input,
        "sarch",
        None,
        None,
        true,
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        with_fuzzy[0].score > no_fuzzy[0].score,
        "fuzzy=true should score higher than fuzzy=false for a close mismatch"
    );
}

#[test]
fn fuzzy_typo_bonus_prefers_code_symbol_over_markdown_file() {
    let function = make_test_node(
        "LoadIdentityMessages",
        "internal/requestctx/context.go::fn::LoadIdentityMessages",
        "internal/requestctx/context.go",
        "go",
    );
    let mut markdown = make_test_node(
        "Load Identity Messages",
        "docs/load_identity_messages.md",
        "docs/load_identity_messages.md",
        "markdown",
    );
    markdown.node.kind = NodeKind::File;
    markdown.score = 12.0;

    let boosted = apply_ranking_boosts(
        vec![markdown, function],
        "LoadIdentityMesages",
        None,
        None,
        true,
        &HashSet::new(),
        &HashSet::new(),
    );

    assert_eq!(boosted[0].node.kind, NodeKind::Function);
    assert_eq!(boosted[0].node.name, "LoadIdentityMessages");
}

#[test]
fn search_fuzzy_typo_recovers_symbol_above_markdown_noise() {
    use atlas_core::{Node, NodeId};

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let function = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "LoadIdentityMessages".to_string(),
        qualified_name: "internal/requestctx/context.go::fn::LoadIdentityMessages".to_string(),
        file_path: "internal/requestctx/context.go".to_string(),
        line_start: 1,
        line_end: 20,
        language: "go".to_string(),
        parent_name: None,
        params: Some("()".to_string()),
        return_type: None,
        modifiers: Some("export".to_string()),
        is_test: false,
        file_hash: "h1".to_string(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph(
            "internal/requestctx/context.go",
            "h1",
            Some("go"),
            Some(20),
            &[function],
            &[],
        )
        .expect("replace function graph");

    let markdown = Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: "Load Identity Messages".to_string(),
        qualified_name: "docs/load_identity_messages.md".to_string(),
        file_path: "docs/load_identity_messages.md".to_string(),
        line_start: 1,
        line_end: 40,
        language: "markdown".to_string(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "h2".to_string(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph(
            "docs/load_identity_messages.md",
            "h2",
            Some("markdown"),
            Some(40),
            &[markdown],
            &[],
        )
        .expect("replace markdown graph");

    let query = SearchQuery {
        text: "LoadIdentityMesages".to_string(),
        fuzzy_match: true,
        include_files: true,
        limit: 10,
        ..Default::default()
    };

    let results = search(&store, &query).expect("search results");
    assert!(!results.is_empty(), "expected fuzzy search results");
    assert_eq!(results[0].node.kind, NodeKind::Function);
    assert_eq!(results[0].node.name, "LoadIdentityMessages");
    assert!(
        results
            .iter()
            .any(|result| result.node.kind == NodeKind::File),
        "expected file noise to remain available when include_files=true"
    );
}
