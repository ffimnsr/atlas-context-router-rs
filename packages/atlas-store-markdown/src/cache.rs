//! Bounded frontmatter cache keyed by path plus `(mtime, size)`.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::error::MarkdownError;
use crate::frontmatter::Frontmatter;
use crate::record::read_frontmatter;

/// Default cache capacity.
pub const DEFAULT_CACHE_CAPACITY: usize = 512;

/// In-process frontmatter cache for long-lived processes.
///
/// Every lookup stats the file first, so cross-process writes invalidate
/// naturally. The key includes both `mtime` and `size` to catch rewrites that
/// land inside the same timestamp tick. `Arc` keeps lookups cheap.
#[derive(Debug)]
pub struct FrontmatterCache {
    capacity: usize,
    inner: Mutex<CacheInner>,
}

#[derive(Debug, Default)]
struct CacheInner {
    entries: HashMap<PathBuf, Entry>,
    order: VecDeque<PathBuf>,
}

#[derive(Debug, Clone)]
struct Entry {
    modified: Option<SystemTime>,
    size: u64,
    frontmatter: Arc<Frontmatter>,
}

impl FrontmatterCache {
    /// New cache with an explicit LRU capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            inner: Mutex::new(CacheInner::default()),
        }
    }

    /// Cache with [`DEFAULT_CACHE_CAPACITY`].
    pub fn with_default_capacity() -> Self {
        Self::new(DEFAULT_CACHE_CAPACITY)
    }

    /// Look up frontmatter, re-reading when `(mtime, size)` changed.
    pub fn get(&self, path: &Path) -> Result<Arc<Frontmatter>, MarkdownError> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.invalidate(path);
                return Err(MarkdownError::io(path, error));
            }
        };
        let modified = metadata.modified().ok();
        let size = metadata.len();

        if let Some(hit) = self.lookup(path, modified, size) {
            return Ok(hit);
        }

        let frontmatter = Arc::new(read_frontmatter(path)?);
        self.insert(path, modified, size, Arc::clone(&frontmatter));
        Ok(frontmatter)
    }

    /// Drop a single entry.
    pub fn invalidate(&self, path: &Path) {
        let mut inner = self.inner.lock().expect("frontmatter cache poisoned");
        inner.entries.remove(path);
        inner.order.retain(|candidate| candidate != path);
    }

    /// Current entry count.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("frontmatter cache poisoned")
            .entries
            .len()
    }

    /// `true` when no entries are cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Configured capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn lookup(
        &self,
        path: &Path,
        modified: Option<SystemTime>,
        size: u64,
    ) -> Option<Arc<Frontmatter>> {
        let mut inner = self.inner.lock().expect("frontmatter cache poisoned");
        let entry = inner.entries.get(path)?;
        if entry.modified != modified || entry.size != size {
            inner.entries.remove(path);
            inner.order.retain(|candidate| candidate != path);
            return None;
        }
        let value = Arc::clone(&entry.frontmatter);
        inner.order.retain(|candidate| candidate != path);
        inner.order.push_back(path.to_path_buf());
        Some(value)
    }

    fn insert(
        &self,
        path: &Path,
        modified: Option<SystemTime>,
        size: u64,
        frontmatter: Arc<Frontmatter>,
    ) {
        let mut inner = self.inner.lock().expect("frontmatter cache poisoned");
        inner.entries.insert(
            path.to_path_buf(),
            Entry {
                modified,
                size,
                frontmatter,
            },
        );
        inner.order.retain(|candidate| candidate != path);
        inner.order.push_back(path.to_path_buf());
        while inner.entries.len() > self.capacity {
            let Some(evicted) = inner.order.pop_front() else {
                break;
            };
            inner.entries.remove(&evicted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        let mut file = fs::File::create(path).expect("create");
        file.write_all(content.as_bytes()).expect("write");
        file.sync_all().expect("sync");
    }

    fn record(title: &str) -> String {
        format!("---\ntitle: {title}\n---\nbody\n")
    }

    #[test]
    fn external_rewrite_invalidates_cache() {
        let dir = TempDir::new().expect("temp");
        let path = dir.path().join("a.md");
        write(&path, &record("first"));

        let cache = FrontmatterCache::new(8);
        let first = cache.get(&path).expect("first");
        assert_eq!(first.get_str("title"), Some("first"));

        write(&path, &record("second version"));
        let modified = SystemTime::now() + std::time::Duration::from_secs(2);
        let file = fs::File::open(&path).expect("open");
        file.set_modified(modified).expect("mtime");
        drop(file);

        let second = cache.get(&path).expect("second");
        assert_eq!(second.get_str("title"), Some("second version"));
    }

    #[test]
    fn shorter_replacement_with_same_mtime_is_detected_by_size() {
        let dir = TempDir::new().expect("temp");
        let path = dir.path().join("a.md");
        write(&path, &record("long-title"));

        let cache = FrontmatterCache::new(8);
        cache.get(&path).expect("prime");
        let metadata = fs::metadata(&path).expect("meta");
        let modified = metadata.modified().expect("mtime");

        write(&path, &record("x"));
        fs::File::open(&path)
            .expect("open")
            .set_modified(modified)
            .expect("restore mtime");

        let refreshed = cache.get(&path).expect("refresh");
        assert_eq!(refreshed.get_str("title"), Some("x"));
    }

    #[test]
    fn deleted_file_returns_not_found_without_panic() {
        let dir = TempDir::new().expect("temp");
        let path = dir.path().join("a.md");
        write(&path, &record("first"));

        let cache = FrontmatterCache::new(8);
        cache.get(&path).expect("prime");
        fs::remove_file(&path).expect("remove");

        let error = cache.get(&path).expect_err("missing");
        match error {
            MarkdownError::Io { source, .. } => {
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(cache.len(), 0, "missing entry must be dropped");
    }

    #[test]
    fn lru_capacity_is_enforced() {
        let dir = TempDir::new().expect("temp");
        let cache = FrontmatterCache::new(2);
        let paths: Vec<_> = (0..3)
            .map(|index| {
                let path = dir.path().join(format!("{index}.md"));
                write(&path, &record(&format!("t{index}")));
                path
            })
            .collect();

        for path in &paths[..2] {
            cache.get(path).expect("prime");
        }
        assert_eq!(cache.len(), 2);
        cache.get(&paths[2]).expect("third");
        assert_eq!(cache.len(), 2, "capacity must cap entry count");
        let first = cache.get(&paths[0]).expect("re-read evicted entry");
        assert_eq!(first.get_str("title"), Some("t0"));
    }
}
