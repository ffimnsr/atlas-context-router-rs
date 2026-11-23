use super::*;

fn head_sha(repo_root: &Path) -> String {
    String::from_utf8(run_command(repo_root, "git", &["rev-parse", "HEAD"]).stdout)
        .expect("git rev-parse output")
        .trim()
        .to_owned()
}

fn commit_repo_change(repo_root: &Path, message: &str) -> String {
    run_command(repo_root, "git", &["add", "."]);
    run_command(repo_root, "git", &["commit", "--quiet", "-m", message]);
    head_sha(repo_root)
}

fn shallow_clone_repo(source: &Path) -> tempfile::TempDir {
    let clone = tempfile::tempdir().expect("clone tempdir");
    let source_url = format!("file://{}", source.display());
    run_command(
        Path::new("/"),
        "git",
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            source_url.as_str(),
            clone.path().to_str().expect("clone path"),
        ],
    );
    clone
}

fn commit_with_time(repo_root: &Path, message: &str, epoch_secs: i64) -> String {
    let output = sanitized_command("git")
        .args(["add", "."])
        .current_dir(repo_root)
        .output()
        .expect("git add for timed commit");
    assert!(output.status.success(), "git add failed: {:?}", output);

    let timestamp = epoch_secs.to_string();
    let output = sanitized_command("git")
        .args(["commit", "--quiet", "-m", message])
        .current_dir(repo_root)
        .env("GIT_AUTHOR_DATE", &timestamp)
        .env("GIT_COMMITTER_DATE", &timestamp)
        .output()
        .expect("git commit for timed commit");
    assert!(
        output.status.success(),
        "git timed commit failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    head_sha(repo_root)
}

mod build;
mod diff;
mod graph;
mod prune;
mod symbols;
