//! Upstream-compatible asset cache with atomic writes.
//!
//! The cache key is `sha1(url)` in hex, matching `tiktoken/load.py`. Cache
//! entries are re-verified with sha256 on every read. A mismatched entry is
//! deleted and treated as a miss so the caller re-downloads it. Writes go to
//! a unique temporary file in the same directory and are moved into place with
//! an atomic rename.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha1::{Digest, Sha1};

use crate::assets::sha256_hex;
use crate::error::{Error, Result};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Cache location and whether it was explicitly configured.
///
/// Environment precedence matches upstream `tiktoken`: `TIKTOKEN_CACHE_DIR`,
/// then `DATA_GYM_CACHE_DIR`, then `<temp_dir>/data-gym-cache`. Environment
/// directories count as explicit, so write failures are reported. The
/// implicit temp directory silently falls back to in-memory use on write
/// failure, matching upstream behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    dir: PathBuf,
    explicit: bool,
}

impl CacheConfig {
    /// Resolve the cache directory from the environment.
    pub fn from_env() -> Self {
        resolve_from_env(|name| std::env::var_os(name))
    }

    /// Use an explicit cache directory. Write failures are reported.
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            explicit: true,
        }
    }

    /// Directory used for cache entries.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether the directory came from an explicit user configuration.
    pub fn is_explicit(&self) -> bool {
        self.explicit
    }

    /// Cache path for an asset URL.
    pub fn path_for(&self, url: &str) -> PathBuf {
        self.dir.join(cache_key(url))
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

fn resolve_from_env(mut get: impl FnMut(&str) -> Option<OsString>) -> CacheConfig {
    for name in ["TIKTOKEN_CACHE_DIR", "DATA_GYM_CACHE_DIR"] {
        if let Some(value) = get(name).filter(|value| !value.is_empty()) {
            return CacheConfig {
                dir: PathBuf::from(value),
                explicit: true,
            };
        }
    }
    CacheConfig {
        dir: std::env::temp_dir().join("data-gym-cache"),
        explicit: false,
    }
}

/// Upstream-compatible `sha1(url)` cache key in lowercase hex.
pub fn cache_key(url: &str) -> String {
    format!("{:x}", Sha1::digest(url.as_bytes()))
}

/// Read a cache entry and verify its sha256 digest.
///
/// Returns `Ok(None)` for a missing entry. A mismatched entry is deleted and
/// also returned as a miss. Unreadable entries in the implicit temp cache are
/// treated as misses; explicit cache directories surface the I/O error.
pub fn read_verified(
    config: &CacheConfig,
    url: &str,
    expected_sha256: &str,
) -> Result<Option<Vec<u8>>> {
    let path = config.path_for(url);
    match fs::read(&path) {
        Ok(bytes) => {
            if sha256_hex(&bytes) == expected_sha256 {
                Ok(Some(bytes))
            } else {
                let _ = fs::remove_file(&path);
                Ok(None)
            }
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) if !config.is_explicit() => Ok(None),
        Err(source) => Err(Error::CacheIo { path, source }),
    }
}

/// Write a cache entry atomically: unique temp file plus rename.
pub fn write_atomic(config: &CacheConfig, url: &str, bytes: &[u8]) -> Result<()> {
    let path = config.path_for(url);
    let parent = config.dir();
    fs::create_dir_all(parent).map_err(|source| Error::CacheIo {
        path: parent.to_path_buf(),
        source,
    })?;

    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        "{}.{}.{}.tmp",
        cache_key(url),
        std::process::id(),
        counter
    ));
    let result = fs::write(&temp, bytes).and_then(|()| fs::rename(&temp, &path));
    if let Err(source) = result {
        let _ = fs::remove_file(&temp);
        // Windows cannot replace an existing destination with rename. Remove
        // and retry once; the destination is a cache entry, never user data.
        if path.exists() {
            let retry = fs::remove_file(&path)
                .and_then(|()| fs::write(&temp, bytes))
                .and_then(|()| fs::rename(&temp, &path));
            if retry.is_ok() {
                return Ok(());
            }
        }
        return Err(Error::CacheIo { path, source });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env(vars: &[(&str, &str)]) -> CacheConfig {
        resolve_from_env(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn env_precedence_prefers_tiktoken_then_data_gym() {
        let config = with_env(&[
            ("TIKTOKEN_CACHE_DIR", "/tmp/tiktoken"),
            ("DATA_GYM_CACHE_DIR", "/tmp/data-gym"),
        ]);
        assert_eq!(config.dir(), Path::new("/tmp/tiktoken"));
        assert!(config.is_explicit());

        let config = with_env(&[("DATA_GYM_CACHE_DIR", "/tmp/data-gym")]);
        assert_eq!(config.dir(), Path::new("/tmp/data-gym"));
        assert!(config.is_explicit());
    }

    #[test]
    fn empty_env_values_are_ignored() {
        let config = with_env(&[("TIKTOKEN_CACHE_DIR", ""), ("DATA_GYM_CACHE_DIR", "")]);
        assert_eq!(
            config.dir(),
            std::env::temp_dir().join("data-gym-cache").as_path()
        );
        assert!(!config.is_explicit());
    }

    #[test]
    fn cache_key_matches_upstream_sha1() {
        assert_eq!(
            cache_key("https://openaipublic.blob.core.windows.net/encodings/r50k_base.tiktoken"),
            "0ea1e91bbb3a60f729a8dc8f777fd2fc07cd8df4"
        );
    }

    #[test]
    fn missing_entry_is_a_miss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = CacheConfig::with_dir(dir.path());
        assert!(
            read_verified(&config, "https://example.test/a", "deadbeef")
                .expect("read")
                .is_none()
        );
    }

    #[test]
    fn corrupt_entry_is_deleted_and_reported_as_miss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = CacheConfig::with_dir(dir.path());
        let url = "https://example.test/a";
        write_atomic(&config, url, b"hello").expect("write");
        let path = config.path_for(url);
        assert!(path.exists());

        let miss = read_verified(&config, url, "0000").expect("read");
        assert!(miss.is_none());
        assert!(!path.exists(), "corrupt entry must be removed");
    }

    #[test]
    fn atomic_write_is_visible_after_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = CacheConfig::with_dir(dir.path());
        let url = "https://example.test/atomic";
        let bytes = b"atomic payload";
        write_atomic(&config, url, bytes).expect("write");
        let expected = sha256_hex(bytes);
        let read = read_verified(&config, url, &expected)
            .expect("read")
            .expect("hit");
        assert_eq!(read, bytes);
    }
}
