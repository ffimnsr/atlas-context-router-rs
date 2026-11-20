//! Store root, record helpers, and duplicate-id detection.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::atomic::{DEFAULT_TMP_MAX_AGE, cleanup_tmp, write_atomic};
use crate::error::MarkdownError;
use crate::frontmatter::Frontmatter;
use crate::lock::StoreLock;
use crate::record::{MarkdownRecord, read_frontmatter, read_record};

/// Markdown record store rooted at `.atlas/`.
#[derive(Debug, Clone)]
pub struct MarkdownStore {
    root: PathBuf,
}

impl MarkdownStore {
    /// Open a store root, creating `tmp/` and `locks/` and clearing stale
    /// temp fragments older than [`DEFAULT_TMP_MAX_AGE`].
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, MarkdownError> {
        let root = root.into();
        for dir in [root.clone(), root.join("tmp"), root.join("locks")] {
            fs::create_dir_all(&dir).map_err(|error| MarkdownError::io(&dir, error))?;
        }
        cleanup_tmp(&root, DEFAULT_TMP_MAX_AGE)?;
        Ok(Self { root })
    }

    /// Store root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Temp staging directory.
    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// Lock directory.
    pub fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    /// Convert a store-relative path to an absolute path.
    pub fn absolute(&self, relative: &Path) -> PathBuf {
        self.root.join(relative)
    }

    /// Write a record atomically.
    pub fn write_record(
        &self,
        relative: &Path,
        record: &MarkdownRecord,
    ) -> Result<(), MarkdownError> {
        let bytes = record.to_bytes(relative)?;
        write_atomic(&self.root, relative, &bytes)
    }

    /// Serialize and write raw record bytes atomically.
    pub fn write_bytes(&self, relative: &Path, bytes: &[u8]) -> Result<(), MarkdownError> {
        write_atomic(&self.root, relative, bytes)
    }

    /// Read a full record.
    pub fn read_record(&self, relative: &Path) -> Result<MarkdownRecord, MarkdownError> {
        read_record(&self.absolute(relative))
    }

    /// Read only frontmatter (L0).
    pub fn read_frontmatter(&self, relative: &Path) -> Result<Frontmatter, MarkdownError> {
        read_frontmatter(&self.absolute(relative))
    }

    /// Delete a record. Returns `true` when a file was removed.
    pub fn delete_record(&self, relative: &Path) -> Result<bool, MarkdownError> {
        validate_relative(relative)?;
        let target = self.absolute(relative);
        match fs::remove_file(&target) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(MarkdownError::io(target, error)),
        }
    }

    /// List `.md` files under a store-relative directory, sorted.
    pub fn list_markdown(&self, relative_dir: &Path) -> Result<Vec<PathBuf>, MarkdownError> {
        validate_relative(relative_dir)?;
        let dir = self.absolute(relative_dir);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        collect_markdown(&dir, &mut out)?;
        let mut relative = Vec::with_capacity(out.len());
        for path in out {
            if let Ok(stripped) = path.strip_prefix(&self.root) {
                relative.push(stripped.to_path_buf());
            }
        }
        relative.sort();
        Ok(relative)
    }

    /// Run a mutation under the named writer lock.
    pub fn with_writer_lock<T>(
        &self,
        store_name: &str,
        action: impl FnOnce() -> Result<T, MarkdownError>,
    ) -> Result<T, MarkdownError> {
        let _lock = StoreLock::acquire(&self.root, store_name)?;
        action()
    }

    /// Detect duplicate values of `id_field` across records under a directory.
    ///
    /// Returns `(id, paths)` only when an id appears more than once.
    pub fn detect_duplicate_ids(
        &self,
        relative_dir: &Path,
        id_field: &str,
    ) -> Result<Vec<(String, Vec<PathBuf>)>, MarkdownError> {
        let files = self.list_markdown(relative_dir)?;
        let mut seen: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for relative in files {
            let frontmatter = match self.read_frontmatter(&relative) {
                Ok(frontmatter) => frontmatter,
                Err(_) => continue,
            };
            let Some(id) = frontmatter.get_str(id_field) else {
                continue;
            };
            seen.entry(id.to_string()).or_default().push(relative);
        }
        Ok(seen
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect())
    }
}

