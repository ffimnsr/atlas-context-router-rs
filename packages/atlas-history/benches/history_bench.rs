//! Historical graph performance checks for Phase 17.12.
//!
//! Run with:
//!   cargo bench -p atlas-history --bench history_bench

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use atlas_history::{
    CommitSelector, build_historical_graph, compute_churn_report, diff_snapshots,
    query_symbol_history, reconstruct_snapshot,
};
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::Store;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

const GIT_TEST_NAME: &str = "Atlas Bench";
const GIT_TEST_EMAIL: &str = "bench@atlas";
const GIT_LOCAL_ENV_VARS: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_KEY_0",
    "GIT_CONFIG_VALUE_0",
    "GIT_DIR",
    "GIT_GRAFT_FILE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_INTERNAL_SUPER_PREFIX",
    "GIT_NAMESPACE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_OBJECT_DIRECTORY",
    "GIT_PREFIX",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
    "GIT_WORK_TREE",
];

struct BenchFixture {
    _repo_dir: tempfile::TempDir,
    _store_dir: tempfile::TempDir,
    canonical_root: String,
    store_path: PathBuf,
    commit_shas: Vec<String>,
}

impl BenchFixture {
    fn repo_path(&self) -> &Path {
        Path::new(&self.canonical_root)
    }

    fn open_store(&self) -> Store {
        Store::open(self.store_path.to_str().expect("store path string")).expect("open bench store")
    }
}

fn sanitized_git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir);
    for var in GIT_LOCAL_ENV_VARS {
        cmd.env_remove(var);
    }
    cmd.env("GIT_AUTHOR_NAME", GIT_TEST_NAME);
    cmd.env("GIT_AUTHOR_EMAIL", GIT_TEST_EMAIL);
    cmd.env("GIT_COMMITTER_NAME", GIT_TEST_NAME);
    cmd.env("GIT_COMMITTER_EMAIL", GIT_TEST_EMAIL);
    cmd
}

fn git(dir: &Path, args: &[&str]) {
    let output = sanitized_git(dir).args(args).output().expect("git command");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(dir: &Path, args: &[&str]) -> String {
    let output = sanitized_git(dir).args(args).output().expect("git output");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).expect("utf8")
}

fn git_init(dir: &Path) {
    git(dir, &["init", "--quiet"]);
    git(dir, &["config", "user.email", GIT_TEST_EMAIL]);
    git(dir, &["config", "user.name", GIT_TEST_NAME]);
    git(dir, &["branch", "-M", "main"]);
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdirs");
    }
    fs::write(path, content).expect("write file");
}

fn commit_all(root: &Path, message: &str) -> String {
    git(root, &["add", "-A"]);
    git(root, &["commit", "--quiet", "-m", message]);
    git_output(root, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn build_commit_content(version: usize) -> String {
    let extra_helper = if version > 0 {
        format!(
            "\npub fn helper_{version}(name: &str) -> String {{\n    helper(name, \"v{version}\")\n}}\n"
        )
    } else {
        String::new()
    };
    format!(
        r#"pub struct Greeter {{
    times: usize,
}}

impl Greeter {{
    pub fn greet_twice(name: &str) -> String {{
        Self::new({times}).render(name)
    }}

    pub fn new(times: usize) -> Self {{
        Self {{ times }}
    }}

    pub fn render(&self, name: &str) -> String {{
        format!("Hello, {{name}}! Hello again, {{name}}! x{{}}", self.times)
    }}
}}

pub fn helper(name: &str, suffix: &str) -> String {{
    let greeting = Greeter::greet_twice(name);
    format!("{{greeting}} [{{suffix}} v{version}]")
}}
{extra_helper}
"#,
        times = (version % 3) + 2,
        extra_helper = extra_helper,
    )
}

fn make_history_fixture(commit_count: usize, dedup_friendly: bool) -> BenchFixture {
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    git_init(repo_dir.path());
    let mut commit_shas = Vec::with_capacity(commit_count.max(1));

    write_file(repo_dir.path(), "src/lib.rs", &build_commit_content(0));
    write_file(repo_dir.path(), "notes.txt", "fixture 0\n");
    commit_shas.push(commit_all(repo_dir.path(), "bench commit 0"));

    for version in 1..commit_count {
        if dedup_friendly {
            write_file(
                repo_dir.path(),
                "notes.txt",
                &format!("fixture {version}\n"),
            );
        } else {
            write_file(
                repo_dir.path(),
                "src/lib.rs",
                &build_commit_content(version),
            );
        }
        commit_shas.push(commit_all(
            repo_dir.path(),
            &format!("bench commit {version}"),
        ));
    }

    let store_dir = tempfile::tempdir().expect("store tempdir");
    let store_path = store_dir.path().join("history.sqlite");
    let canonical_root = std::fs::canonicalize(repo_dir.path())
        .expect("canonical repo")
        .to_string_lossy()
        .into_owned();

    BenchFixture {
        _repo_dir: repo_dir,
        _store_dir: store_dir,
        canonical_root,
        store_path,
        commit_shas,
    }
}

