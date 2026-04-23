/// Integration tests for atlas-repo that require a live git process.
///
/// These tests create temporary git repositories using `git init` / `git add` /
/// `git commit` to verify the full repo-scanning and change-detection paths.
use atlas_repo::{DiffTarget, changed_files, collect_files};
use camino::Utf8Path;
use std::process::Command;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Git env vars that may be inherited from the process that spawned the test
/// (e.g. a pre-commit hook running inside a real repository).  We strip them
/// from every git subprocess so that each test works against its own temp repo.
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

const GIT_TEST_NAME: &str = "Atlas Test";
const GIT_TEST_EMAIL: &str = "test@atlas";

/// Build a `Command` for git with inherited env vars stripped so tests are
/// isolated from the ambient git environment (e.g. a pre-commit hook context).
fn sanitized_git(dir: &std::path::Path) -> Command {
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

/// Initialise a bare git repo in `dir` with a minimal identity config so that
/// `git commit` works without the user's global config.
fn git_init(dir: &std::path::Path) {
    let run = |args: &[&str]| {
        let status = sanitized_git(dir).args(args).status().expect("git command");
        assert!(status.success(), "git {args:?} failed");
    };
    run(&["init", "--quiet"]);
    run(&["config", "user.email", GIT_TEST_EMAIL]);
    run(&["config", "user.name", GIT_TEST_NAME]);
}

fn git_add_all(dir: &std::path::Path) {
    let status = sanitized_git(dir)
        .args(["add", "-A"])
        .status()
        .expect("git add");
    assert!(status.success(), "git add -A failed");
}

fn git_commit(dir: &std::path::Path, msg: &str) {
    let status = sanitized_git(dir)
        .args(["commit", "--quiet", "-m", msg])
        .status()
        .expect("git commit");
    assert!(status.success(), "git commit failed");
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = sanitized_git(dir).args(args).status().expect("git command");
    assert!(status.success(), "git {args:?} failed");
}

fn init_committed_submodule(parent: &std::path::Path, path: &str) -> tempfile::TempDir {
    let submodule_dir = tempfile::tempdir().unwrap();
    let submodule_root = submodule_dir.path();
    git_init(submodule_root);
    std::fs::create_dir_all(submodule_root.join("src")).unwrap();
    std::fs::write(submodule_root.join("src/lib.rs"), "pub fn nested() {}\n").unwrap();
    git_add_all(submodule_root);
    git_commit(submodule_root, "initial submodule");

    git(
        parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            submodule_root.to_str().unwrap(),
            path,
        ],
    );
    git_commit(parent, "add submodule");

    submodule_dir
}

// ---------------------------------------------------------------------------
// 14.3 tracked-file collection
// ---------------------------------------------------------------------------

#[test]
fn collect_files_returns_git_tracked_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("lib.rs"), "pub fn foo() {}\n").unwrap();

    // Only stage the two Rust files; leave notes.txt untracked.
    Command::new("git")
        .args(["add", "main.rs", "lib.rs"])
        .current_dir(root)
        .status()
        .unwrap();
    std::fs::write(root.join("notes.txt"), "ignore me\n").unwrap();

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let files = collect_files(root_utf8, None).unwrap();

    let names: Vec<&str> = files.iter().map(|p| p.as_str()).collect();
    assert!(names.contains(&"main.rs"), "main.rs should be collected");
    assert!(names.contains(&"lib.rs"), "lib.rs should be collected");
}

#[test]
fn collect_files_excludes_binary_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    // Text file.
    std::fs::write(root.join("readme.txt"), "hello\n").unwrap();
    // Binary file (null byte in content).
    std::fs::write(root.join("data.bin"), b"\x00\x01\x02\x03").unwrap();

    git_add_all(root);

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let files = collect_files(root_utf8, None).unwrap();

    let names: Vec<&str> = files.iter().map(|p| p.as_str()).collect();
    assert!(
        names.contains(&"readme.txt"),
        "text file should be collected"
    );
    assert!(!names.contains(&"data.bin"), "binary file must be excluded");
}

#[test]
fn collect_files_excludes_files_over_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    let small = root.join("small.txt");
    std::fs::write(&small, b"x").unwrap();
    // 5-byte limit — write 10 bytes.
    let big = root.join("big.txt");
    std::fs::write(&big, b"0123456789").unwrap();

    git_add_all(root);

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    // 5-byte max — big.txt (10 bytes) must be excluded.
    let files = collect_files(root_utf8, Some(5)).unwrap();

    let names: Vec<&str> = files.iter().map(|p| p.as_str()).collect();
    assert!(
        names.contains(&"small.txt"),
        "small file should be collected"
    );
    assert!(
        !names.contains(&"big.txt"),
        "oversized file must be excluded"
    );
}

