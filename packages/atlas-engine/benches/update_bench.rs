use std::fs;
use std::process::Command;

use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_store_sqlite::Store;
use camino::Utf8PathBuf;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

struct BenchFixture {
    _repo_dir: tempfile::TempDir,
    _store_dir: tempfile::TempDir,
    repo_root: Utf8PathBuf,
    db_path: String,
    module_count: usize,
    module_paths: Vec<String>,
}

fn write_file(root: &Utf8PathBuf, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, content).expect("write fixture file");
}

fn init_git_repo(root: &Utf8PathBuf) {
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .status()
        .expect("git init");
    assert!(status.success(), "git init should succeed");
}

fn workspace_manifest() -> &'static str {
    r#"[package]
name = "bench-fixture"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#
}

fn lib_source(module_count: usize) -> String {
    let mut src = String::new();
    for idx in 0..module_count {
        src.push_str(&format!("pub mod module_{idx};\n"));
    }
    src.push_str("\npub fn dispatch(input: usize) -> usize {\n");
    src.push_str("    module_0::chain_0(input)\n");
    src.push_str("}\n");
    src
}

fn module_source(index: usize, module_count: usize, version: usize) -> String {
    let next = index + 1;
    let next_use = if next < module_count {
        format!("use crate::module_{next}::chain_{next};\n\n")
    } else {
        String::new()
    };
    let tail_call = if next < module_count {
        format!("    chain_{next}(local)\n")
    } else {
        "    local\n".to_owned()
    };
    format!(
        r#"{next_use}pub fn helper_{index}(value: usize) -> usize {{
    value + {index} + {version}
}}

pub fn chain_{index}(value: usize) -> usize {{
    let local = helper_{index}(value);
{tail_call}}}
"#,
        next_use = next_use,
        tail_call = tail_call,
    )
}

fn make_fixture(module_count: usize) -> BenchFixture {
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let repo_root = Utf8PathBuf::from_path_buf(repo_dir.path().to_path_buf()).expect("utf8 path");
    let store_dir = tempfile::tempdir().expect("store tempdir");
    let db_path = store_dir.path().join("atlas.sqlite");
    let db_path = db_path.to_string_lossy().into_owned();

    write_file(&repo_root, "Cargo.toml", workspace_manifest());
    write_file(&repo_root, "src/lib.rs", &lib_source(module_count));

    let module_paths: Vec<String> = (0..module_count)
        .map(|idx| format!("src/module_{idx}.rs"))
        .collect();
    for (idx, path) in module_paths.iter().enumerate() {
        write_file(&repo_root, path, &module_source(idx, module_count, 1));
    }
    init_git_repo(&repo_root);

    let mut store = Store::open(&db_path).expect("open bench db");
    store.migrate().expect("migrate bench db");

    build_graph(&repo_root, &db_path, &BuildOptions::default()).expect("initial build");

    BenchFixture {
        _repo_dir: repo_dir,
        _store_dir: store_dir,
        repo_root,
        db_path,
        module_count,
        module_paths,
    }
}

fn bench_incremental_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("engine/incremental_update");

    for touched_count in [1usize, 4, 8] {
        let fixture = make_fixture(24);
        let touched: Vec<(usize, String)> = fixture
            .module_paths
            .iter()
            .take(touched_count)
            .enumerate()
            .map(|(idx, path)| (idx, path.clone()))
            .collect();
        let mut version = 2usize;

        group.bench_with_input(
            BenchmarkId::from_parameter(touched_count),
            &touched,
            |b, touched| {
                b.iter(|| {
                    for (idx, path) in touched {
                        write_file(
                            &fixture.repo_root,
                            path,
                            &module_source(*idx, fixture.module_count, version),
                        );
                    }

                    update_graph(
                        &fixture.repo_root,
                        &fixture.db_path,
                        &UpdateOptions {
                            target: UpdateTarget::Files(
                                touched.iter().map(|(_, path)| path.clone()).collect(),
                            ),
                            ..Default::default()
                        },
                    )
                    .expect("incremental update");

                    version += 1;
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_incremental_update);
criterion_main!(benches);
