//! Integration tests for atlas-refactor — Phase 24.
//!
//! Uses an in-memory SQLite store with hand-crafted nodes and edges to
//! exercise every major code path without requiring a real repository.

use std::path::Path;

use atlas_core::{EdgeKind, NodeKind, RefactorEditKind, SafetyBand};
use atlas_refactor::RefactorEngine;
use atlas_store_sqlite::Store;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Create a temporary directory and return its path.
fn tmp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// Open an in-memory store (uses a temp file path to avoid collisions).
fn open_store(dir: &Path) -> Store {
    let db = dir.join("test.db");
    Store::open(db.to_str().unwrap()).expect("open store")
}

/// Write `content` to `dir/rel_path` creating parent dirs as needed.
fn write_file(dir: &Path, rel_path: &str, content: &str) {
    let full = dir.join(rel_path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(full, content).unwrap();
}

/// Insert a `ParsedFile` into the store.
fn insert_pf(store: &mut Store, pf: atlas_core::ParsedFile) {
    store
        .replace_files_transactional(&[pf])
        .expect("replace_files_transactional");
}

/// Return a minimal `ParsedFile` for a file with one function node.
fn parsed_file_with_fn(
    path: &str,
    fn_name: &str,
    qname: &str,
    line_start: u32,
    line_end: u32,
) -> atlas_core::ParsedFile {
    atlas_core::ParsedFile {
        path: path.to_string(),
        language: Some("rust".to_string()),
        hash: "abc".to_string(),
        size: None,
        nodes: vec![atlas_core::Node {
            id: atlas_core::NodeId::UNSET,
            kind: NodeKind::Function,
            name: fn_name.to_string(),
            qualified_name: qname.to_string(),
            file_path: path.to_string(),
            line_start,
            line_end,
            language: "rust".to_string(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "abc".to_string(),
            extra_json: serde_json::Value::Null,
        }],
        edges: vec![],
    }
}

// ---------------------------------------------------------------------------
// 24.2 — Rename tests
// ---------------------------------------------------------------------------

#[test]
fn rename_single_file_symbol() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/lib.rs";
    let content = "fn old_name() {\n    println!(\"hi\");\n}\n";
    write_file(dir.path(), src, content);

    let pf = parsed_file_with_fn(src, "old_name", "src/lib.rs::fn::old_name", 1, 3);
    insert_pf(&mut store, pf);

    let engine = RefactorEngine::new(&store, dir.path());
    let plan = engine
        .plan_rename("src/lib.rs::fn::old_name", "new_name")
        .expect("plan");

    assert!(!plan.edits.is_empty(), "expected at least one edit");
    assert!(plan.affected_files.contains(&src.to_string()));
    assert_eq!(plan.estimated_safety, SafetyBand::Safe);

    // Dry run.
    let result = engine.apply_rename(&plan, true).expect("dry run");
    assert!(result.dry_run);
    assert!(result.validation.valid);
    assert!(!result.patches.is_empty());
    let diff = &result.patches[0].unified_diff;
    assert!(
        diff.contains("+fn new_name()"),
        "expected +fn new_name in diff:\n{diff}"
    );
    assert!(
        diff.contains("-fn old_name()"),
        "expected -fn old_name in diff:\n{diff}"
    );

    // File must be unchanged after dry run.
    let after = std::fs::read_to_string(dir.path().join(src)).unwrap();
    assert_eq!(after, content, "dry-run must not write files");
}

#[test]
fn rename_multi_file_symbol() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let def_file = "src/lib.rs";
    let ref_file = "src/main.rs";

    let def_content = "pub fn my_func() {}\n";
    let ref_content = "use crate::my_func;\nfn main() { my_func(); }\n";
    write_file(dir.path(), def_file, def_content);
    write_file(dir.path(), ref_file, ref_content);

    // Insert definition node.
    let pf_def = parsed_file_with_fn(def_file, "my_func", "src/lib.rs::fn::my_func", 1, 1);
    insert_pf(&mut store, pf_def);

    // Insert reference node and edge manually.
    let pf_ref = atlas_core::ParsedFile {
        path: ref_file.to_string(),
        language: Some("rust".to_string()),
        hash: "xyz".to_string(),
        size: None,
        nodes: vec![atlas_core::Node {
            id: atlas_core::NodeId::UNSET,
            kind: NodeKind::Function,
            name: "main".to_string(),
            qualified_name: "src/main.rs::fn::main".to_string(),
            file_path: ref_file.to_string(),
            line_start: 2,
            line_end: 2,
            language: "rust".to_string(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "xyz".to_string(),
            extra_json: serde_json::Value::Null,
        }],
        edges: vec![atlas_core::Edge {
            id: 0,
            kind: EdgeKind::Calls,
            source_qn: "src/main.rs::fn::main".to_string(),
            target_qn: "src/lib.rs::fn::my_func".to_string(),
            file_path: ref_file.to_string(),
            line: Some(2),
            confidence: 1.0,
            confidence_tier: Some("high".to_string()),
            extra_json: serde_json::Value::Null,
        }],
    };
    insert_pf(&mut store, pf_ref);

    let engine = RefactorEngine::new(&store, dir.path());
    let plan = engine
        .plan_rename("src/lib.rs::fn::my_func", "renamed_func")
        .unwrap();

    assert!(
        plan.affected_files.contains(&def_file.to_string())
            || plan.affected_files.contains(&ref_file.to_string()),
        "affected files: {:?}",
        plan.affected_files
    );

    let result = engine.apply_rename(&plan, true).unwrap();
    assert!(result.validation.valid);
    assert!(!result.patches.is_empty());
}

#[test]
fn rename_collision_rejected() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/lib.rs";
    let content = "fn old_name() {}\nfn new_name() {}\n";
    write_file(dir.path(), src, content);

    // Two nodes in same file.
    let pf = atlas_core::ParsedFile {
        path: src.to_string(),
        language: Some("rust".to_string()),
        hash: "abc".to_string(),
        size: None,
        nodes: vec![
            atlas_core::Node {
                id: atlas_core::NodeId::UNSET,
                kind: NodeKind::Function,
                name: "old_name".to_string(),
                qualified_name: "src/lib.rs::fn::old_name".to_string(),
                file_path: src.to_string(),
                line_start: 1,
                line_end: 1,
                language: "rust".to_string(),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: "abc".to_string(),
                extra_json: serde_json::Value::Null,
            },
            atlas_core::Node {
                id: atlas_core::NodeId::UNSET,
                kind: NodeKind::Function,
                name: "new_name".to_string(),
                qualified_name: "src/lib.rs::fn::new_name".to_string(),
                file_path: src.to_string(),
                line_start: 2,
                line_end: 2,
                language: "rust".to_string(),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: "abc".to_string(),
                extra_json: serde_json::Value::Null,
            },
        ],
        edges: vec![],
    };
    insert_pf(&mut store, pf);

    let engine = RefactorEngine::new(&store, dir.path());
    let err = engine.plan_rename("src/lib.rs::fn::old_name", "new_name");
    assert!(err.is_err(), "collision must be rejected");
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("collision"), "error: {msg}");
}

