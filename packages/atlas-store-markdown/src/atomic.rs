//! Atomic writes staged in `.atlas/tmp/` and rename publication.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::MarkdownError;

/// Age after which leftover temp fragments are considered stale.
pub const DEFAULT_TMP_MAX_AGE: Duration = Duration::from_secs(60 * 60);

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `store_root/relative` atomically.
///
/// The payload is written to `.atlas/tmp/`, fsynced, then renamed over the
/// target. Readers observe either the old file or the new file, never a torn
/// write. A crash leaves only a fragment in `.atlas/tmp/`.
pub fn write_atomic(store_root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), MarkdownError> {
    write_atomic_hooked(store_root, relative, bytes, |_| Ok(()))
}

/// Atomic write with a hook invoked after fsync and before rename.
///
/// The hook exists for crash-simulation tests; production callers use
/// [`write_atomic`].
pub fn write_atomic_hooked(
    store_root: &Path,
    relative: &Path,
    bytes: &[u8],
    before_rename: impl FnOnce(&Path) -> Result<(), MarkdownError>,
) -> Result<(), MarkdownError> {
    crate::store::validate_relative(relative)?;
    let target = store_root.join(relative);
    let parent = target.parent().ok_or_else(|| MarkdownError::UnsafePath {
        path: relative.to_path_buf(),
    })?;
    fs::create_dir_all(parent).map_err(|error| MarkdownError::io(parent, error))?;

    let tmp_dir = store_root.join("tmp");
    fs::create_dir_all(&tmp_dir).map_err(|error| MarkdownError::io(&tmp_dir, error))?;
    let temp_path = unique_temp_path(&tmp_dir);

    {
        let mut file =
            File::create(&temp_path).map_err(|error| MarkdownError::io(&temp_path, error))?;
        file.write_all(bytes)
            .map_err(|error| MarkdownError::io(&temp_path, error))?;
        file.sync_all()
            .map_err(|error| MarkdownError::io(&temp_path, error))?;
    }

    before_rename(&temp_path)?;

    fs::rename(&temp_path, &target).map_err(|error| {
        MarkdownError::io(
            &target,
            std::io::Error::new(
                error.kind(),
                format!("rename from {} failed: {error}", temp_path.display()),
            ),
        )
    })?;
    sync_dir(parent);
    Ok(())
}

/// Remove temp fragments older than `max_age`.
pub fn cleanup_tmp(store_root: &Path, max_age: Duration) -> Result<usize, MarkdownError> {
    let tmp_dir = store_root.join("tmp");
    if !tmp_dir.exists() {
        return Ok(0);
    }
    let mut removed = 0usize;
    let entries = fs::read_dir(&tmp_dir).map_err(|error| MarkdownError::io(&tmp_dir, error))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("tmp") {
            continue;
        }
        let is_stale = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .map(|age| age > max_age)
            .unwrap_or(false);
        if is_stale && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn unique_temp_path(tmp_dir: &Path) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    tmp_dir.join(format!("{}-{counter}-{nanos}.tmp", std::process::id()))
}

fn sync_dir(dir: &Path) {
    // Directory fsync is best-effort: unsupported on some platforms/file systems.
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_then_read_round_trips() {
        let dir = TempDir::new().expect("temp");
        write_atomic(dir.path(), Path::new("memories/topic/a.md"), b"one").expect("write");
        assert_eq!(
            fs::read(dir.path().join("memories/topic/a.md")).expect("read"),
            b"one"
        );
    }

    #[test]
    fn simulated_crash_leaves_target_intact_and_fragment_in_tmp() {
        let dir = TempDir::new().expect("temp");
        let relative = Path::new("memories/topic/a.md");
        write_atomic(dir.path(), relative, b"old").expect("seed");

        let result = write_atomic_hooked(dir.path(), relative, b"new", |_temp| {
            Err(MarkdownError::Lock {
                path: PathBuf::from("simulated"),
                message: "crash before rename".to_string(),
            })
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read(dir.path().join(relative)).expect("read"),
            b"old",
            "target must stay intact"
        );
        let fragments: Vec<_> = fs::read_dir(dir.path().join("tmp"))
            .expect("tmp")
            .filter_map(Result::ok)
            .collect();
        assert_eq!(fragments.len(), 1, "fragment must land in .atlas/tmp");
        assert!(
            !dir.path().join("memories/topic").join("a.md.tmp").exists(),
            "no temp may exist next to the committed record"
        );
    }

    #[test]
    fn stale_fragments_are_cleaned_and_fresh_ones_survive() {
        let dir = TempDir::new().expect("temp");
        let tmp_dir = dir.path().join("tmp");
        fs::create_dir_all(&tmp_dir).expect("mkdir");

        let stale = tmp_dir.join("1-0-1.tmp");
        fs::write(&stale, b"stale").expect("write");
        let old = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        let file = File::open(&stale).expect("open");
        file.set_modified(old).expect("set mtime");
        drop(file);

        let fresh = tmp_dir.join("2-0-2.tmp");
        fs::write(&fresh, b"fresh").expect("write");

        let removed = cleanup_tmp(dir.path(), DEFAULT_TMP_MAX_AGE).expect("cleanup");
        assert_eq!(removed, 1);
        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn absolute_or_parent_paths_are_rejected() {
        let dir = TempDir::new().expect("temp");
        assert!(write_atomic(dir.path(), Path::new("/etc/passwd"), b"x").is_err());
        assert!(write_atomic(dir.path(), Path::new("../escape.md"), b"x").is_err());
    }
}
