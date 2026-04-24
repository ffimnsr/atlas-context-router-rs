use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) const GIT_TEST_NAME: &str = "Atlas Test";
pub(crate) const GIT_TEST_EMAIL: &str = "test@atlas";

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

pub(crate) fn sanitized_git(dir: &Path) -> Command {
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

pub(crate) fn git(dir: &Path, args: &[&str]) {
    let status = sanitized_git(dir).args(args).status().expect("git command");
    assert!(status.success(), "git {args:?} failed");
}

pub(crate) fn git_output(dir: &Path, args: &[&str]) -> String {
    let output = sanitized_git(dir).args(args).output().expect("git output");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).expect("utf8")
}

pub(crate) fn git_init(dir: &Path) {
    git(dir, &["init", "--quiet"]);
    git(dir, &["config", "user.email", GIT_TEST_EMAIL]);
    git(dir, &["config", "user.name", GIT_TEST_NAME]);
    git(dir, &["branch", "-M", "main"]);
}

pub(crate) fn git_clone_shallow(source: &Path, dest: &Path) {
    let source_url = format!("file://{}", source.display());
    let output = Command::new("git")
        .args(["clone", "--quiet", "--depth", "1", &source_url])
        .arg(dest)
        .output()
        .expect("git clone --depth 1");
    assert!(
        output.status.success(),
        "git clone failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn write_file(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdirs");
    }
    fs::write(path, content).expect("write file");
}

pub(crate) fn commit_all(root: &Path, message: &str) -> String {
    git(root, &["add", "-A"]);
    git(root, &["commit", "--quiet", "-m", message]);
    git_output(root, &["rev-parse", "HEAD"]).trim().to_owned()
}
