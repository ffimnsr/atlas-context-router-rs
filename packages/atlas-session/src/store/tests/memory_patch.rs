use super::*;

use crate::store::memory_ops::MemoryOp;
use crate::store::memory_patch::{apply_patch_block, parse_patch_blocks};

#[test]
fn parses_multiple_blocks_in_document_order() {
    let patch = "<<<< SEARCH\nalpha\n====\nALPHA\n>>>> REPLACE\n\n<<<< SEARCH\nbeta\n====\nBETA\n>>>> REPLACE\n";
    let blocks = parse_patch_blocks(patch).unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].search, "alpha");
    assert_eq!(blocks[0].replace, "ALPHA");
    assert_eq!(blocks[1].search, "beta");
}

#[test]
fn rejects_invalid_markers() {
    for patch in [
        "<<<< SEARCH\nalpha\n====\nALPHA\n",
        "<<<< SEARCH\nalpha\n",
        "not a marker\n",
        "<<<< SEARCH\nalpha\n<<<< SEARCH\n====\nx\n>>>> REPLACE\n",
        "",
    ] {
        let error = parse_patch_blocks(patch).unwrap_err().to_string();
        assert!(
            error.contains("memory_patch_invalid"),
            "unexpected error for {patch:?}: {error}"
        );
    }
}

#[test]
fn exact_patch_requires_unique_match() {
    let applied = apply_patch_block("deploy uses helm today", "helm", "helm charts", None).unwrap();
    assert!(applied.matched_exact);
    assert_eq!(applied.edit_distance, 0);
    assert_eq!(applied.body, "deploy uses helm charts today");

    let error = apply_patch_block("helm and helm", "helm", "charts", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("memory_patch_ambiguous"), "got: {error}");
}

#[test]
fn fuzzy_patch_applies_within_threshold() {
    // 10-char search, default threshold = ceil(15%) = 2.
    let applied = apply_patch_block("the connectiom pool", "connection", "database", None).unwrap();
    assert!(!applied.matched_exact);
    assert_eq!(applied.edit_distance, 1);
    assert_eq!(applied.body, "the database pool");
}

#[test]
fn fuzzy_patch_rejects_above_threshold_and_above_half_length() {
    let error = apply_patch_block("completely different", "connection", "x", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("memory_patch_invalid"), "got: {error}");

    let error = apply_patch_block("connexion", "connection", "x", Some(10))
        .unwrap_err()
        .to_string();
    assert!(error.contains("50%"), "got: {error}");
}

#[test]
fn patch_cannot_empty_the_body() {
    let error = apply_patch_block("only text", "only text", "", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("empty body"), "got: {error}");
}

#[test]
fn store_patch_applies_blocks_and_records_history() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_memory(
        &store,
        "patch-me",
        "2026-01-01T00:00:00Z",
        "deploy uses helm",
        "deploy",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );

    let patch = "<<<< SEARCH\nhelm\n====\nhelm charts\n>>>> REPLACE\n";
    let outcomes = store
        .apply_memory_patch("/repo", "patch-me", patch, None)
        .unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].op, "eua_patch");

    let record = store.get_memory("/repo", "patch-me").unwrap().unwrap();
    assert_eq!(record.body, "deploy uses helm charts");

    let history = store
        .memory_op_history("/repo", "patch-me")
        .unwrap()
        .unwrap();
    assert_eq!(history.entries.len(), 1);
    let details = history.entries[0].details.as_ref().expect("patch details");
    assert_eq!(details["matched_exact"], serde_json::json!(true));
    assert_eq!(details["replace_len"], serde_json::json!(11));
    assert!(details["search_hash"].as_str().unwrap().len() == 64);
}

#[test]
fn store_patch_rejects_superseded_memories() {
    let (_dir, mut store) = open_store(16, 1024);
    seed_memory(
        &store,
        "old",
        "2026-01-01T00:00:00Z",
        "old fact",
        "deploy",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    store
        .conn
        .execute(
            "UPDATE memories SET superseded_by = 'newer' WHERE id = 'old'",
            [],
        )
        .unwrap();

    let patch = "<<<< SEARCH\nold\n====\nnew\n>>>> REPLACE\n";
    let error = store
        .apply_memory_patch("/repo", "old", patch, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("superseded"), "got: {error}");
}

#[test]
fn direct_eua_operator_matches_store_patch() {
    let mut entity = MemoryEntity {
        id: "m".to_owned(),
        repo_root: "/repo".to_owned(),
        topic: String::new(),
        title: String::new(),
        body: "alpha beta".to_owned(),
        importance: MemoryImportance::Normal,
        scope: MemoryScope::Project,
        source_id: None,
        metadata: serde_json::json!({}),
        numeric_value: None,
        op_history: Vec::new(),
        superseded_by: None,
    };
    let op = MemoryOp::EuaPatch {
        search: "beta".to_owned(),
        replace: "gamma".to_owned(),
        max_edit_distance: None,
    };
    apply_memory_op(&mut entity, &op, "2026-01-01T00:00:00Z").unwrap();
    assert_eq!(entity.body, "alpha gamma");
}
