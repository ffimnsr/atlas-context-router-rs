#![no_main]

use arbitrary::Arbitrary;
use atlas_engine::{
    BuildOptions, BuildRunBudget, UpdateOptions, UpdateTarget, build_graph, update_graph,
};
use atlas_fuzz::SupportedPathKind;
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::Store;
use camino::{Utf8Path, Utf8PathBuf};
use libfuzzer_sys::fuzz_target;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::Command;
use tempfile::TempDir;

const MAX_INITIAL_FILES: usize = 8;
const MAX_MUTATIONS: usize = 24;
const MAX_SOURCE_BYTES: usize = 4096;
const SLOT_COUNT: u8 = 8;
const SEED_SLOT: u8 = 250;

#[derive(Arbitrary, Debug, Clone, Copy)]
enum UpdatePathKind {
    Supported(SupportedPathKind),
    UnsupportedText,
    UnsupportedYaml,
    UnsupportedBinary,
}

impl UpdatePathKind {
    fn rel_path(self, slot: u8) -> String {
        match self {
            Self::Supported(kind) => format!("src/fuzz_{slot}.{}", kind.extension()),
            Self::UnsupportedText => format!("src/fuzz_{slot}.txt"),
            Self::UnsupportedYaml => format!("src/fuzz_{slot}.yaml"),
            Self::UnsupportedBinary => format!("src/fuzz_{slot}.bin"),
        }
    }

    fn is_supported(self) -> bool {
        matches!(self, Self::Supported(_))
    }
}

#[derive(Arbitrary, Debug, Clone)]
struct InitialFile {
    slot: u8,
    path_kind: UpdatePathKind,
    source: Vec<u8>,
}

#[derive(Arbitrary, Debug, Clone)]
enum FileMutation {
    Add {
        slot: u8,
        path_kind: UpdatePathKind,
        source: Vec<u8>,
    },
    Modify {
        slot: u8,
        source: Vec<u8>,
    },
    Delete {
        slot: u8,
    },
    Rename {
        slot: u8,
        new_path_kind: UpdatePathKind,
    },
}

#[derive(Arbitrary, Debug)]
struct UpdateGraphCase {
    initial_files: Vec<InitialFile>,
    mutations: Vec<FileMutation>,
    batch_size: u8,
}

#[derive(Clone, Debug)]
struct LiveFile {
    path: String,
    path_kind: UpdatePathKind,
}

#[derive(Debug, Default)]
struct MutationLog {
    deleted_paths: BTreeSet<String>,
    explicit_candidate_paths: BTreeSet<String>,
    renamed_paths: Vec<RenamedPath>,
    unsupported_explicit_paths: BTreeSet<String>,
}

#[derive(Debug)]
struct RenamedPath {
    old_path: String,
    new_path: String,
    new_supported: bool,
}

struct Fixture {
    _temp_dir: TempDir,
    repo_root: Utf8PathBuf,
    db_path: String,
    live_files: BTreeMap<u8, LiveFile>,
    log: MutationLog,
}

fuzz_target!(|case: UpdateGraphCase| {
    run_working_tree_case(&case);
    run_explicit_file_case(&case);
});

fn run_working_tree_case(case: &UpdateGraphCase) {
    let fixture = prepare_fixture(case);
    let summary = update_graph(
        fixture.repo_root.as_ref(),
        &fixture.db_path,
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: normalized_batch_size(case.batch_size),
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
        },
    )
    .expect("working-tree update must stay bounded for benign malformed input");

    let store = Store::open(&fixture.db_path).expect("open store after working-tree update");
    for deleted_path in &fixture.log.deleted_paths {
        assert_eq!(
            store.file_hash(deleted_path).expect("read deleted file hash"),
            None,
            "deleted path must not remain indexed"
        );
        assert!(
            store
                .node_signatures_by_file(deleted_path)
                .expect("read deleted file signatures")
                .is_empty(),
            "deleted path must not retain node rows"
        );
    }

    for renamed in &fixture.log.renamed_paths {
        assert_eq!(
            store.file_hash(&renamed.old_path).expect("read renamed old hash"),
            None,
            "renamed old path must not remain indexed"
        );
        if renamed.new_supported {
            assert!(
                store
                    .file_hash(&renamed.new_path)
                    .expect("read renamed new hash")
                    .is_some(),
                "renamed supported path must be indexed under its new key"
            );
        }
    }

    if !fixture.log.deleted_paths.is_empty() || !fixture.log.renamed_paths.is_empty() {
        assert!(
            summary.deleted + summary.renamed > 0,
            "working-tree update must report delete or rename activity when such mutations were applied"
        );
    }
}