#[test]
fn collect_files_preserves_nested_posix_paths_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    let nested = root.join("src").join("mac os").join("helper.rs");
    std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
    std::fs::write(&nested, "pub fn helper() {}\n").unwrap();

    let script = root.join("scripts").join("build tool.py");
    std::fs::create_dir_all(script.parent().unwrap()).unwrap();
    std::fs::write(&script, "print('atlas')\n").unwrap();

    git_add_all(root);

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let files = collect_files(root_utf8, None).unwrap();
    let names: Vec<&str> = files.iter().map(|p| p.as_str()).collect();

    assert!(
        names.contains(&"src/mac os/helper.rs"),
        "nested Rust path should stay repo-relative with forward slashes; got {names:?}"
    );
    assert!(
        names.contains(&"scripts/build tool.py"),
        "paths with spaces should be preserved exactly; got {names:?}"
    );
}

#[test]
fn collect_files_recurses_into_initialized_submodules() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    let _submodule_dir = init_committed_submodule(root, "docs/wiki");

    std::fs::write(root.join("docs/wiki/notes.txt"), "draft\n").unwrap();

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let files = collect_files(root_utf8, None).unwrap();
    let names: Vec<&str> = files.iter().map(|p| p.as_str()).collect();

    assert!(
        names.contains(&"docs/wiki/src/lib.rs"),
        "tracked submodule file should be collected; got {names:?}"
    );
    assert!(
        names.contains(&"docs/wiki/notes.txt"),
        "untracked submodule file should be collected with submodule prefix; got {names:?}"
    );
}

#[test]
fn changed_files_working_tree_detects_unstaged_submodule_file_modification() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    let _submodule_dir = init_committed_submodule(root, "docs/wiki");

    std::fs::write(root.join("docs/wiki/src/lib.rs"), "pub fn nested_v2() {}\n").unwrap();

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::WorkingTree).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "docs/wiki/src/lib.rs" && c.change_type == ChangeType::Modified),
        "unstaged submodule file modification should surface child path; got {changes:?}"
    );
}

#[test]
fn changed_files_staged_detects_staged_submodule_file_modification() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    let _submodule_dir = init_committed_submodule(root, "docs/wiki");

    std::fs::write(root.join("docs/wiki/src/lib.rs"), "pub fn nested_v2() {}\n").unwrap();
    git(root.join("docs/wiki").as_path(), &["add", "src/lib.rs"]);

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "docs/wiki/src/lib.rs" && c.change_type == ChangeType::Modified),
        "staged submodule file modification should surface child path; got {changes:?}"
    );
}

#[test]
fn changed_files_staged_expands_added_submodule_into_child_files() {
    let submodule_dir = tempfile::tempdir().unwrap();
    let submodule_root = submodule_dir.path();
    git_init(submodule_root);
    std::fs::create_dir_all(submodule_root.join("src")).unwrap();
    std::fs::write(submodule_root.join("src/lib.rs"), "pub fn nested() {}\n").unwrap();
    git_add_all(submodule_root);
    git_commit(submodule_root, "initial submodule");

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    git(
        root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            submodule_root.to_str().unwrap(),
            "docs/wiki",
        ],
    );

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "docs/wiki/src/lib.rs" && c.change_type == ChangeType::Added),
        "staged submodule add should expand into child adds; got {changes:?}"
    );
}

#[test]
fn changed_files_base_ref_expands_submodule_gitlink_delta_into_child_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    let _submodule_dir = init_committed_submodule(root, "docs/wiki");

    std::fs::write(root.join("docs/wiki/src/lib.rs"), "pub fn nested_v2() {}\n").unwrap();
    git(root.join("docs/wiki").as_path(), &["add", "src/lib.rs"]);
    git_commit(root.join("docs/wiki").as_path(), "submodule update");
    git(root, &["add", "docs/wiki"]);
    git_commit(root, "bump submodule");

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::BaseRef("HEAD~1".to_string())).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "docs/wiki/src/lib.rs" && c.change_type == ChangeType::Modified),
        "base-ref diff should expand gitlink delta into child file changes; got {changes:?}"
    );
}

#[test]
fn changed_files_base_ref_expands_dirty_submodule_worktree_without_gitlink_bump() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    let _submodule_dir = init_committed_submodule(root, "docs/wiki");

    std::fs::write(root.join("docs/wiki/src/lib.rs"), "pub fn nested_v2() {}\n").unwrap();

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::BaseRef("HEAD".to_string())).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "docs/wiki/src/lib.rs" && c.change_type == ChangeType::Modified),
        "base-ref diff should expand dirty submodule worktree into child changes; got {changes:?}"
    );
}

#[test]
fn changed_files_staged_expands_deleted_submodule_into_child_deletes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    let _submodule_dir = init_committed_submodule(root, "docs/wiki");

    git(root, &["rm", "-f", "docs/wiki"]);

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "docs/wiki/src/lib.rs" && c.change_type == ChangeType::Deleted),
        "staged submodule delete should expand into child deletes; got {changes:?}"
    );
}

