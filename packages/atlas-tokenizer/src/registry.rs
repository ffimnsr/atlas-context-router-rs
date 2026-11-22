//! Encoding registry and cached loader.
//!
//! [`EncodingLoader`] is explicit and testable: callers provide a
//! [`CacheConfig`]. [`default_loader`] is a process-wide immutable loader that
//! resolves the cache directory from the upstream environment variables on
//! first use.

#[cfg(feature = "download")]
use std::collections::HashMap;
#[cfg(feature = "download")]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock};

#[cfg(feature = "download")]
use crate::assets::{asset_for, sha256_hex};
use crate::cache::CacheConfig;
#[cfg(feature = "download")]
use crate::encoding::EncodingSpec;
use crate::encoding::{Encoding, EncodingName};
use crate::error::{Error, Result};
use crate::model::encoding_name_for_model;

#[cfg(feature = "download")]
use crate::download::fetch_verified;

/// Source of vocabulary asset bytes used by [`EncodingLoader`].
///
/// The default implementation downloads over HTTPS and verifies sha256.
/// Custom implementations are useful for tests and air-gapped embedders; the
/// loader verifies sha256 again before parsing, so a custom fetcher cannot
/// introduce unverified vocabulary bytes.
#[cfg(feature = "download")]
pub trait AssetFetcher: Send + Sync + std::fmt::Debug {
    /// Fetch `url` and return bytes expected to match `sha256`.
    fn fetch(&self, url: &str, sha256: &str) -> Result<Vec<u8>>;
}

#[cfg(feature = "download")]
#[derive(Debug)]
struct DownloadFetcher;

#[cfg(feature = "download")]
impl AssetFetcher for DownloadFetcher {
    fn fetch(&self, url: &str, sha256: &str) -> Result<Vec<u8>> {
        fetch_verified(url, sha256)
    }
}

/// Explicit encoding loader with an in-memory cache and a shared disk cache.
pub struct EncodingLoader {
    config: CacheConfig,
    #[cfg(feature = "download")]
    loaded: Mutex<HashMap<(String, EncodingName), Arc<Encoding>>>,
    #[cfg(feature = "download")]
    fetcher: Arc<dyn AssetFetcher>,
}

impl std::fmt::Debug for EncodingLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncodingLoader")
            .field("cache_dir", &self.config.dir())
            .field("cache_explicit", &self.config.is_explicit())
            .finish_non_exhaustive()
    }
}

impl EncodingLoader {
    /// Build a loader with an explicit cache configuration.
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "download")]
            loaded: Mutex::new(HashMap::new()),
            #[cfg(feature = "download")]
            fetcher: Arc::new(DownloadFetcher),
        }
    }

    /// Build a loader with a custom asset fetcher.
    #[cfg(feature = "download")]
    #[doc(hidden)]
    pub fn with_fetcher(config: CacheConfig, fetcher: Arc<dyn AssetFetcher>) -> Self {
        Self {
            config,
            loaded: Mutex::new(HashMap::new()),
            fetcher,
        }
    }

    /// Load an encoding by name.
    #[cfg(feature = "download")]
    pub fn load(&self, name: EncodingName) -> Result<Arc<Encoding>> {
        let asset = asset_for(name);
        self.load_asset(asset.url, asset.sha256, name)
    }

    /// Load an encoding by name when asset download is disabled.
    #[cfg(not(feature = "download"))]
    pub fn load(&self, name: EncodingName) -> Result<Arc<Encoding>> {
        Err(Error::DownloadDisabled {
            name: name.as_str().to_owned(),
        })
    }

    /// Load an encoding from a specific asset URL and sha256 digest.
    #[cfg(feature = "download")]
    pub fn load_asset(
        &self,
        url: &str,
        expected_sha256: &str,
        name: EncodingName,
    ) -> Result<Arc<Encoding>> {
        if !name.is_enabled() {
            return Err(Error::EncodingUnavailable {
                name: name.as_str().to_owned(),
                feature: name.feature(),
            });
        }
        let key = (url.to_owned(), name);
        if let Some(encoding) = self
            .loaded
            .lock()
            .expect("encoding loader cache lock poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(encoding);
        }

        let bytes = match crate::cache::read_verified(&self.config, url, expected_sha256)? {
            Some(bytes) => bytes,
            None => {
                let fetched = self.fetcher.fetch(url, expected_sha256)?;
                let actual = sha256_hex(&fetched);
                if actual != expected_sha256 {
                    return Err(Error::HashMismatch {
                        expected: expected_sha256.to_owned(),
                        actual,
                    });
                }
                match crate::cache::write_atomic(&self.config, url, &fetched) {
                    Ok(()) => {}
                    Err(err) if self.config.is_explicit() => return Err(err),
                    Err(_) => {
                        // Upstream behavior: an unwritable default cache is
                        // ignored and the verified bytes are still used.
                    }
                }
                fetched
            }
        };

        let encoding = Arc::new(Encoding::from_rank_bytes(
            &bytes,
            EncodingSpec::for_name(name),
        )?);
        let mut loaded = self
            .loaded
            .lock()
            .expect("encoding loader cache lock poisoned");
        Ok(loaded.entry(key).or_insert(encoding).clone())
    }

    /// Load the encoding for a model name.
    pub fn load_for_model(&self, model: &str) -> Result<Arc<Encoding>> {
        self.load(encoding_name_for_model(model)?)
    }
}

/// Process-wide default loader using the upstream cache environment.
pub fn default_loader() -> &'static EncodingLoader {
    static LOADER: OnceLock<EncodingLoader> = OnceLock::new();
    LOADER.get_or_init(|| EncodingLoader::new(CacheConfig::from_env()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loader_is_stable() {
        let first = default_loader() as *const EncodingLoader;
        let second = default_loader() as *const EncodingLoader;
        assert_eq!(first, second);
    }

    #[cfg(not(feature = "download"))]
    #[test]
    fn named_encodings_require_download_feature() {
        let dir = tempfile::tempdir().expect("tempdir");
        let loader = EncodingLoader::new(CacheConfig::with_dir(dir.path()));
        let err = loader
            .load(EncodingName::Cl100kBase)
            .expect_err("download disabled");
        assert!(matches!(err, Error::DownloadDisabled { .. }));
    }
}