fn run_explicit_file_case(case: &UpdateGraphCase) {
    let fixture = prepare_fixture(case);
    let explicit_paths = explicit_update_paths(&fixture.live_files, &fixture.log);
    let registry = ParserRegistry::with_defaults();
    let supported_explicit = explicit_paths
        .iter()
        .filter(|path| registry.supports(path))
        .count();
    let unsupported_explicit = explicit_paths
        .iter()
        .filter(|path| !registry.supports(path))
        .count();

    let summary = update_graph(
        fixture.repo_root.as_ref(),
        &fixture.db_path,
        &UpdateOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: normalized_batch_size(case.batch_size),
            // Duplicate one supported path so a single update reparses the same
            // file twice and forces TreeCache old-tree handoff in phase 1.
            target: UpdateTarget::Files(explicit_paths.clone()),
            budget: BuildRunBudget::default(),
        },
    )
    .expect("explicit-file update must stay bounded for benign malformed input");

    if supported_explicit > 0 {
        assert!(
            summary.parsed > 0,
            "explicit-file update must parse at least one supported path"
        );
    }
    if unsupported_explicit > 0 {
        assert!(
            summary.skipped_unsupported > 0,
            "explicit-file update must report unsupported-path skips"
        );
    }

    let store = Store::open(&fixture.db_path).expect("open store after explicit update");
    for live in fixture.live_files.values() {
        if live.path_kind.is_supported() && explicit_paths.contains(&live.path) {
            assert!(
                store.file_hash(&live.path).expect("read explicit live file hash").is_some(),
                "supported explicit path must remain indexed after update"
            );
        }
    }
}

fn prepare_fixture(case: &UpdateGraphCase) -> Fixture {
    let temp_dir = tempfile::tempdir().expect("temp repo");
    let repo_root =
        Utf8PathBuf::from_path_buf(temp_dir.path().to_path_buf()).expect("utf8 repo path");

    git(repo_root.as_ref(), &["init", "--quiet"]);

    let mut live_files = build_initial_file_map(case);
    for initial in case.initial_files.iter().take(MAX_INITIAL_FILES) {
        let slot = normalize_slot(initial.slot);
        let path_kind = initial.path_kind;
        let path = path_kind.rel_path(slot);
        write_file(repo_root.as_ref(), &path, bounded_bytes(&initial.source));
        live_files.insert(slot, LiveFile { path, path_kind });
    }

    for live in live_files.values() {
        let abs_path = repo_root.join(&live.path);
        if !abs_path.exists() {
            write_file(repo_root.as_ref(), &live.path, default_source(live.path_kind));
        }
    }

    git(repo_root.as_ref(), &["add", "."]);
    git(repo_root.as_ref(), &["commit", "--quiet", "-m", "init"]);

    let db_path = repo_root.join("worldtree.db").to_string();
    build_graph(
        repo_root.as_ref(),
        &db_path,
        &BuildOptions {
            fail_fast: true,
            dry_run: false,
            batch_size: normalized_batch_size(case.batch_size),
            budget: BuildRunBudget::default(),
        },
    )
    .expect("initial build must succeed");

    let log = apply_mutations(repo_root.as_ref(), &mut live_files, case);

    Fixture {
        _temp_dir: temp_dir,
        repo_root,
        db_path,
        live_files,
        log,
    }
}

