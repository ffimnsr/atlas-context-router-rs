use std::path::{Path, PathBuf};

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use atlas_refactor::RefactorEngine;
use atlas_store_sqlite::Store;
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use tempfile::TempDir;

fn make_store(db_path: &Path) -> Store {
    let mut store = Store::open(db_path.to_str().expect("utf-8 db path")).expect("open store");
    store.migrate().expect("migrate store");
    store
}

fn write_file(repo_root: &Path, relative_path: &str, content: &str) {
    let path = repo_root.join(relative_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(path, content).expect("write fixture file");
}

fn make_node(file_path: &str, qualified_name: String, name: String, line_start: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name,
        qualified_name,
        file_path: file_path.to_owned(),
        line_start,
        line_end: line_start,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "bench-hash".to_owned(),
        extra_json: serde_json::Value::Null,
    }
}

fn make_edge(file_path: &str, source_qn: String, target_qn: String, line: u32) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn,
        target_qn,
        file_path: file_path.to_owned(),
        line: Some(line),
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::Value::Null,
    }
}

struct RenameFixture {
    _dir: TempDir,
    store: Store,
    target_qname: String,
}

fn setup_rename_fixture(caller_count: usize) -> RenameFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("rename.db");
    let mut store = make_store(&db_path);
    let mut parsed_files = Vec::with_capacity(caller_count + 1);

    let target_file = "src/lib.rs";
    let target_qname = "src/lib.rs::fn::target_fn".to_owned();
    write_file(
        dir.path(),
        target_file,
        "pub fn target_fn() {}\npub fn helper() {}\n",
    );
    parsed_files.push(ParsedFile {
        path: target_file.to_owned(),
        language: Some("rust".to_owned()),
        hash: "rename-target".to_owned(),
        size: None,
        nodes: vec![
            make_node(target_file, target_qname.clone(), "target_fn".to_owned(), 1),
            make_node(
                target_file,
                "src/lib.rs::fn::helper".to_owned(),
                "helper".to_owned(),
                2,
            ),
        ],
        edges: vec![],
    });

    for caller_idx in 0..caller_count {
        let file_path = format!("src/caller_{caller_idx}.rs");
        let caller_qname = format!("{file_path}::fn::caller_{caller_idx}");
        let content =
            format!("use crate::target_fn;\npub fn caller_{caller_idx}() {{ target_fn(); }}\n");
        write_file(dir.path(), &file_path, &content);
        parsed_files.push(ParsedFile {
            path: file_path.clone(),
            language: Some("rust".to_owned()),
            hash: format!("rename-caller-{caller_idx}"),
            size: None,
            nodes: vec![make_node(
                &file_path,
                caller_qname.clone(),
                format!("caller_{caller_idx}"),
                2,
            )],
            edges: vec![make_edge(&file_path, caller_qname, target_qname.clone(), 2)],
        });
    }

    store
        .replace_files_transactional(&parsed_files)
        .expect("seed rename fixture");

    RenameFixture {
        _dir: dir,
        store,
        target_qname,
    }
}

struct ImportCleanupFixture {
    _dir: TempDir,
    store: Store,
    import_file: String,
}

fn build_import_cleanup_source(import_count: usize) -> String {
    let mut content = String::new();
    for import_idx in 0..import_count {
        content.push_str(&format!("use crate::dep_{import_idx}::Type{import_idx};\n"));
    }
    content.push('\n');
    content.push_str("pub fn exercise_imports() {\n");
    for import_idx in (0..import_count).step_by(3) {
        content.push_str(&format!(
            "    let _value_{import_idx} = Type{import_idx}::default();\n"
        ));
    }
    content.push_str("}\n");
    content
}

fn setup_import_cleanup_fixture(import_count: usize) -> ImportCleanupFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path: PathBuf = dir.path().join("imports.db");
    let store = make_store(&db_path);
    let import_file = "src/imports.rs".to_owned();
    let content = build_import_cleanup_source(import_count);
    write_file(dir.path(), &import_file, &content);

    ImportCleanupFixture {
        _dir: dir,
        store,
        import_file,
    }
}

fn bench_rename_planning_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("refactor/rename_planning_latency");
    for caller_count in [64usize, 256] {
        let mut fixture = setup_rename_fixture(caller_count);
        let target_qname = fixture.target_qname.clone();
        let repo_root = fixture._dir.path().to_path_buf();
        let engine = RefactorEngine::new(&mut fixture.store, repo_root.as_path());

        group.bench_with_input(
            BenchmarkId::from_parameter(caller_count),
            &target_qname,
            |b, target_qname| {
                b.iter(|| {
                    black_box(
                        engine
                            .plan_rename(black_box(target_qname.as_str()), "renamed_target_fn")
                            .expect("plan rename"),
                    );
                });
            },
        );
    }
    group.finish();
}

fn bench_import_cleanup_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("refactor/import_cleanup_latency");
    for import_count in [128usize, 512] {
        let mut fixture = setup_import_cleanup_fixture(import_count);
        let import_file = fixture.import_file.clone();
        let repo_root = fixture._dir.path().to_path_buf();
        let engine = RefactorEngine::new(&mut fixture.store, repo_root.as_path());

        group.bench_with_input(
            BenchmarkId::from_parameter(import_count),
            &import_file,
            |b, import_file| {
                b.iter(|| {
                    black_box(
                        engine
                            .plan_import_cleanup(black_box(import_file.as_str()))
                            .expect("plan import cleanup"),
                    );
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_rename_planning_latency,
    bench_import_cleanup_latency,
);
criterion_main!(benches);
