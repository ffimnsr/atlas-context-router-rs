use super::*;
use crate::{BuildOptions, BuildRunBudget, build_graph};
use atlas_core::BudgetStatus;
use atlas_core::model::{ChangeType, ChangedFile};
use atlas_parser::ExternalParserConfig;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Atlas Test")
        .env("GIT_AUTHOR_EMAIL", "test@atlas")
        .env("GIT_COMMITTER_NAME", "Atlas Test")
        .env("GIT_COMMITTER_EMAIL", "test@atlas")
        .status()
        .expect("git command");
    assert!(status.success(), "git {args:?} failed");
}

#[test]
fn update_graph_blocks_when_parse_failure_budget_is_exceeded() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("lib.rs"), "pub fn ok() {}\n").unwrap();
    git(repo_root, &["add", "lib.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();

    let lib_path = repo_root.join("lib.rs");
    let mut perms = std::fs::metadata(&lib_path).unwrap().permissions();
    perms.set_mode(0o000);
    std::fs::set_permissions(&lib_path, perms).unwrap();

    let budget = BuildRunBudget {
        max_parse_failures: 0,
        max_parse_failure_ratio_bps: 10_000,
        ..BuildRunBudget::default()
    };

    let summary = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: false,
            dry_run: false,
            batch_size: 16,
            target: UpdateTarget::WorkingTree,
            budget,
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();

    assert_eq!(summary.budget.budget_status, BudgetStatus::Blocked);
    assert_eq!(summary.parse_errors, 1);
    assert_eq!(
        summary.budget_counters.budget_stop_reason.as_deref(),
        Some("max_parse_failures")
    );
}

#[test]
fn update_graph_rejects_missing_parser_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("lib.rs"), "pub fn ok() {}\n").unwrap();
    git(repo_root, &["add", "lib.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    drop(Store::open(db_path.to_str().unwrap()).unwrap());

    let error = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            target: UpdateTarget::Files(vec!["lib.rs".to_owned()]),
            ..UpdateOptions::default()
        },
    )
    .expect_err("missing fingerprint must fail closed");
    assert!(
        error.to_string().contains("no parser fingerprint"),
        "unexpected error: {error}"
    );
}

#[test]
fn update_graph_rejects_parser_fingerprint_mismatch() {
    use atlas_parser::ExternalExtractor;

    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("lib.rs"), "pub fn ok() {}\n").unwrap();
    git(repo_root, &["add", "lib.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions::default(),
    )
    .unwrap();

    let error = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            target: UpdateTarget::Files(vec!["lib.rs".to_owned()]),
            external_parsers: vec![ExternalParserConfig {
                extractor: Some(ExternalExtractor::Php),
                grammar_dir: Some("grammars/tree-sitter-php".to_owned()),
                ..Default::default()
            }],
            ..UpdateOptions::default()
        },
    )
    .expect_err("fingerprint mismatch must fail closed");
    assert!(
        error.to_string().contains("parser configuration changed"),
        "unexpected error: {error}"
    );
}

#[test]
fn update_graph_registers_repo_root_for_repo_scoped_reads() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("lib.rs"), "pub fn ok() {}\n").unwrap();
    git(repo_root, &["add", "lib.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();

    std::fs::write(
        repo_root.join("lib.rs"),
        "pub fn ok() {}\npub fn later() {}\n",
    )
    .unwrap();

    let summary = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();

    assert_eq!(summary.parsed, 1);
    let store = Store::open(db_path.to_str().unwrap()).unwrap();
    assert!(
        store
            .find_repo_id(repo_root.to_str().unwrap())
            .unwrap()
            .is_some(),
        "incremental update must keep repo root registered in repos table"
    );
}

// --- parallel parse → sequential write boundary --------------------------
//
// Verifies that the incremental update pipeline collects all parallel parse
// results for changed files before writing anything to the store.
// A batch_size of 1 forces one file per Rayon batch; all `ParsedFile`
// values from the changed-file phase are collected into `parsed_changed`
// before the sequential write phase begins.  Both updated files must appear
// correctly in the store after the update regardless of Rayon scheduling.
#[test]
fn body_only_update_preserves_cross_file_caller_edge() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("lib.rs"), "mod a;\nmod b;\n").unwrap();
    std::fs::write(
        repo_root.join("a.rs"),
        "pub fn target() -> i32 {\n    1\n}\n",
    )
    .unwrap();
    std::fs::write(
        repo_root.join("b.rs"),
        "use crate::a::target;\n\npub fn caller() -> i32 {\n    target()\n}\n",
    )
    .unwrap();
    git(repo_root, &["add", "lib.rs", "a.rs", "b.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions::default(),
    )
    .unwrap();

    let caller_edge_exists = || {
        Store::open(db_path.to_str().unwrap())
            .unwrap()
            .edges_by_file("b.rs")
            .unwrap()
            .into_iter()
            .any(|edge| {
                edge.source_qn.ends_with("::fn::caller") && edge.target_qn.ends_with("::fn::target")
            })
    };
    assert!(
        caller_edge_exists(),
        "build must resolve b.rs caller to target"
    );

    std::fs::write(
        repo_root.join("a.rs"),
        "pub fn target() -> i32 {\n    2\n}\n",
    )
    .unwrap();
    let summary = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();

    assert_eq!(summary.parsed, 1, "body-only edit must not reparse caller");
    assert!(
        caller_edge_exists(),
        "body-only target replacement must retain unchanged caller edge"
    );
}

