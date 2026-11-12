//! Emit the deterministic weak-edge reachability measurement for the fixture.
//!
//! Run with:
//! `cargo run --release -p atlas-engine --bench reachability_bench`

use std::fs;
use std::path::Path;
use std::process::Command;

use atlas_engine::{
    BuildOptions, DEFAULT_MEASUREMENT_SAMPLE_LIMIT, build_graph, measure_same_package_edges,
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
        vec!["config", "user.name", "Atlas Bench"],
        vec!["config", "user.email", "atlas-bench@example.com"],
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

fn main() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("testdata")
        .join("reachability_repo");
    let temp = tempfile::tempdir().expect("temp dir");
    let repo_root = temp.path().join("repo");
    copy_dir(&fixture, &repo_root);
    let atlas_dir = repo_root.join(".atlas");
    fs::create_dir_all(&atlas_dir).expect("create .atlas");
    fs::write(
        atlas_dir.join("config.toml"),
        "[reachability]\nenabled = false\n",
    )
    .expect("write config");
    init_git_repo(&repo_root);

    let repo = Utf8Path::from_path(&repo_root).expect("utf8 repo path");
    let db_path = temp.path().join("graph.db");
    let db_path = db_path.to_str().expect("utf8 db path");

    build_graph(repo, db_path, &BuildOptions::default()).expect("build graph");
    let store = Store::open(db_path).expect("open store");
    let repo_id = stable_repo_id(repo);
    let measurement =
        measure_same_package_edges(&store, &repo_id, DEFAULT_MEASUREMENT_SAMPLE_LIMIT)
            .expect("measure same_package edges");

    println!(
        "{}",
        serde_json::to_string_pretty(&measurement).expect("serialize measurement")
    );
}
