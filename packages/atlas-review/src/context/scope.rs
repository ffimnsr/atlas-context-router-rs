//! Directory-semantic query (DSQ) scope for context assembly (VR2).
//!
//! Scope is canonicalized through [`atlas_repo::CanonicalRepoPath`] and
//! resolved to in-memory prefix predicates before target resolution or
//! selection runs, mirroring the SQL predicates used by the graph and content
//! stores.

use atlas_core::{AtlasError, Result, SearchQuery};
use atlas_repo::CanonicalRepoPath;

use super::*;

/// Canonical directory scope for one context request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextScope {
    /// Recursive canonical path prefix.
    pub subpath: Option<String>,
    /// Non-recursive canonical parent directory.
    pub exact_dir: Option<String>,
    /// Canonical prefixes excluded from resolution and selection.
    pub exclude_subpaths: Vec<String>,
}

impl ContextScope {
    /// Build a validated scope from a request.
    pub fn from_request(request: &ContextRequest) -> Result<Self> {
        Self::new(
            request.subpath.as_deref(),
            request.exact_dir.as_deref(),
            &request.exclude_subpaths,
        )
    }

    /// Canonicalize every provided path, failing closed on invalid input.
    pub fn new(
        subpath: Option<&str>,
        exact_dir: Option<&str>,
        exclude_subpaths: &[String],
    ) -> Result<Self> {
        let canonicalize = |value: &str| {
            CanonicalRepoPath::from_repo_relative(value)
                .map(|path| path.as_str().to_owned())
                .map_err(|error| {
                    AtlasError::Other(format!("invalid directory scope '{value}': {error}"))
                })
        };
        Ok(Self {
            subpath: subpath.map(canonicalize).transpose()?,
            exact_dir: exact_dir.map(canonicalize).transpose()?,
            exclude_subpaths: exclude_subpaths
                .iter()
                .map(|value| canonicalize(value))
                .collect::<Result<Vec<_>>>()?,
        })
    }

    /// True when no scope predicate is active.
    pub fn is_active(&self) -> bool {
        self.subpath.is_some() || self.exact_dir.is_some() || !self.exclude_subpaths.is_empty()
    }

    /// Apply the scope to a search query used during target resolution.
    pub fn apply_to_search(&self, query: &mut SearchQuery) {
        query.subpath = self.subpath.clone();
        query.exact_dir = self.exact_dir.clone();
        query.exclude_subpaths = self.exclude_subpaths.clone();
    }

    /// True when `path` is inside the active scope.
    pub fn contains(&self, path: &str) -> bool {
        if let Some(prefix) = self.subpath.as_deref()
            && !path_starts_with(path, prefix)
        {
            return false;
        }
        if let Some(dir) = self.exact_dir.as_deref()
            && parent_dir(path) != dir
        {
            return false;
        }
        !self
            .exclude_subpaths
            .iter()
            .any(|prefix| path_starts_with(path, prefix))
    }
}

/// Raw lexicographic prefix predicate, matching the SQL DSQ range bounds.
fn path_starts_with(path: &str, prefix: &str) -> bool {
    path >= prefix && path < scope_upper_bound(prefix).as_str()
}

/// Canonical parent directory; files at the repo root have an empty parent.
fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

/// Strict lexicographic upper bound equivalent to `path.starts_with(prefix)`.
fn scope_upper_bound(prefix: &str) -> String {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = char::from_u32(last as u32 + 1) {
            chars.push(next);
            break;
        }
    }
    chars.into_iter().collect()
}

/// Drop selected nodes, edges, and files outside the active scope.
///
/// Selection is filtered after ranking so scope cannot widen or reorder the
/// candidate set; it only removes out-of-scope evidence.
pub(super) fn apply_context_scope(result: &mut ContextResult, scope: &ContextScope) {
    if !scope.is_active() {
        return;
    }
    result
        .nodes
        .retain(|selected| scope.contains(&selected.node.file_path));
    result.files.retain(|file| scope.contains(&file.path));
    let kept: HashSet<String> = result
        .nodes
        .iter()
        .map(|selected| selected.node.qualified_name.clone())
        .collect();
    result.edges.retain(|selected| {
        let source_kept = kept.is_empty() || kept.contains(&selected.edge.source_qn);
        let target_kept = kept.is_empty() || kept.contains(&selected.edge.target_qn);
        source_kept || target_kept
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_scope_matches_lexicographic_prefix() {
        let scope = ContextScope::new(Some("src/auth"), None, &[]).unwrap();
        assert!(scope.contains("src/auth/mod.rs"));
        assert!(scope.contains("src/auth/deep/mod.rs"));
        assert!(!scope.contains("src/session/mod.rs"));
    }

    #[test]
    fn exact_dir_is_non_recursive() {
        let scope = ContextScope::new(None, Some("src/auth"), &[]).unwrap();
        assert!(scope.contains("src/auth/mod.rs"));
        assert!(!scope.contains("src/auth/deep/mod.rs"));
        assert!(!scope.contains("src/session/mod.rs"));
    }

    #[test]
    fn exclusions_prune_subtrees() {
        let scope = ContextScope::new(Some("src"), None, &["src/generated".to_owned()]).unwrap();
        assert!(scope.contains("src/auth/mod.rs"));
        assert!(!scope.contains("src/generated/schema.rs"));
    }

    #[test]
    fn traversal_scope_is_rejected() {
        assert!(ContextScope::new(Some("../escape"), None, &[]).is_err());
        assert!(ContextScope::new(None, Some("/abs/path"), &[]).is_err());
        assert!(ContextScope::new(None, None, &["a/../../b".to_owned()]).is_err());
    }

    #[test]
    fn search_query_scope_round_trips() {
        let scope =
            ContextScope::new(Some("src"), Some("src/auth"), &["src/gen".to_owned()]).unwrap();
        let mut query = SearchQuery::default();
        scope.apply_to_search(&mut query);
        assert_eq!(query.subpath.as_deref(), Some("src"));
        assert_eq!(query.exact_dir.as_deref(), Some("src/auth"));
        assert_eq!(query.exclude_subpaths, vec!["src/gen".to_owned()]);
    }
}