#[test]
fn update_changed_files_parallel_parse_completes_before_store_write() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("a.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::write(repo_root.join("b.rs"), "pub fn beta() {}\n").unwrap();
    std::fs::write(repo_root.join("c.rs"), "pub fn gamma() {}\n").unwrap();
    git(repo_root, &["add", "a.rs", "b.rs", "c.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions::default(),
    )
    .unwrap();

    // Modify two files in the working tree, each gaining a new function.
    std::fs::write(
        repo_root.join("a.rs"),
        "pub fn alpha() {}\npub fn alpha2() {}\n",
    )
    .unwrap();
    std::fs::write(
        repo_root.join("b.rs"),
        "pub fn beta() {}\npub fn beta2() {}\n",
    )
    .unwrap();

    let summary = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        // batch_size=1 forces one file per Rayon batch so both parse
        // results are collected before the sequential write phase.
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 1,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();

    assert_eq!(
        summary.parsed, 2,
        "both modified files must be parsed in the update"
    );

    // New functions from both parallel-parsed files must be in the store.
    // If the write phase ran before parse collection finished, one file's
    // new nodes would be missing.
    let store = Store::open(db_path.to_str().unwrap()).unwrap();
    let a_sigs = store.node_signatures_by_file("a.rs").unwrap();
    assert!(
        a_sigs.contains_key("a.rs::fn::alpha2"),
        "a.rs::fn::alpha2 must be in the store after update"
    );
    let b_sigs = store.node_signatures_by_file("b.rs").unwrap();
    assert!(
        b_sigs.contains_key("b.rs::fn::beta2"),
        "b.rs::fn::beta2 must be in the store after update"
    );
    // Unchanged file must be intact.
    let c_sigs = store.node_signatures_by_file("c.rs").unwrap();
    assert!(
        c_sigs.contains_key("c.rs::fn::gamma"),
        "c.rs::fn::gamma must still be present after update"
    );
}

#[test]
fn copied_file_update_keeps_source_graph() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("base.rs"), "pub fn base() {}\n").unwrap();
    git(repo_root, &["add", "base.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions::default(),
    )
    .unwrap();

    std::fs::write(
        repo_root.join("copy.rs"),
        "pub fn base() {}\npub fn copy() {}\n",
    )
    .unwrap();

    let summary = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            target: UpdateTarget::Batch(vec![ChangedFile {
                path: "copy.rs".to_string(),
                change_type: ChangeType::Copied,
                old_path: Some("base.rs".to_string()),
            }]),
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();

    assert_eq!(
        summary.deleted, 0,
        "copy updates must not delete source graph"
    );

    let store = Store::open(db_path.to_str().unwrap()).unwrap();
    let base_sigs = store.node_signatures_by_file("base.rs").unwrap();
    let copy_sigs = store.node_signatures_by_file("copy.rs").unwrap();

    assert!(
        base_sigs.contains_key("base.rs::fn::base"),
        "source file graph must remain after copied-file update"
    );
    assert!(
        copy_sigs.contains_key("copy.rs::fn::copy"),
        "copied file graph must be indexed"
    );
}

// Verifies that dependent file parse results are collected before the write
// phase executes.  Files passed explicitly via `UpdateTarget::Files` all go
// through the changed-file parallel parse phase; the write phase must see
// all of them regardless of Rayon scheduling order.
//
// Note: the dependency-detection path (phase 2 / `parsed_deps`) follows the
// same collect-then-write pattern as phase 1; that structural guarantee
// holds for both phases because both populate a Vec<ParsedFile> before the
// single sequential write loop at the end of `update_graph`.
#[test]
fn update_explicit_file_list_parse_completes_before_store_write() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("x.rs"), "pub fn x1() {}\n").unwrap();
    std::fs::write(repo_root.join("y.rs"), "pub fn y1() {}\n").unwrap();
    git(repo_root, &["add", "x.rs", "y.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions::default(),
    )
    .unwrap();

    // Modify both files; pass them explicitly to exercise the Files path.
    std::fs::write(repo_root.join("x.rs"), "pub fn x1() {}\npub fn x2() {}\n").unwrap();
    std::fs::write(repo_root.join("y.rs"), "pub fn y1() {}\npub fn y2() {}\n").unwrap();

    let summary = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 1,
            target: UpdateTarget::Files(vec!["x.rs".to_string(), "y.rs".to_string()]),
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();

    assert_eq!(
        summary.parsed, 2,
        "both explicitly listed files must be parsed"
    );

    let store = Store::open(db_path.to_str().unwrap()).unwrap();
    assert!(
        store
            .node_signatures_by_file("x.rs")
            .unwrap()
            .contains_key("x.rs::fn::x2"),
        "x.rs::fn::x2 must be in the store after explicit-file update"
    );
    assert!(
        store
            .node_signatures_by_file("y.rs")
            .unwrap()
            .contains_key("y.rs::fn::y2"),
        "y.rs::fn::y2 must be in the store after explicit-file update"
    );
}

