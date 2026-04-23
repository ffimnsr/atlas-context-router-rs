use std::collections::HashMap;

/// In-process cache of tree-sitter parse trees, keyed by canonical repo-relative
/// path.
///
/// Enables incremental re-parsing within a single process lifetime: after a
/// file is parsed the caller stores the resulting tree here and retrieves it
/// on the next parse of the same file, passing it as
/// [`crate::traits::ParseContext::old_tree`].  tree-sitter then reuses
/// unchanged subtrees rather than parsing the whole file from scratch.
///
/// This cache is in-memory only, but it still follows the canonical path
/// identity invariant: callers must key it with the canonical repo-path string
/// they persist to graph/content/session state, not a raw watch, CLI, or git
/// path spelling.
///
/// The cache is intentionally in-memory only; trees are not serialisable
/// across process boundaries, so a fresh run always starts empty.
pub struct TreeCache {
    trees: HashMap<String, tree_sitter::Tree>,
}

impl Default for TreeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            trees: HashMap::new(),
        }
    }

    /// Return a reference to the cached tree for canonical `path`, if any.
    pub fn get(&self, path: &str) -> Option<&tree_sitter::Tree> {
        self.trees.get(path)
    }

    /// Store (or replace) the tree for canonical `path`.
    pub fn insert(&mut self, path: String, tree: tree_sitter::Tree) {
        self.trees.insert(path, tree);
    }

    /// Remove and return the tree for canonical `path`.  Callers can move the tree into
    /// a parallel work item without cloning; re-insert the returned tree
    /// afterwards to restore the cache entry.
    pub fn remove(&mut self, path: &str) -> Option<tree_sitter::Tree> {
        self.trees.remove(path)
    }

    /// Drop the cached tree for canonical `path`, e.g. when a file is deleted.
    pub fn evict(&mut self, path: &str) {
        self.trees.remove(path);
    }

    /// Number of cached trees.
    pub fn len(&self) -> usize {
        self.trees.len()
    }

    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_evict() {
        let mut cache = TreeCache::new();
        assert!(cache.is_empty());

        // Build a tiny real tree so we have a Tree to store.
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(b"fn main() {}", None).unwrap();

        cache.insert("src/main.rs".into(), tree);
        assert_eq!(cache.len(), 1);
        assert!(cache.get("src/main.rs").is_some());

        cache.evict("src/main.rs");
        assert!(cache.is_empty());
    }

    #[test]
    fn remove_returns_tree() {
        let mut cache = TreeCache::new();
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(b"fn foo() {}", None).unwrap();
        cache.insert("a.rs".into(), tree);

        let removed = cache.remove("a.rs");
        assert!(removed.is_some());
        assert!(cache.is_empty());
    }

    /// Verify that the registry round-trip (parse → cache → incremental
    /// re-parse) produces the same graph output as a fresh parse.
    #[test]
    fn incremental_reparse_via_registry_matches_fresh() {
        use crate::ParserRegistry;

        let registry = ParserRegistry::with_defaults();
        let src = b"fn hello() {} fn world() {}";

        // First parse: no old tree.
        let (pf1, tree1) = registry
            .parse("src/lib.rs", "hash1", src, None)
            .expect("should parse");
        let tree1 = tree1.expect("rust parser must return a tree");

        // Second parse: supply the tree from the first run.
        let (pf2, _tree2) = registry
            .parse("src/lib.rs", "hash2", src, Some(&tree1))
            .expect("should parse with old tree");

        // Symbol names must be identical regardless of whether the tree was
        // reused.
        let qns1: Vec<&str> = pf1
            .nodes
            .iter()
            .map(|n| n.qualified_name.as_str())
            .collect();
        let qns2: Vec<&str> = pf2
            .nodes
            .iter()
            .map(|n| n.qualified_name.as_str())
            .collect();
        assert_eq!(qns1, qns2, "incremental re-parse must produce same nodes");
    }
}
