use std::collections::HashMap;

use rusqlite::params;

use atlas_core::{AtlasError, Result};

use super::util::format_now;
use super::{ContentStore, DimensionMismatchError, EmbeddingProviderEntry};

/// Derive the provider name from the embedding base URL.
///
/// Mirrors the endpoint detection in `EmbeddingConfig::endpoint_url`:
///   URLs containing `/v1` → `"openai"`
///   All others            → `"ollama"`
pub fn provider_name_from_url(base_url: &str) -> &'static str {
    if base_url.contains("/v1") {
        "openai"
    } else {
        "ollama"
    }
}

impl ContentStore {
    // ------------------------------------------------------------------
    // In-process dimension cache
    // ------------------------------------------------------------------

    /// Look up a cached dimension for `(provider_name, model_name)`.
    ///
    /// Returns `None` when the pair has not been observed yet in this process.
    pub fn cached_dimension(&self, provider_name: &str, model_name: &str) -> Option<u32> {
        self.dim_cache
            .get(&(provider_name.to_owned(), model_name.to_owned()))
            .copied()
    }

    fn record_dimension_in_cache(&mut self, provider_name: &str, model_name: &str, dim: u32) {
        self.dim_cache
            .insert((provider_name.to_owned(), model_name.to_owned()), dim);
    }

    // ------------------------------------------------------------------
    // Registry persistence
    // ------------------------------------------------------------------

    /// Register an observed embedding dimension for `(provider_name, model_name)`.
    ///
    /// **Freeze semantics**: if the pair already has a persisted entry the
    /// dimension must match.  A mismatch returns `Err(AtlasError::Other(…))`
    /// wrapping a [`DimensionMismatchError`] message and the caller must abort
    /// the embedding run and require an explicit rebuild.
    ///
    /// On success the dimension is written into the in-process cache so
    /// subsequent calls within the same session skip the DB round-trip.
    pub fn register_embedding_dimension(
        &mut self,
        provider_name: &str,
        model_name: &str,
        dimension: u32,
    ) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        // Check persisted record first.
        let existing: Option<u32> = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT dimension FROM embedding_provider_registry
                     WHERE provider_name = ?1 AND model_name = ?2",
                )
                .map_err(db_err)?;
            let mut rows = stmt
                .query(params![provider_name, model_name])
                .map_err(db_err)?;
            rows.next()
                .map_err(db_err)?
                .map(|row| row.get::<_, u32>(0))
                .transpose()
                .map_err(db_err)?
        };

        if let Some(stored_dim) = existing {
            if stored_dim != dimension {
                return Err(AtlasError::Other(
                    DimensionMismatchError {
                        provider_name: provider_name.to_owned(),
                        model_name: model_name.to_owned(),
                        expected_dimension: stored_dim,
                        actual_dimension: dimension,
                    }
                    .to_string(),
                ));
            }
        } else {
            let now = format_now();
            self.conn
                .execute(
                    "INSERT INTO embedding_provider_registry
                         (provider_name, model_name, dimension, discovered_at, index_schema_version)
                     VALUES (?1, ?2, ?3, ?4, 1)
                     ON CONFLICT(provider_name, model_name) DO NOTHING",
                    params![provider_name, model_name, dimension, now],
                )
                .map_err(db_err)?;
        }

        self.record_dimension_in_cache(provider_name, model_name, dimension);
        Ok(())
    }

    /// Check that `dimension` matches the registered dimension for
    /// `(provider_name, model_name)`.
    ///
    /// Returns `Ok(())` when no registry entry exists yet (not yet frozen)
    /// or when the dimensions match.  Returns an error on mismatch.
    ///
    /// Uses the in-process cache first to avoid DB round-trips in tight loops.
    pub fn check_embedding_dimension(
        &mut self,
        provider_name: &str,
        model_name: &str,
        dimension: u32,
    ) -> Result<()> {
        // Fast path: in-process cache.
        if let Some(cached) = self.cached_dimension(provider_name, model_name) {
            if cached != dimension {
                return Err(AtlasError::Other(
                    DimensionMismatchError {
                        provider_name: provider_name.to_owned(),
                        model_name: model_name.to_owned(),
                        expected_dimension: cached,
                        actual_dimension: dimension,
                    }
                    .to_string(),
                ));
            }
            return Ok(());
        }
        // Slow path: consult DB (first call per session).
        self.register_embedding_dimension(provider_name, model_name, dimension)
    }

    /// Return the registry entry for `(provider_name, model_name)`, or `None`.
    pub fn get_embedding_provider(
        &self,
        provider_name: &str,
        model_name: &str,
    ) -> Result<Option<EmbeddingProviderEntry>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT provider_name, model_name, dimension, discovered_at, index_schema_version
                 FROM embedding_provider_registry
                 WHERE provider_name = ?1 AND model_name = ?2",
            )
            .map_err(db_err)?;
        let mut rows = stmt
            .query(params![provider_name, model_name])
            .map_err(db_err)?;
        if let Some(row) = rows.next().map_err(db_err)? {
            Ok(Some(Self::row_to_provider_entry(row)?))
        } else {
            Ok(None)
        }
    }

    /// Return all registry entries, ordered by `provider_name`, `model_name`.
    pub fn list_embedding_providers(&self) -> Result<Vec<EmbeddingProviderEntry>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT provider_name, model_name, dimension, discovered_at, index_schema_version
                 FROM embedding_provider_registry
                 ORDER BY provider_name, model_name",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| Ok(Self::row_to_provider_entry_fallible(row)))
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    fn row_to_provider_entry(
        row: &rusqlite::Row<'_>,
    ) -> atlas_core::Result<EmbeddingProviderEntry> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        Ok(EmbeddingProviderEntry {
            provider_name: row.get(0).map_err(db_err)?,
            model_name: row.get(1).map_err(db_err)?,
            dimension: row.get::<_, u32>(2).map_err(db_err)?,
            discovered_at: row.get(3).map_err(db_err)?,
            index_schema_version: row.get(4).map_err(db_err)?,
        })
    }

    fn row_to_provider_entry_fallible(
        row: &rusqlite::Row<'_>,
    ) -> atlas_core::Result<EmbeddingProviderEntry> {
        Self::row_to_provider_entry(row)
    }

    // ------------------------------------------------------------------
    // Cache warm-up helper
    // ------------------------------------------------------------------

    /// Pre-populate the in-process cache from all persisted registry entries.
    ///
    /// Call once after opening the store to ensure the first embed batch
    /// benefits from the fast cache path.
    pub fn warm_dimension_cache(&mut self) -> Result<()> {
        let entries = self.list_embedding_providers()?;
        for entry in entries {
            self.dim_cache
                .insert((entry.provider_name, entry.model_name), entry.dimension);
        }
        Ok(())
    }
}

// Re-export to allow building an initial cache without an open store.
pub type DimensionCache = HashMap<(String, String), u32>;
