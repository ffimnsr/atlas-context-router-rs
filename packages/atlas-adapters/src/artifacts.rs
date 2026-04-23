//! Shared artifact-routing helpers for CLI and MCP surfaces.

use std::path::{Path, PathBuf};

use camino::Utf8Path;

use atlas_core::{AtlasError, Result};
use atlas_repo::CanonicalRepoPath;

use crate::bridge::BRIDGE_DIR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactIdentityKind {
    RepoPath,
    SyntheticPath,
    ArtifactLabel,
    External,
}

impl ArtifactIdentityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RepoPath => "repo_path",
            Self::SyntheticPath => "synthetic_path",
            Self::ArtifactLabel => "artifact_label",
            Self::External => "external",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactIdentity {
    kind: ArtifactIdentityKind,
    value: String,
}

impl ArtifactIdentity {
    pub fn repo_path(repo_root: &str, path: &str) -> Result<Self> {
        let canonical =
            CanonicalRepoPath::from_cli_argument(Utf8Path::new(repo_root), Utf8Path::new(path))
                .map_err(|err| AtlasError::Other(err.to_string()))?;
        Ok(Self::from_canonical_repo_path(canonical))
    }

    pub fn from_canonical_repo_path(path: CanonicalRepoPath) -> Self {
        Self {
            kind: ArtifactIdentityKind::RepoPath,
            value: path.as_str().to_owned(),
        }
    }

    pub fn synthetic_path(path: impl Into<String>) -> Self {
        Self {
            kind: ArtifactIdentityKind::SyntheticPath,
            value: path.into(),
        }
    }

    pub fn artifact_label(label: impl Into<String>) -> Self {
        Self {
            kind: ArtifactIdentityKind::ArtifactLabel,
            value: label.into(),
        }
    }

    pub fn external(label: impl Into<String>) -> Self {
        Self {
            kind: ArtifactIdentityKind::External,
            value: label.into(),
        }
    }

    pub fn kind(&self) -> ArtifactIdentityKind {
        self.kind
    }

    pub fn kind_str(&self) -> &'static str {
        self.kind.as_str()
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

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

/// Generate stable source id from structured identity and content sample.
///
/// File-backed identities must already carry canonical repo-path seed data.
/// For `repo_path`, callers should derive `identity.value()` from
/// `CanonicalRepoPath` before hashing so cross-store joins stay stable.
///
/// Uses SHA-256 over `identity_kind\0identity_value\0content` with at most
/// first 64 KB of content.
/// Encoded as first 16 digest bytes in lowercase hex.
pub fn generate_source_id(identity: &ArtifactIdentity, content: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(identity.kind_str().as_bytes());
    hasher.update(b"\x00");
    hasher.update(identity.value().as_bytes());
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
        ArtifactIdentity, ArtifactIdentityKind, derive_bridge_dir, derive_content_db_path,
        derive_session_db_path, generate_source_id,
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
        let identity = ArtifactIdentity::artifact_label("my label");
        let first = generate_source_id(&identity, "some content");
        let second = generate_source_id(&identity, "some content");
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);
    }

    #[test]
    fn generate_source_id_changes_with_input() {
        let first = generate_source_id(&ArtifactIdentity::artifact_label("label a"), "content a");
        let second = generate_source_id(&ArtifactIdentity::artifact_label("label b"), "content b");
        assert_ne!(first, second);
    }

    #[test]
    fn repo_path_identity_canonicalizes_abs_and_rel_paths() {
        let rel = ArtifactIdentity::repo_path("/repo", "src/lib.rs").unwrap();
        let abs = ArtifactIdentity::repo_path("/repo", "/repo/src/lib.rs").unwrap();
        assert_eq!(rel, abs);
        assert_eq!(rel.kind(), ArtifactIdentityKind::RepoPath);
        assert_eq!(rel.value(), "src/lib.rs");
    }

    #[test]
    fn repo_path_identity_canonicalizes_dot_segments_and_backslashes() {
        let dotted = ArtifactIdentity::repo_path("/repo", "./src/../src\\lib.rs").unwrap();
        let canonical = ArtifactIdentity::repo_path("/repo", "src/lib.rs").unwrap();
        assert_eq!(dotted, canonical);
        assert_eq!(canonical.value(), "src/lib.rs");
    }
}
