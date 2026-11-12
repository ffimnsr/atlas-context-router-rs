use std::fs;
use std::path::Path;
use std::process::Command;

use atlas_engine::{
    BuildOptions, DEFAULT_MEASUREMENT_SAMPLE_LIMIT, ReachabilityStatus, UpdateOptions,
    UpdateTarget, build_graph, classify_rust_file_path, measure_same_package_edges, update_graph,
};
use atlas_repo::stable_repo_id;
use atlas_store_sqlite::Store;
use camino::Utf8Path;

fn copy_dir(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create destination");
    for entry in fs::read_dir(source).expect("read fixture dir") {
        let entry = entry.expect("fixture entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("entry type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy fixture file");
        }
    }
}

fn init_git_repo(path: &Path) {
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.name", "Atlas Tests"],
        vec!["config", "user.email", "atlas-tests@example.com"],
        vec!["add", "."],
    ] {
        let status = Command::new("git")
            .args(&args)
            .current_dir(path)
            .status()
            .expect("git command");
        assert!(status.success(), "git {:?} failed", args);
    }
}

fn prepare_fixture(temp: &tempfile::TempDir, guard_enabled: bool) -> (String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("testdata")
        .join("reachability_repo");
    let repo_root = temp.path().join("repo");
    copy_dir(&fixture, &repo_root);
    if !guard_enabled {
        let atlas_dir = repo_root.join(".atlas");
        fs::create_dir_all(&atlas_dir).expect("create .atlas");
        fs::write(
            atlas_dir.join("config.toml"),
            "[reachability]\nenabled = false\n",
        )
        .expect("write config");
    }
    init_git_repo(&repo_root);
    let db_path = temp.path().join("graph.db");
    (
        repo_root.to_str().expect("utf8 repo path").to_owned(),
        db_path.to_str().expect("utf8 db path").to_owned(),
    )
}

#[test]
fn fixture_reproduces_wrong_same_package_edge_and_measurement_is_stable() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (repo_root, db_path) = prepare_fixture(&temp, false);
    let repo = Utf8Path::new(&repo_root);

    build_graph(repo, &db_path, &BuildOptions::default()).expect("build graph");
    let store = Store::open(&db_path).expect("open store");

    let edges = store.edges_by_file("src/bin/tool.rs").expect("tool edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == atlas_core::EdgeKind::Calls
                && edge.target_qn == "src/orphan.rs::fn::orphan_helper"
                && edge.confidence_tier.as_deref() == Some("same_package")
        }),
        "fixture with the guard disabled must reproduce the wrong same_package edge: {edges:?}"
    );

    let repo_id = stable_repo_id(repo);
    let first = measure_same_package_edges(&store, &repo_id, DEFAULT_MEASUREMENT_SAMPLE_LIMIT)
        .expect("first measurement");
    let second = measure_same_package_edges(&store, &repo_id, DEFAULT_MEASUREMENT_SAMPLE_LIMIT)
        .expect("second measurement");

    assert_eq!(first, second, "measurement must be byte-stable");
    assert!(first.sampled_edges >= 1);
    assert!(
        first.orphan_candidate_edges >= 1,
        "orphan candidate must be measured: {first:?}"
    );
    assert_eq!(first.measurement_source, "edges.confidence_tier");
    assert_eq!(first.sample_limit, DEFAULT_MEASUREMENT_SAMPLE_LIMIT);

    assert_eq!(
        classify_rust_file_path("src/bin/tool.rs"),
        atlas_engine::RustFileKind::Main
    );
    assert_eq!(
        classify_rust_file_path("src/orphan.rs"),
        atlas_engine::RustFileKind::Other
    );
}

#[test]
fn guard_rejects_unreachable_orphan_candidate() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (repo_root, db_path) = prepare_fixture(&temp, true);
    let repo = Utf8Path::new(&repo_root);

    build_graph(repo, &db_path, &BuildOptions::default()).expect("build graph");
    let store = Store::open(&db_path).expect("open store");

    let edges = store.edges_by_file("src/bin/tool.rs").expect("tool edges");
    let call = edges
        .iter()
        .find(|edge| edge.kind == atlas_core::EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(call.target_qn, "orphan_helper", "stale weak target cleared");
    assert!(call.confidence_tier.is_none());
    assert_eq!(call.extra_json["unresolved"], serde_json::json!(true));
    assert!(
        call.extra_json.get("reachability").is_none(),
        "rejected edges must not persist reachability provenance"
    );

    let repo_id = stable_repo_id(repo);
    let summary = atlas_engine::diagnose_reachability(
        &store,
        &repo_id,
        &atlas_engine::config::ReachabilityConfig::default(),
    )
    .expect("reachability diagnostics");
    assert!(summary.enabled);
    assert!(summary.owners_indexed >= 1);
    assert!(summary.unresolved_mods == 0);
}

#[test]
fn deleted_orphan_file_removes_stale_weak_edge() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (repo_root, db_path) = prepare_fixture(&temp, false);
    let repo = Utf8Path::new(&repo_root);

    build_graph(repo, &db_path, &BuildOptions::default()).expect("build graph");
    let store = Store::open(&db_path).expect("open store");
    let edges = store.edges_by_file("src/bin/tool.rs").expect("tool edges");
    assert!(edges.iter().any(|edge| {
        edge.kind == atlas_core::EdgeKind::Calls
            && edge.target_qn == "src/orphan.rs::fn::orphan_helper"
    }));
    drop(store);

    fs::remove_file(Path::new(&repo_root).join("src/orphan.rs")).expect("delete orphan fixture");
    update_graph(
        repo,
        &db_path,
        &UpdateOptions {
            target: UpdateTarget::WorkingTree,
            ..UpdateOptions::default()
        },
    )
    .expect("update graph");

    let store = Store::open(&db_path).expect("reopen store");
    let edges = store
        .edges_by_file("src/bin/tool.rs")
        .expect("tool edges after update");
    assert!(
        !edges
            .iter()
            .any(|edge| edge.kind == atlas_core::EdgeKind::Calls),
        "deleted candidate must not leave a stale call edge: {edges:?}"
    );
}

#[test]
fn guard_marks_reachable_weak_edge_verified() {
    let temp = tempfile::tempdir().expect("temp dir");
    let (repo_root, db_path) = prepare_fixture(&temp, true);
    let repo = Utf8Path::new(&repo_root);

    build_graph(repo, &db_path, &BuildOptions::default()).expect("build graph");
    let store = Store::open(&db_path).expect("open store");

    let edges = store.edges_by_file("src/lib.rs").expect("lib edges");
    let call = edges
        .iter()
        .find(|edge| {
            edge.kind == atlas_core::EdgeKind::Calls
                && edge.target_qn == "src/helper.rs::fn::helper_fn"
        })
        .expect("call edge");
    assert_eq!(call.confidence_tier.as_deref(), Some("same_package"));
    assert_eq!(
        call.extra_json["reachability"],
        serde_json::json!(ReachabilityStatus::Verified.as_str())
    );
}