// ---------------------------------------------------------------------------
// 24.3 — Dead-code removal tests
// ---------------------------------------------------------------------------

#[test]
fn dead_code_removal_private_helper() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/util.rs";
    let content = "fn used() {}\n\nfn unused_helper() {\n    let x = 1;\n}\n";
    write_file(dir.path(), src, content);

    // Only `unused_helper` is in the graph with no inbound edges.
    let pf = atlas_core::ParsedFile {
        path: src.to_string(),
        language: Some("rust".to_string()),
        hash: "h1".to_string(),
        size: None,
        nodes: vec![
            atlas_core::Node {
                id: atlas_core::NodeId::UNSET,
                kind: NodeKind::Function,
                name: "used".to_string(),
                qualified_name: "src/util.rs::fn::used".to_string(),
                file_path: src.to_string(),
                line_start: 1,
                line_end: 1,
                language: "rust".to_string(),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: "h1".to_string(),
                extra_json: serde_json::Value::Null,
            },
            atlas_core::Node {
                id: atlas_core::NodeId::UNSET,
                kind: NodeKind::Function,
                name: "unused_helper".to_string(),
                qualified_name: "src/util.rs::fn::unused_helper".to_string(),
                file_path: src.to_string(),
                line_start: 3,
                line_end: 5,
                language: "rust".to_string(),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: "h1".to_string(),
                extra_json: serde_json::Value::Null,
            },
        ],
        edges: vec![],
    };
    insert_pf(&mut store, pf);

    let engine = RefactorEngine::new(&store, dir.path());
    let plan = engine
        .plan_dead_code_removal("src/util.rs::fn::unused_helper")
        .unwrap();

    assert!(!plan.edits.is_empty());
    let removal = plan
        .edits
        .iter()
        .find(|e| e.edit_kind == RefactorEditKind::RemoveSpan);
    assert!(removal.is_some(), "expected a RemoveSpan edit");

    let result = engine.apply_dead_code_removal(&plan, true).unwrap();
    assert!(result.dry_run);
    assert!(result.validation.valid);
}

#[test]
fn protected_entrypoint_not_removed() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/main.rs";
    write_file(dir.path(), src, "fn main() {}\n");

    let pf = parsed_file_with_fn(src, "main", "src/main.rs::fn::main", 1, 1);
    insert_pf(&mut store, pf);

    let engine = RefactorEngine::new(&store, dir.path());
    let err = engine.plan_dead_code_removal("src/main.rs::fn::main");
    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("entrypoint"), "error: {msg}");
}

