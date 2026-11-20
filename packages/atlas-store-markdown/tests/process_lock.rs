//! Process-level lock semantics: a killed writer must release the OS lock.

use std::time::{Duration, Instant};

use atlas_store_markdown::lock::StoreLock;

const READY: &str = "child-ready";

/// Helper test executed in a child process. Returns immediately unless the
/// parent test set `ATLAS_CHILD_LOCK_DIR`.
#[test]
fn child_lock_holder() {
    let Ok(dir) = std::env::var("ATLAS_CHILD_LOCK_DIR") else {
        return;
    };
    let root = std::path::PathBuf::from(dir);
    let _lock = StoreLock::acquire(&root, "memories").expect("child acquires lock");
    std::fs::write(root.join(READY), b"1").expect("signal ready");
    let release = root.join("child-release");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !release.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for(path: &std::path::Path, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn killed_writer_releases_lock() {
    let dir = tempfile::TempDir::new().expect("temp");
    let exe = std::env::current_exe().expect("current exe");

    let mut child = std::process::Command::new(exe)
        .args(["--exact", "child_lock_holder", "--nocapture"])
        .env("ATLAS_CHILD_LOCK_DIR", dir.path())
        .spawn()
        .expect("spawn child");

    wait_for(&dir.path().join(READY), "child ready");

    assert!(
        StoreLock::try_acquire(dir.path(), "memories")
            .expect("try lock")
            .is_none(),
        "child must hold the lock"
    );

    child.kill().expect("kill child");
    child.wait().expect("reap child");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if StoreLock::try_acquire(dir.path(), "memories")
            .expect("try lock")
            .is_some()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "killed writer must release the OS lock"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
