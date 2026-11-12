//! TrieHI in-memory path trie for directory-semantic scope (VR2 stage 2).
//!
//! Each trie node owns two roaring bitmaps over retrieval-chunk row ids:
//!
//! * `local` — chunks whose file lives directly in this directory
//! * `inclusive` — `local` union every child's `inclusive` subtree set
//!
//! The invariant `inclusive(v) = local(v) ∪ ⋃ inclusive(child)` lets recursive
//! and non-recursive DSQ predicates resolve without SQL and without scanning
//! out-of-scope chunks before cosine scoring.

use std::collections::BTreeMap;

use atlas_core::{AtlasError, Result};
use roaring::RoaringBitmap;

use super::retrieval::VectorScope;

/// One directory in the path trie.
#[derive(Debug, Default, Clone)]
pub struct TrieNode {
    /// Chunks whose file lives directly in this directory.
    pub local: RoaringBitmap,
    /// `local` plus every descendant directory's chunks.
    pub inclusive: RoaringBitmap,
    /// Child directories keyed by the last path segment.
    pub children: BTreeMap<String, TrieNode>,
}

/// Repo-scoped directory trie over retrieval-chunk ids.
#[derive(Debug, Default, Clone)]
pub struct DirectoryTrie {
    repos: BTreeMap<String, TrieNode>,
    len: usize,
}

impl DirectoryTrie {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from `(repo_id, file_path, chunk_id)` rows.
    pub fn build<I>(rows: I) -> Self
    where
        I: IntoIterator<Item = (String, String, u32)>,
    {
        let mut trie = Self::new();
        for (repo, file_path, chunk_id) in rows {
            trie.insert(&repo, &file_path, chunk_id);
        }
        trie
    }

    /// Number of indexed chunks.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Insert one chunk at its file's parent directory, updating ancestors in
    /// `O(depth)`.
    pub fn insert(&mut self, repo_id: &str, file_path: &str, chunk_id: u32) {
        let segments = directory_segments(file_path);
        let root = self.repos.entry(repo_id.to_owned()).or_default();
        root.inclusive.insert(chunk_id);
        let mut node = root;
        for segment in segments {
            node = node.children.entry(segment.to_owned()).or_default();
            node.inclusive.insert(chunk_id);
        }
        node.local.insert(chunk_id);
        self.len += 1;
    }

    /// Remove one chunk from its file's parent directory. Returns `false` when
    /// the chunk was not present.
    pub fn remove(&mut self, repo_id: &str, file_path: &str, chunk_id: u32) -> bool {
        let segments = directory_segments(file_path);
        let Some(root) = self.repos.get_mut(repo_id) else {
            return false;
        };
        if !root.inclusive.contains(chunk_id) {
            return false;
        }
        let removed = remove_from_chain(root, &segments, chunk_id);
        if removed {
            self.len = self.len.saturating_sub(1);
        }
        removed
    }

    /// Relink one chunk from `old_path` to `new_path` (DSM MOVE).
    ///
    /// Removes from the old ancestor chain and adds to the new one in
    /// `O(depth)` each. Returns `false` when the old entry was missing, which
    /// callers treat as "rebuild from `retrieval_chunks`".
    pub fn relink(&mut self, repo_id: &str, old_path: &str, new_path: &str, chunk_id: u32) -> bool {
        if old_path == new_path {
            return self
                .repos
                .get(repo_id)
                .is_some_and(|root| root.inclusive.contains(chunk_id));
        }
        if !self.remove(repo_id, old_path, chunk_id) {
            return false;
        }
        self.insert(repo_id, new_path, chunk_id);
        true
    }

    /// Resolve recursive, non-recursive, and exclusion predicates to one
    /// bitmap without SQL.
    pub fn resolve(&self, scope: &VectorScope) -> Result<RoaringBitmap> {
        let has_path_scope = scope.subpath_prefix.is_some()
            || scope.exact_dir.is_some()
            || !scope.exclude_prefixes.is_empty();
        if has_path_scope && scope.repo_ids.is_empty() {
            return Err(AtlasError::Other(
                "directory scope requires an explicit repo scope; refusing to scan every repo"
                    .to_owned(),
            ));
        }

        // Prefix scopes are resolved directory-wise; a path that is not an
        // indexed directory yields an empty result rather than a partial one.
        let mut matches = RoaringBitmap::new();
        let repos: Vec<(&String, &TrieNode)> = if scope.repo_ids.is_empty() {
            self.repos.iter().collect()
        } else {
            scope
                .repo_ids
                .iter()
                .filter_map(|repo_id| self.repos.get_key_value(repo_id))
                .collect()
        };

        for (_, root) in repos {
            let mut selected = root.inclusive.clone();
            if let Some(prefix) = scope.subpath_prefix.as_deref() {
                selected = node_at(root, prefix)
                    .map(|node| node.inclusive.clone())
                    .unwrap_or_default();
            }
            if let Some(dir) = scope.exact_dir.as_deref() {
                let local = node_at(root, dir)
                    .map(|node| node.local.clone())
                    .unwrap_or_default();
                selected &= local;
            }
            for prefix in &scope.exclude_prefixes {
                if let Some(node) = node_at(root, prefix) {
                    selected -= &node.inclusive;
                }
            }
            matches |= selected;
        }
        Ok(matches)
    }

