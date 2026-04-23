//! Session identity derivation.
//!
//! `session_id = hash(repo_root + worktree + frontend)`.
//! Paths are normalized to UTF-8 before hashing so the same repo on
//! different machines or with different mounts produces the same id.

use sha2::{Digest, Sha256};

/// Opaque stable identifier for a single session context.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    /// Derive a session id from the three anchors.
    ///
    /// `repo_root` and `worktree_id` are path-derived hash seeds and must be
    /// normalized before hashing. They are repo/worktree anchors rather than
    /// repo-relative file identities; file-backed snapshot references are
    /// canonicalized separately through `CanonicalRepoPath` before persistence.
    /// `frontend` identifies the calling surface (e.g. `"cli"`, `"mcp"`,
    /// `"vscode"`).
    pub fn derive(repo_root: &str, worktree_id: &str, frontend: &str) -> Self {
        let normalized_root = normalize_path(repo_root);
        let normalized_wt = normalize_path(worktree_id);

        let mut hasher = Sha256::new();
        hasher.update(normalized_root.as_bytes());
        hasher.update(b"\x00");
        hasher.update(normalized_wt.as_bytes());
        hasher.update(b"\x00");
        hasher.update(frontend.as_bytes());

        let bytes = hasher.finalize();
        let hex = hex_encode(&bytes);
        SessionId(hex)
    }

    /// Return the underlying string representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Normalize a path string: lower-case drive letters, convert `\` to `/`,
/// strip trailing slashes.
fn normalize_path(p: &str) -> String {
    let forward = p.replace('\\', "/");
    // Trim trailing slashes (but keep a lone `/` intact).
    let trimmed = forward.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_ascii_lowercase()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_produce_same_id() {
        let a = SessionId::derive("/home/user/repo", "main", "cli");
        let b = SessionId::derive("/home/user/repo", "main", "cli");
        assert_eq!(a, b);
    }

    #[test]
    fn different_frontend_produces_different_id() {
        let a = SessionId::derive("/home/user/repo", "main", "cli");
        let b = SessionId::derive("/home/user/repo", "main", "mcp");
        assert_ne!(a, b);
    }

    #[test]
    fn different_worktree_produces_different_id() {
        let a = SessionId::derive("/home/user/repo", "main", "cli");
        let b = SessionId::derive("/home/user/repo", "feature-branch", "cli");
        assert_ne!(a, b);
    }

    #[test]
    fn trailing_slash_normalized() {
        let a = SessionId::derive("/home/user/repo/", "main", "cli");
        let b = SessionId::derive("/home/user/repo", "main", "cli");
        assert_eq!(a, b);
    }

    #[test]
    fn backslash_normalized() {
        let a = SessionId::derive("C:\\Users\\repo", "main", "cli");
        let b = SessionId::derive("C:/Users/repo", "main", "cli");
        assert_eq!(a, b);
    }
}