#[test]
fn update_summary_marks_degraded_for_reconcile_failures() {
    let summary = UpdateSummary {
        call_target_reconcile_failures: 1,
        ..UpdateSummary::default()
    };

    assert!(summary.is_degraded());
}

// Regression for committed-refactor staleness: the default WorkingTree
// target must pick up changes that were already committed, because it
// diffs against the last indexed ref recorded by the previous build/
// update instead of only the git index.
#[test]
fn update_default_target_reparses_committed_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("lib.rs"), "pub fn keep() {}\n").unwrap();
    git(repo_root, &["add", "lib.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions::default(),
    )
    .unwrap();

    // Commit the refactor: old function pruned, new module file added.
    std::fs::write(repo_root.join("lib.rs"), "pub fn slim() {}\n").unwrap();
    std::fs::write(repo_root.join("extra.rs"), "pub fn extra() {}\n").unwrap();
    git(repo_root, &["add", "lib.rs", "extra.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "refactor"]);

    let summary = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();
    assert!(
        summary.parsed >= 2,
        "committed refactor must be parsed, parsed={}",
        summary.parsed
    );

    let store = Store::open(db_path.to_str().unwrap()).unwrap();
    let lib_sigs = store.node_signatures_by_file("lib.rs").unwrap();
    assert!(
        lib_sigs.contains_key("lib.rs::fn::slim"),
        "refactored lib.rs symbols must be in the graph"
    );
    assert!(
        !lib_sigs.contains_key("lib.rs::fn::keep"),
        "pruned function must leave the graph"
    );
    assert!(
        store
            .node_signatures_by_file("extra.rs")
            .unwrap()
            .contains_key("extra.rs::fn::extra"),
        "added module file must be in the graph"
    );
}

#[test]
fn update_default_target_advances_indexed_ref_and_noops_when_clean() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("lib.rs"), "pub fn one() {}\n").unwrap();
    git(repo_root, &["add", "lib.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions::default(),
    )
    .unwrap();

    // Commit a change, then sync it with the default target.
    std::fs::write(
        repo_root.join("lib.rs"),
        "pub fn one() {}\npub fn two() {}\n",
    )
    .unwrap();
    git(repo_root, &["add", "lib.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "add two"]);

    let first = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();
    assert_eq!(first.parsed, 1, "committed change must be parsed once");

    let store = Store::open(db_path.to_str().unwrap()).unwrap();
    let head = head_ref(Utf8Path::from_path(repo_root).unwrap()).unwrap();
    assert_eq!(
        store
            .last_indexed_ref(repo_root.to_str().unwrap())
            .unwrap()
            .as_deref(),
        Some(head.as_str()),
        "clean update must advance the indexed ref to HEAD"
    );

    let second = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();
    assert_eq!(second.parsed, 0, "clean tree after sync must be a no-op");
    assert_eq!(second.deleted, 0, "clean tree must not delete anything");
}

#[test]
fn update_default_target_still_sees_unstaged_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo_root = dir.path();

    git(repo_root, &["init", "--quiet"]);
    std::fs::write(repo_root.join("lib.rs"), "pub fn base() {}\n").unwrap();
    git(repo_root, &["add", "lib.rs"]);
    git(repo_root, &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db");
    build_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &BuildOptions::default(),
    )
    .unwrap();

    // Unstaged edit: must still be parsed by the default target even
    // though the diff now compares the worktree against HEAD.
    std::fs::write(
        repo_root.join("lib.rs"),
        "pub fn base() {}\npub fn draft() {}\n",
    )
    .unwrap();

    let summary = update_graph(
        Utf8Path::from_path(repo_root).unwrap(),
        db_path.to_str().unwrap(),
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: 16,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: vec![],
        },
    )
    .unwrap();
    assert_eq!(summary.parsed, 1, "unstaged edit must be parsed");

    let store = Store::open(db_path.to_str().unwrap()).unwrap();
    assert!(
        store
            .node_signatures_by_file("lib.rs")
            .unwrap()
            .contains_key("lib.rs::fn::draft"),
        "unstaged edit symbols must be in the graph"
    );
}