fn build_initial_file_map(case: &UpdateGraphCase) -> BTreeMap<u8, LiveFile> {
    let mut live_files = BTreeMap::new();
    for initial in case.initial_files.iter().take(MAX_INITIAL_FILES) {
        let slot = normalize_slot(initial.slot);
        live_files.insert(
            slot,
            LiveFile {
                path: initial.path_kind.rel_path(slot),
                path_kind: initial.path_kind,
            },
        );
    }

    if !live_files.values().any(|live| live.path_kind.is_supported()) {
        live_files.insert(
            SEED_SLOT,
            LiveFile {
                path: "src/fuzz_seed.rs".to_owned(),
                path_kind: UpdatePathKind::Supported(SupportedPathKind::Rust),
            },
        );
    }

    live_files
}

fn apply_mutations(
    repo_root: &Utf8Path,
    live_files: &mut BTreeMap<u8, LiveFile>,
    case: &UpdateGraphCase,
) -> MutationLog {
    let mut log = MutationLog::default();

    for mutation in case.mutations.iter().take(MAX_MUTATIONS) {
        match mutation {
            FileMutation::Add {
                slot,
                path_kind,
                source,
            } => {
                let slot = normalize_slot(*slot);
                if live_files.contains_key(&slot) {
                    if let Some(live) = live_files.get(&slot) {
                        write_file(repo_root, &live.path, bounded_bytes(source));
                        note_explicit_candidate(&mut log, live);
                    }
                    continue;
                }

                let live = LiveFile {
                    path: path_kind.rel_path(slot),
                    path_kind: *path_kind,
                };
                write_file(repo_root, &live.path, bounded_bytes(source));
                git_intent_to_add(repo_root, &live.path);
                note_explicit_candidate(&mut log, &live);
                live_files.insert(slot, live);
            }
            FileMutation::Modify { slot, source } => {
                let slot = normalize_slot(*slot);
                if let Some(live) = live_files.get(&slot) {
                    write_file(repo_root, &live.path, bounded_bytes(source));
                    note_explicit_candidate(&mut log, live);
                }
            }
            FileMutation::Delete { slot } => {
                let slot = normalize_slot(*slot);
                if let Some(live) = live_files.remove(&slot) {
                    remove_file(repo_root, &live.path);
                    log.deleted_paths.insert(live.path);
                }
            }
            FileMutation::Rename { slot, new_path_kind } => {
                let slot = normalize_slot(*slot);
                let Some(live) = live_files.get_mut(&slot) else {
                    continue;
                };

                let old_path = live.path.clone();
                let new_path = new_path_kind.rel_path(slot);
                if old_path == new_path {
                    continue;
                }

                rename_file(repo_root, &old_path, &new_path);
                git_intent_to_add(repo_root, &new_path);
                live.path = new_path.clone();
                live.path_kind = *new_path_kind;
                note_explicit_candidate(&mut log, live);
                log.renamed_paths.push(RenamedPath {
                    old_path,
                    new_path,
                    new_supported: new_path_kind.is_supported(),
                });
            }
        }
    }

    log
}

fn explicit_update_paths(live_files: &BTreeMap<u8, LiveFile>, log: &MutationLog) -> Vec<String> {
    let current_paths: BTreeSet<&str> = live_files.values().map(|live| live.path.as_str()).collect();
    let mut paths: Vec<String> = log
        .explicit_candidate_paths
        .iter()
        .filter(|path| current_paths.contains(path.as_str()))
        .cloned()
        .collect();

    if paths.is_empty()
        && let Some(seed) = live_files.values().find(|live| live.path_kind.is_supported())
    {
        paths.push(seed.path.clone());
    }

    if let Some(supported_path) = live_files
        .values()
        .find(|live| live.path_kind.is_supported())
        .map(|live| live.path.clone())
    {
        paths.push(supported_path.clone());
        paths.push(supported_path);
    }

    paths
}

fn note_explicit_candidate(log: &mut MutationLog, live: &LiveFile) {
    log.explicit_candidate_paths.insert(live.path.clone());
    if !live.path_kind.is_supported() {
        log.unsupported_explicit_paths.insert(live.path.clone());
    }
}