/// Reject absolute paths, parent traversal, and non-normal components.
pub(crate) fn validate_relative(relative: &Path) -> Result<(), MarkdownError> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(MarkdownError::UnsafePath {
            path: relative.to_path_buf(),
        });
    }
    for component in relative.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(MarkdownError::UnsafePath {
                    path: relative.to_path_buf(),
                });
            }
        }
    }
    Ok(())
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), MarkdownError> {
    let entries = fs::read_dir(dir).map_err(|error| MarkdownError::io(dir, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| MarkdownError::io(dir, error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| MarkdownError::io(entry.path(), error))?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_markdown(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    use tempfile::TempDir;

    fn record(id: &str) -> MarkdownRecord {
        let mut frontmatter = Frontmatter::new();
        frontmatter.set("id", Value::str(id));
        frontmatter.set("title", Value::str(format!("title {id}")));
        MarkdownRecord::new(frontmatter, "body\n")
    }

    #[test]
    fn write_read_delete_and_list() {
        let dir = TempDir::new().expect("temp");
        let store = MarkdownStore::open(dir.path()).expect("open");
        let relative = Path::new("memories/topic/a.md");
        store.write_record(relative, &record("a")).expect("write");
        assert_eq!(
            store
                .read_record(relative)
                .expect("read")
                .frontmatter
                .get_str("id"),
            Some("a")
        );
        let listed = store.list_markdown(Path::new("memories")).expect("list");
        assert_eq!(listed, vec![relative.to_path_buf()]);
        assert!(store.delete_record(relative).expect("delete"));
        assert!(!store.delete_record(relative).expect("idempotent delete"));
    }

    #[test]
    fn writer_lock_is_bound_to_store_name() {
        let dir = TempDir::new().expect("temp");
        let store = MarkdownStore::open(dir.path()).expect("open");
        let value = store
            .with_writer_lock("memories", || Ok(7))
            .expect("locked action");
        assert_eq!(value, 7);
        assert!(dir.path().join("locks").join("memories.lock").exists());
    }

    #[test]
    fn duplicate_ids_are_reported_across_topic_directories() {
        let dir = TempDir::new().expect("temp");
        let store = MarkdownStore::open(dir.path()).expect("open");
        store
            .write_record(Path::new("memories/one/a.md"), &record("same"))
            .expect("write a");
        store
            .write_record(Path::new("memories/two/b.md"), &record("same"))
            .expect("write b");
        store
            .write_record(Path::new("memories/one/c.md"), &record("unique"))
            .expect("write c");

        let duplicates = store
            .detect_duplicate_ids(Path::new("memories"), "id")
            .expect("detect");
        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0].0, "same");
        assert_eq!(duplicates[0].1.len(), 2);
    }

    #[test]
    fn locks_and_temps_never_live_in_record_directories() {
        let dir = TempDir::new().expect("temp");
        let store = MarkdownStore::open(dir.path()).expect("open");
        for index in 0..5 {
            let relative = PathBuf::from(format!("memories/topic/{index}.md"));
            store
                .write_record(&relative, &record(&format!("{index}")))
                .expect("write");
        }
        let mut stack = vec![dir.path().join("memories")];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read_dir") {
                let entry = entry.expect("entry");
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                assert!(
                    !name.ends_with(".tmp"),
                    "temp file in committed dir: {name}"
                );
                assert!(
                    !name.ends_with(".lock"),
                    "lock file in committed dir: {name}"
                );
                if entry.file_type().expect("type").is_dir() {
                    stack.push(path);
                }
            }
        }
        assert!(dir.path().join("tmp").is_dir());
        assert!(dir.path().join("locks").is_dir());
    }

    #[test]
    fn unsafe_paths_rejected() {
        assert!(validate_relative(Path::new("../x.md")).is_err());
        assert!(validate_relative(Path::new("/x.md")).is_err());
        assert!(validate_relative(Path::new("x.md")).is_ok());
    }
}