// ---------------------------------------------------------------------------
// 24.3 — Import cleanup tests
// ---------------------------------------------------------------------------

#[test]
fn unused_import_removed() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/lib.rs";
    let content = "use std::io::Write;\nuse std::fmt::Display;\n\nfn foo() {\n    let x: Display = todo!();\n    let _ = x;\n}\n";
    write_file(dir.path(), src, content);
    insert_pf(
        &mut store,
        parsed_file_with_fn(src, "foo", "src/lib.rs::fn::foo", 4, 7),
    );

    let engine = RefactorEngine::new(&store, dir.path());
    let plan = engine.plan_import_cleanup(src).unwrap();

    // `Write` is unused, `Display` is used.
    let removals: Vec<_> = plan
        .edits
        .iter()
        .filter(|e| e.edit_kind == RefactorEditKind::RemoveImport)
        .collect();
    assert_eq!(removals.len(), 1, "only Write should be removed");
    assert!(
        removals[0].old_text.contains("Write"),
        "wrong import removed: {:?}",
        removals[0].old_text
    );

    let result = engine.apply_import_cleanup(&plan, true).unwrap();
    assert!(result.validation.valid);
    let diff = result
        .patches
        .first()
        .map(|p| p.unified_diff.as_str())
        .unwrap_or("");
    assert!(diff.contains("-use std::io::Write"), "diff: {diff}");
}

#[test]
fn used_import_preserved() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/lib.rs";
    let content = "use std::io::Write;\nfn foo(w: &mut dyn Write) {}\n";
    write_file(dir.path(), src, content);
    insert_pf(
        &mut store,
        parsed_file_with_fn(src, "foo", "src/lib.rs::fn::foo", 2, 2),
    );

    let engine = RefactorEngine::new(&store, dir.path());
    let plan = engine.plan_import_cleanup(src).unwrap();

    assert!(
        plan.edits.is_empty(),
        "Write is used — no edits expected; got {:?}",
        plan.edits
    );
}

// ---------------------------------------------------------------------------
// 24.4 — Extract-function candidate detection
// ---------------------------------------------------------------------------

#[test]
fn extract_function_candidate_detection_basic() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/compute.rs";
    // Large function body (>20 lines) with a dense block.
    let body_lines: Vec<String> = (0..30).map(|i| format!("    let x{i} = {i};")).collect();
    let content = format!("fn large_fn() {{\n{}\n}}\n", body_lines.join("\n"));
    write_file(dir.path(), src, &content);

    let total = body_lines.len() as u32 + 2;
    let pf = parsed_file_with_fn(src, "large_fn", "src/compute.rs::fn::large_fn", 1, total);
    insert_pf(&mut store, pf);

    let engine = RefactorEngine::new(&store, dir.path());
    let candidates = engine.detect_extract_function_candidates(src).unwrap();

    assert!(!candidates.is_empty(), "expected at least one candidate");
    let c = &candidates[0];
    assert_eq!(c.file_path, src);
    assert!(c.difficulty_score > 0.0);
}

// ---------------------------------------------------------------------------
// 24.4 — Dry-run output stability
// ---------------------------------------------------------------------------

#[test]
fn dry_run_output_stable() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/lib.rs";
    let content = "fn stable() {}\n";
    write_file(dir.path(), src, content);
    let pf = parsed_file_with_fn(src, "stable", "src/lib.rs::fn::stable", 1, 1);
    insert_pf(&mut store, pf);

    let engine = RefactorEngine::new(&store, dir.path());
    let plan = engine
        .plan_rename("src/lib.rs::fn::stable", "stable_v2")
        .unwrap();

    let r1 = engine.apply_rename(&plan, true).unwrap();
    let r2 = engine.apply_rename(&plan, true).unwrap();
    assert_eq!(r1.patches.len(), r2.patches.len());
    if !r1.patches.is_empty() {
        assert_eq!(r1.patches[0].unified_diff, r2.patches[0].unified_diff);
    }
}

// ---------------------------------------------------------------------------
// 24.4 — simulate_refactor_impact
// ---------------------------------------------------------------------------

#[test]
fn simulate_impact_basic() {
    let dir = tmp_dir();
    let mut store = open_store(dir.path());

    let src = "src/lib.rs";
    write_file(dir.path(), src, "fn target() {}\n");
    let pf = parsed_file_with_fn(src, "target", "src/lib.rs::fn::target", 1, 1);
    insert_pf(&mut store, pf);

    let engine = RefactorEngine::new(&store, dir.path());
    let plan = engine
        .plan_rename("src/lib.rs::fn::target", "renamed")
        .unwrap();
    let sim = engine.simulate_refactor_impact(&plan).unwrap();

    // Safety score must be in [0, 1].
    assert!((0.0..=1.0).contains(&sim.safety_score));
}
