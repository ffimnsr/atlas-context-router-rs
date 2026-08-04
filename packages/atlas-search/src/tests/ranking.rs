use super::*;

#[test]
fn apply_ranking_boosts_exact_name() {
    use atlas_core::{Node, NodeId, NodeKind};

    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "search".to_string(),
        qualified_name: "src/lib.rs::fn::search".to_string(),
        file_path: "src/lib.rs".to_string(),
        line_start: 1,
        line_end: 10,
        language: "rust".to_string(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: Some("pub".to_string()),
        is_test: false,
        file_hash: "abc".to_string(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    };

    let input = vec![ScoredNode::new(node, 5.0)];
    let boosted = apply_ranking_boosts(
        input,
        "search",
        None,
        None,
        false,
        &HashSet::new(),
        &HashSet::new(),
    );

    // Exact name (+20) + fn kind (+3) + pub (+2) = +25 on top of 5.0
    assert!(
        boosted[0].score >= 30.0,
        "expected score >= 30, got {}",
        boosted[0].score
    );
}

#[test]
fn same_directory_boost_applied() {
    let same_dir = make_test_node("foo", "src/util.rs::fn::foo", "src/util.rs", "rust");
    let diff_dir = make_test_node("foo", "other/lib.rs::fn::foo", "other/lib.rs", "rust");

    let input = vec![diff_dir.clone(), same_dir.clone()];
    let boosted = apply_ranking_boosts(
        input,
        "foo",
        Some("src/main.rs"),
        None,
        false,
        &HashSet::new(),
        &HashSet::new(),
    );

    let same_score = boosted
        .iter()
        .find(|r| r.node.file_path == "src/util.rs")
        .unwrap()
        .score;
    let diff_score = boosted
        .iter()
        .find(|r| r.node.file_path == "other/lib.rs")
        .unwrap()
        .score;
    assert!(
        same_score > diff_score,
        "same-dir result should score higher; same={same_score} diff={diff_score}"
    );
}

#[test]
fn same_language_boost_applied() {
    let rust_node = make_test_node("parse", "src/a.rs::fn::parse", "src/a.rs", "rust");
    let go_node = make_test_node("parse", "src/a.go::fn::parse", "src/a.go", "go");

    let input = vec![go_node.clone(), rust_node.clone()];
    let boosted = apply_ranking_boosts(
        input,
        "parse",
        None,
        Some("rust"),
        false,
        &HashSet::new(),
        &HashSet::new(),
    );

    let rust_score = boosted
        .iter()
        .find(|r| r.node.language == "rust")
        .unwrap()
        .score;
    let go_score = boosted
        .iter()
        .find(|r| r.node.language == "go")
        .unwrap()
        .score;
    assert!(
        rust_score > go_score,
        "same-language result should score higher; rust={rust_score} go={go_score}"
    );
}

#[test]
fn same_dir_and_same_lang_both_applied() {
    // Node in same dir AND same language should get both boosts.
    let best = make_test_node("helper", "src/a.rs::fn::helper", "src/a.rs", "rust");
    let dir_only = make_test_node("helper", "src/b.go::fn::helper", "src/b.go", "go");
    let neither = make_test_node("helper", "lib/c.py::fn::helper", "lib/c.py", "python");

    let input = vec![neither.clone(), dir_only.clone(), best.clone()];
    let boosted = apply_ranking_boosts(
        input,
        "helper",
        Some("src/main.rs"),
        Some("rust"),
        false,
        &HashSet::new(),
        &HashSet::new(),
    );

    let best_score = boosted
        .iter()
        .find(|r| r.node.file_path == "src/a.rs")
        .unwrap()
        .score;
    let dir_only_score = boosted
        .iter()
        .find(|r| r.node.file_path == "src/b.go")
        .unwrap()
        .score;
    let neither_score = boosted
        .iter()
        .find(|r| r.node.file_path == "lib/c.py")
        .unwrap()
        .score;

    assert!(
        best_score > dir_only_score,
        "dir+lang node must beat dir-only; best={best_score} dir_only={dir_only_score}"
    );
    assert!(
        dir_only_score > neither_score,
        "dir-only node must beat neither; dir_only={dir_only_score} neither={neither_score}"
    );
}

#[test]
fn no_reference_file_no_boost() {
    let n1 = make_test_node("f", "src/a.rs::fn::f", "src/a.rs", "rust");
    let n2 = make_test_node("f", "lib/b.rs::fn::f", "lib/b.rs", "rust");

    let input = vec![n1.clone(), n2.clone()];
    let boosted = apply_ranking_boosts(
        input,
        "f",
        None,
        None,
        false,
        &HashSet::new(),
        &HashSet::new(),
    );

    // Both same language, no reference → scores should be equal (both
    // start at 1.0 with only the fn-kind +3 applied equally).
    let score_a = boosted
        .iter()
        .find(|r| r.node.file_path == "src/a.rs")
        .unwrap()
        .score;
    let score_b = boosted
        .iter()
        .find(|r| r.node.file_path == "lib/b.rs")
        .unwrap()
        .score;
    assert_eq!(
        score_a, score_b,
        "without reference both nodes should score equally"
    );
}
