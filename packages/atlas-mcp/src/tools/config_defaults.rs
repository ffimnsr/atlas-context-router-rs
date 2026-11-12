//! Config-derived defaults shared by MCP query tools.
//!
//! Kept out of `shared.rs` (already beyond the 1,000-line threshold) and out
//! of `graph.rs` so call sites only consume the resolved value.

use atlas_engine::Config;

/// Whether `[search] hybrid_enabled = true` should activate hybrid retrieval
/// when a caller omits the explicit `hybrid` argument.
pub(crate) fn hybrid_enabled_default(repo_root: &str) -> bool {
    Config::load(&atlas_engine::paths::atlas_dir(repo_root))
        .map(|config| config.search.hybrid_enabled)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hybrid_default_reads_repo_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let atlas_dir = dir.path().join(".atlas");
        std::fs::create_dir_all(&atlas_dir).expect("atlas dir");
        std::fs::write(
            atlas_dir.join("config.toml"),
            "[search]\nhybrid_enabled = true\n",
        )
        .expect("write config");
        let repo_root = dir.path().to_string_lossy().to_string();

        assert!(hybrid_enabled_default(&repo_root));
    }

    #[test]
    fn hybrid_default_is_false_without_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path().to_string_lossy().to_string();

        assert!(!hybrid_enabled_default(&repo_root));
    }
}