    /// Check the `inclusive = local ∪ children` invariant for every node.
    pub fn check_invariants(&self) -> bool {
        self.repos.values().all(check_node)
    }
}

fn check_node(node: &TrieNode) -> bool {
    let mut expected = node.local.clone();
    for child in node.children.values() {
        if !check_node(child) {
            return false;
        }
        expected |= &child.inclusive;
    }
    expected == node.inclusive
}

/// Directory segments for a file path; a bare file lives at the repo root.
fn directory_segments(file_path: &str) -> Vec<&str> {
    match file_path.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent.split('/').collect(),
        _ => Vec::new(),
    }
}

fn remove_from_chain(node: &mut TrieNode, segments: &[&str], chunk_id: u32) -> bool {
    let Some((segment, rest)) = segments.split_first() else {
        if !node.local.remove(chunk_id) {
            return false;
        }
        node.inclusive.remove(chunk_id);
        return true;
    };
    let Some(child) = node.children.get_mut(*segment) else {
        return false;
    };
    let removed = remove_from_chain(child, rest, chunk_id);
    if removed {
        node.inclusive.remove(chunk_id);
        if child.inclusive.is_empty() && child.local.is_empty() && child.children.is_empty() {
            node.children.remove(*segment);
        }
    }
    removed
}

fn node_at<'a>(root: &'a TrieNode, path: &str) -> Option<&'a TrieNode> {
    let mut node = root;
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        node = node.children.get(segment)?;
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<(String, String, u32)> {
        vec![
            ("r".to_owned(), "src/auth/token.rs".to_owned(), 1),
            ("r".to_owned(), "src/auth/mod.rs".to_owned(), 2),
            ("r".to_owned(), "src/session/store.rs".to_owned(), 3),
            ("r".to_owned(), "src/lib.rs".to_owned(), 4),
            ("r".to_owned(), "tests/auth_test.rs".to_owned(), 5),
        ]
    }

    #[test]
    fn insert_maintains_inclusive_invariant() {
        let trie = DirectoryTrie::build(rows());
        assert_eq!(trie.len(), 5);
        assert!(trie.check_invariants());
    }

    #[test]
    fn recursive_scope_resolves_subtree() {
        let trie = DirectoryTrie::build(rows());
        let bitmap = trie
            .resolve(&VectorScope {
                subpath_prefix: Some("src/auth".to_owned()),
                repo_ids: vec!["r".to_owned()],
                ..VectorScope::default()
            })
            .unwrap();
        assert_eq!(bitmap.iter().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn exact_dir_is_non_recursive() {
        let trie = DirectoryTrie::build(rows());
        let bitmap = trie
            .resolve(&VectorScope {
                exact_dir: Some("src/auth".to_owned()),
                repo_ids: vec!["r".to_owned()],
                ..VectorScope::default()
            })
            .unwrap();
        assert_eq!(bitmap.iter().collect::<Vec<_>>(), vec![1, 2]);
        let root = trie
            .resolve(&VectorScope {
                exact_dir: Some("src".to_owned()),
                repo_ids: vec!["r".to_owned()],
                ..VectorScope::default()
            })
            .unwrap();
        assert_eq!(root.iter().collect::<Vec<_>>(), vec![4]);
    }

    #[test]
    fn exclusions_prune_subtrees() {
        let trie = DirectoryTrie::build(rows());
        let bitmap = trie
            .resolve(&VectorScope {
                subpath_prefix: Some("src".to_owned()),
                exclude_prefixes: vec!["src/session".to_owned()],
                repo_ids: vec!["r".to_owned()],
                ..VectorScope::default()
            })
            .unwrap();
        assert_eq!(bitmap.iter().collect::<Vec<_>>(), vec![1, 2, 4]);
    }

    #[test]
    fn move_delete_and_merge_preserve_invariant() {
        let mut trie = DirectoryTrie::build(rows());

        // Move
        assert!(trie.relink("r", "src/auth/token.rs", "src/session/token.rs", 1));
        assert!(trie.check_invariants());
        let auth = trie
            .resolve(&VectorScope {
                subpath_prefix: Some("src/auth".to_owned()),
                repo_ids: vec!["r".to_owned()],
                ..VectorScope::default()
            })
            .unwrap();
        assert_eq!(auth.iter().collect::<Vec<_>>(), vec![2]);

        // Delete
        assert!(trie.remove("r", "src/session/token.rs", 1));
        assert!(!trie.remove("r", "src/session/token.rs", 1));
        assert!(trie.check_invariants());

        // Merge (insert another trie's rows)
        let extra = vec![
            ("r2".to_owned(), "src/auth/new.rs".to_owned(), 9),
            ("r".to_owned(), "src/auth/extra.rs".to_owned(), 10),
        ];
        for (repo, path, id) in extra {
            trie.insert(&repo, &path, id);
        }
        assert!(trie.check_invariants());
        assert_eq!(trie.len(), 6);
    }

    #[test]
    fn path_scope_without_repo_fails_closed() {
        let trie = DirectoryTrie::build(rows());
        assert!(
            trie.resolve(&VectorScope {
                subpath_prefix: Some("src".to_owned()),
                ..VectorScope::default()
            })
            .is_err()
        );
    }
}