fn write_file(repo_root: &Utf8Path, rel_path: &str, source: Vec<u8>) {
    let abs_path = repo_root.join(rel_path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(abs_path, source).expect("write file");
}

fn remove_file(repo_root: &Utf8Path, rel_path: &str) {
    let abs_path = repo_root.join(rel_path);
    if abs_path.exists() {
        fs::remove_file(abs_path).expect("remove file");
    }
}

fn rename_file(repo_root: &Utf8Path, old_rel_path: &str, new_rel_path: &str) {
    let old_abs = repo_root.join(old_rel_path);
    let new_abs = repo_root.join(new_rel_path);
    if let Some(parent) = new_abs.parent() {
        fs::create_dir_all(parent).expect("create rename parent dir");
    }
    if new_abs.exists() {
        fs::remove_file(&new_abs).expect("remove conflicting rename target");
    }
    fs::rename(old_abs, new_abs).expect("rename file");
}

fn git(repo_root: &Utf8Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Atlas Fuzz")
        .env("GIT_AUTHOR_EMAIL", "fuzz@atlas")
        .env("GIT_COMMITTER_NAME", "Atlas Fuzz")
        .env("GIT_COMMITTER_EMAIL", "fuzz@atlas")
        .status()
        .expect("git command");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_intent_to_add(repo_root: &Utf8Path, rel_path: &str) {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(["add", "-N", "--"])
        .arg(rel_path)
        .env("GIT_AUTHOR_NAME", "Atlas Fuzz")
        .env("GIT_AUTHOR_EMAIL", "fuzz@atlas")
        .env("GIT_COMMITTER_NAME", "Atlas Fuzz")
        .env("GIT_COMMITTER_EMAIL", "fuzz@atlas")
        .status()
        .expect("git add -N");
    assert!(status.success(), "git add -N {rel_path} failed");
}

fn bounded_bytes(source: &[u8]) -> Vec<u8> {
    source[..source.len().min(MAX_SOURCE_BYTES)].to_vec()
}

fn default_source(path_kind: UpdatePathKind) -> Vec<u8> {
    match path_kind {
        UpdatePathKind::Supported(SupportedPathKind::Rust) => b"pub fn seed() {}\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::Go) => {
            b"package main\nfunc seed() {}\n".to_vec()
        }
        UpdatePathKind::Supported(SupportedPathKind::Python) => b"def seed():\n    pass\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::JavaScript) => {
            b"function seed() {}\n".to_vec()
        }
        UpdatePathKind::Supported(SupportedPathKind::TypeScript) => {
            b"function seed(): void {}\n".to_vec()
        }
        UpdatePathKind::Supported(SupportedPathKind::Json) => b"{}\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::Toml) => b"name = \"seed\"\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::Html) => b"<html></html>\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::Css) => b"body {}\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::Bash) => b"echo seed\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::Markdown) => b"# seed\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::Java) => {
            b"class Seed { void seed() {} }\n".to_vec()
        }
        UpdatePathKind::Supported(SupportedPathKind::CSharp) => {
            b"class Seed { void SeedFn() {} }\n".to_vec()
        }
        UpdatePathKind::Supported(SupportedPathKind::Php) => b"<?php function seed() {}\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::C) => b"int seed(void) { return 0; }\n".to_vec(),
        UpdatePathKind::Supported(SupportedPathKind::Cpp) => {
            b"int seed() { return 0; }\n".to_vec()
        }
        UpdatePathKind::Supported(SupportedPathKind::Scala) => {
            b"object Seed { def seed(): Unit = {} }\n".to_vec()
        }
        UpdatePathKind::Supported(SupportedPathKind::Ruby) => b"def seed; end\n".to_vec(),
        UpdatePathKind::UnsupportedText => b"seed text\n".to_vec(),
        UpdatePathKind::UnsupportedYaml => b"seed: true\n".to_vec(),
        UpdatePathKind::UnsupportedBinary => vec![0, 1, 2, 3],
    }
}

fn normalize_slot(slot: u8) -> u8 {
    slot % SLOT_COUNT
}

fn normalized_batch_size(batch_size: u8) -> usize {
    usize::from(batch_size % 4) + 1
}