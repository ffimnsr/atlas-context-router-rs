//! Advisory writer locks centralized in `.atlas/locks/`.

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::MarkdownError;

/// An exclusive writer lock held for the duration of a mutation.
///
/// The OS releases the lock when the holding process exits or dies, so a
/// crashed writer never leaves a stale lock behind.
#[derive(Debug)]
pub struct StoreLock {
    file: File,
    path: PathBuf,
}

impl StoreLock {
    /// Block until the named store lock is acquired.
    pub fn acquire(store_root: &Path, store_name: &str) -> Result<Self, MarkdownError> {
        let (file, path) = open_lock_file(store_root, store_name)?;
        file.lock_exclusive().map_err(|error| MarkdownError::Lock {
            path: path.clone(),
            message: error.to_string(),
        })?;
        Ok(Self { file, path })
    }

    /// Try to acquire without blocking. `Ok(None)` means another writer holds it.
    pub fn try_acquire(store_root: &Path, store_name: &str) -> Result<Option<Self>, MarkdownError> {
        let (file, path) = open_lock_file(store_root, store_name)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { file, path })),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(MarkdownError::Lock {
                path,
                message: error.to_string(),
            }),
        }
    }

    /// Lock file path backing this guard.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Canonical lock path for a named store.
pub fn lock_path(store_root: &Path, store_name: &str) -> PathBuf {
    store_root.join("locks").join(format!("{store_name}.lock"))
}

fn open_lock_file(store_root: &Path, store_name: &str) -> Result<(File, PathBuf), MarkdownError> {
    if store_name.is_empty()
        || !store_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(MarkdownError::Lock {
            path: store_root.to_path_buf(),
            message: format!("invalid lock name `{store_name}`"),
        });
    }
    let locks_dir = store_root.join("locks");
    fs::create_dir_all(&locks_dir).map_err(|error| MarkdownError::io(&locks_dir, error))?;
    let path = lock_path(store_root, store_name);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| MarkdownError::io(&path, error))?;
    Ok((file, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    fn wait_until_lock_available(root: &Path, name: &str) -> StoreLock {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(lock) = StoreLock::try_acquire(root, name).expect("try lock") {
                return lock;
            }
            assert!(
                Instant::now() < deadline,
                "lock was not released within the bounded wait"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn contention_serializes_writers() {
        let dir = TempDir::new().expect("temp");
        let root = dir.path().to_path_buf();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let holder_root = root.clone();
        let holder = std::thread::spawn(move || {
            let _lock = StoreLock::acquire(&holder_root, "memories").expect("acquire");
            ready_tx.send(()).expect("signal");
            let _ = release_rx.recv();
        });

        ready_rx.recv().expect("holder ready");
        assert!(
            StoreLock::try_acquire(&root, "memories")
                .expect("try")
                .is_none(),
            "second writer must not acquire while held"
        );
        let other_store = StoreLock::try_acquire(&root, "decisions").expect("try other");
        assert!(other_store.is_some(), "locks are per store name");
        drop(other_store);

        release_tx.send(()).expect("release");
        holder.join().expect("join");
        let _lock = wait_until_lock_available(&root, "memories");
    }

    #[test]
    fn lock_paths_live_under_locks_dir() {
        let dir = TempDir::new().expect("temp");
        let path = lock_path(dir.path(), "memories");
        assert_eq!(path, dir.path().join("locks").join("memories.lock"));
        assert!(!path.starts_with(dir.path().join("memories")));
    }

    #[test]
    fn traversal_names_rejected() {
        let dir = TempDir::new().expect("temp");
        assert!(StoreLock::acquire(dir.path(), "../evil").is_err());
        assert!(StoreLock::acquire(dir.path(), "").is_err());
    }
}
