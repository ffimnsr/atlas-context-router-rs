//! Shared artifact-routing helpers for CLI and MCP surfaces.

use std::path::{Path, PathBuf};

use crate::bridge::BRIDGE_DIR;

/// Derive session DB path from graph DB path.
pub fn derive_session_db_path(db_path: &str) -> String {
    if let Some(parent) = Path::new(db_path).parent() {
        parent.join("session.db").to_string_lossy().into_owned()
    } else {
        "session.db".to_string()
    }
}

/// Derive content DB path from graph DB path.
pub fn derive_content_db_path(db_path: &str) -> String {
    if let Some(parent) = Path::new(db_path).parent() {
        parent.join("context.db").to_string_lossy().into_owned()
    } else {
        "context.db".to_string()
    }
}

/// Derive bridge artifact directory from graph DB path.
pub fn derive_bridge_dir(db_path: &str) -> PathBuf {
    if let Some(parent) = Path::new(db_path).parent() {
        parent.join(BRIDGE_DIR)
    } else {
        PathBuf::from(BRIDGE_DIR)
    }
}

/// Generate stable source id from label and content sample.
///
/// Uses SHA-256 over `label\0content` with at most first 64 KB of content.
/// Encoded as first 16 digest bytes in lowercase hex.
pub fn generate_source_id(label: &str, content: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update(b"\x00");
    let sample = &content.as_bytes()[..content.len().min(65_536)];
    hasher.update(sample);
    let digest = hasher.finalize();
    digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        derive_bridge_dir, derive_content_db_path, derive_session_db_path, generate_source_id,
    };

    #[test]
    fn derive_session_db_path_reuses_atlas_dir() {
        let path = derive_session_db_path("/repo/.atlas/worldtree.db");
        assert!(path.ends_with("session.db"), "got: {path}");
        assert!(path.contains(".atlas"), "got: {path}");
    }

    #[test]
    fn derive_content_db_path_reuses_atlas_dir() {
        let path = derive_content_db_path("/repo/.atlas/worldtree.db");
        assert!(path.ends_with("context.db"), "got: {path}");
    }

    #[test]
    fn derive_bridge_dir_reuses_atlas_dir() {
        let dir = derive_bridge_dir("/repo/.atlas/worldtree.db");
        assert_eq!(dir, std::path::PathBuf::from("/repo/.atlas/bridge"));
    }

    #[test]
    fn generate_source_id_is_deterministic() {
        let first = generate_source_id("my label", "some content");
        let second = generate_source_id("my label", "some content");
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);
    }

    #[test]
    fn generate_source_id_changes_with_input() {
        let first = generate_source_id("label a", "content a");
        let second = generate_source_id("label b", "content b");
        assert_ne!(first, second);
    }
}