#[test]
fn changed_files_staged_expands_renamed_submodule_into_child_renames() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    let _submodule_dir = init_committed_submodule(root, "docs/wiki");

    git(root, &["mv", "docs/wiki", "docs/wiki-renamed"]);

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes.iter().any(|c| {
            c.change_type == ChangeType::Renamed
                && c.path == "docs/wiki-renamed/src/lib.rs"
                && c.old_path.as_deref() == Some("docs/wiki/src/lib.rs")
        }),
        "staged submodule rename should expand into child renames; got {changes:?}"
    );
}

// ---------------------------------------------------------------------------
// 14.3 change detection
// ---------------------------------------------------------------------------

#[test]
fn changed_files_detects_staged_added_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    // Initial commit.
    std::fs::write(root.join("lib.rs"), "pub fn old() {}\n").unwrap();
    git_add_all(root);
    git_commit(root, "initial");

    // Stage a new file.
    std::fs::write(root.join("new.rs"), "pub fn new() {}\n").unwrap();
    git_add_all(root);

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "new.rs" && c.change_type == ChangeType::Added),
        "staged new file must appear as Added; got {changes:?}"
    );
}

#[test]
fn changed_files_detects_staged_modified_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    std::fs::write(root.join("lib.rs"), "pub fn v1() {}\n").unwrap();
    git_add_all(root);
    git_commit(root, "initial");

    // Modify and stage.
    std::fs::write(root.join("lib.rs"), "pub fn v2() {}\n").unwrap();
    git_add_all(root);

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "lib.rs" && c.change_type == ChangeType::Modified),
        "staged modified file must appear as Modified; got {changes:?}"
    );
}

#[test]
fn changed_files_detects_staged_deleted_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    std::fs::write(root.join("to_delete.rs"), "// bye\n").unwrap();
    git_add_all(root);
    git_commit(root, "initial");

    // Delete and stage.
    std::fs::remove_file(root.join("to_delete.rs")).unwrap();
    Command::new("git")
        .args(["rm", "--cached", "to_delete.rs"])
        .current_dir(root)
        .status()
        .unwrap();

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "to_delete.rs" && c.change_type == ChangeType::Deleted),
        "staged deleted file must appear as Deleted; got {changes:?}"
    );
}

#[test]
fn changed_files_detects_staged_renamed_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    std::fs::write(root.join("old.rs"), "pub fn x() {}\n").unwrap();
    git_add_all(root);
    git_commit(root, "initial");

    // Rename via git mv.
    Command::new("git")
        .args(["mv", "old.rs", "renamed.rs"])
        .current_dir(root)
        .status()
        .unwrap();

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes.iter().any(|c| c.change_type == ChangeType::Renamed
            && c.path == "renamed.rs"
            && c.old_path.as_deref() == Some("old.rs")),
        "staged rename must appear as Renamed with old_path; got {changes:?}"
    );
}

#[test]
fn changed_files_working_tree_detects_unstaged_modification() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    std::fs::write(root.join("lib.rs"), "pub fn v1() {}\n").unwrap();
    git_add_all(root);
    git_commit(root, "initial");

    // Modify without staging.
    std::fs::write(root.join("lib.rs"), "pub fn v2() {}\n").unwrap();

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::WorkingTree).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "lib.rs" && c.change_type == ChangeType::Modified),
        "unstaged modification must appear as Modified in working-tree diff; got {changes:?}"
    );
}

#[test]
fn changed_files_base_ref_handles_nested_paths_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    let nested = root.join("src").join("mac os").join("helper.rs");
    std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
    std::fs::write(&nested, "pub fn helper_v1() {}\n").unwrap();
    git_add_all(root);
    git_commit(root, "initial");

    std::fs::write(&nested, "pub fn helper_v2() {}\n").unwrap();

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::BaseRef("HEAD".to_string())).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes
            .iter()
            .any(|c| c.path == "src/mac os/helper.rs" && c.change_type == ChangeType::Modified),
        "base-ref diff should preserve nested forward-slash paths with spaces; got {changes:?}"
    );
}

#[test]
fn changed_files_detects_staged_renamed_nested_file_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);

    let original_dir = root.join("src").join("old folder");
    std::fs::create_dir_all(&original_dir).unwrap();
    let old_path = original_dir.join("old name.rs");
    std::fs::write(&old_path, "pub fn before() {}\n").unwrap();
    git_add_all(root);
    git_commit(root, "initial");

    let new_dir = root.join("src").join("new folder");
    std::fs::create_dir_all(&new_dir).unwrap();
    let status = Command::new("git")
        .args([
            "mv",
            "src/old folder/old name.rs",
            "src/new folder/new name.rs",
        ])
        .current_dir(root)
        .status()
        .expect("git mv");
    assert!(status.success(), "git mv should succeed");

    let root_utf8 = Utf8Path::from_path(root).unwrap();
    let changes = changed_files(root_utf8, &DiffTarget::Staged).unwrap();

    use atlas_core::model::ChangeType;
    assert!(
        changes.iter().any(|c| c.change_type == ChangeType::Renamed
            && c.path == "src/new folder/new name.rs"
            && c.old_path.as_deref() == Some("src/old folder/old name.rs")),
        "staged rename should preserve old/new nested paths with spaces; got {changes:?}"
    );
}