fn explicit_selector(commit_shas: &[String]) -> CommitSelector {
    CommitSelector::Explicit {
        shas: commit_shas.to_vec(),
    }
}

fn seed_history(fixture: &BenchFixture) {
    let store = fixture.open_store();
    let registry = ParserRegistry::with_defaults();
    build_historical_graph(
        fixture.repo_path(),
        &fixture.canonical_root,
        &store,
        &explicit_selector(&fixture.commit_shas),
        &registry,
        Some("bench"),
    )
    .expect("seed bench history");
}

fn bench_history_build_commits_per_second(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    let mut group = c.benchmark_group("history/build_commits_per_second");

    for commit_count in [10usize, 25, 50] {
        let fixture = make_history_fixture(commit_count, false);
        group.throughput(Throughput::Elements(commit_count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(commit_count),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let store = fixture.open_store();
                    build_historical_graph(
                        fixture.repo_path(),
                        &fixture.canonical_root,
                        &store,
                        &explicit_selector(&fixture.commit_shas),
                        &registry,
                        Some("bench"),
                    )
                    .expect("build bench history")
                })
            },
        );
    }

    group.finish();
}

fn bench_snapshot_reconstruction_speed(c: &mut Criterion) {
    let fixture = make_history_fixture(30, false);
    seed_history(&fixture);
    let latest_sha = fixture.commit_shas.last().expect("latest sha").clone();
    let store = fixture.open_store();

    c.bench_function("history/snapshot_reconstruction_speed", |b| {
        b.iter(|| {
            reconstruct_snapshot(&store, &fixture.canonical_root, &latest_sha)
                .expect("reconstruct snapshot")
        })
    });
}

fn bench_graph_diff_speed(c: &mut Criterion) {
    let fixture = make_history_fixture(30, false);
    seed_history(&fixture);
    let store = fixture.open_store();
    let left = fixture.commit_shas[fixture.commit_shas.len() - 2].clone();
    let right = fixture.commit_shas.last().expect("right sha").clone();

    c.bench_function("history/graph_diff_speed", |b| {
        b.iter(|| {
            diff_snapshots(
                fixture.repo_path(),
                &store,
                &fixture.canonical_root,
                &left,
                &right,
            )
            .expect("diff snapshots")
        })
    });
}

fn bench_symbol_history_query_latency(c: &mut Criterion) {
    let fixture = make_history_fixture(30, false);
    seed_history(&fixture);
    let store = fixture.open_store();

    c.bench_function("history/symbol_history_query_latency", |b| {
        b.iter(|| {
            query_symbol_history(&store, &fixture.canonical_root, "src/lib.rs::fn::helper")
                .expect("query symbol history")
        })
    });
}

fn bench_storage_growth_with_and_without_deduplication(c: &mut Criterion) {
    let unique_fixture = make_history_fixture(30, false);
    seed_history(&unique_fixture);
    let dedup_fixture = make_history_fixture(30, true);
    seed_history(&dedup_fixture);
    let unique_store = unique_fixture.open_store();
    let dedup_store = dedup_fixture.open_store();
    let mut group = c.benchmark_group("history/storage_growth_with_and_without_deduplication");
    group.throughput(Throughput::Elements(30));

    group.bench_function("unique", |b| {
        b.iter(|| {
            compute_churn_report(
                &unique_store,
                &unique_fixture.canonical_root,
                unique_fixture.store_path.to_str().expect("db path"),
            )
            .expect("unique churn report")
        })
    });
    group.bench_function("deduplicated", |b| {
        b.iter(|| {
            compute_churn_report(
                &dedup_store,
                &dedup_fixture.canonical_root,
                dedup_fixture.store_path.to_str().expect("db path"),
            )
            .expect("dedup churn report")
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_history_build_commits_per_second,
    bench_snapshot_reconstruction_speed,
    bench_graph_diff_speed,
    bench_symbol_history_query_latency,
    bench_storage_growth_with_and_without_deduplication,
);
criterion_main!(benches);
