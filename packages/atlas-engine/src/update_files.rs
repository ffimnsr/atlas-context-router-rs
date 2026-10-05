//! Explicit-file change classification and deletion helpers for incremental
//! updates.
//!
//! `atlas update --files` and MCP `change_source.files` may reference paths
//! that were deleted before the update ran. Missing files are classified as
//! deletions so the update cleans the stale graph slice without parse errors
//! or warnings. Repo-path identity comes from `atlas_repo::normalize_repo_change_path`.
//!
//! Deletions are applied only when the graph still has a footprint for the
//! path; repeating the same explicit deletion is therefore a no-op instead of
//! re-reporting `deleted: 1`. A missing directory path deletes every indexed
//! file below it, matching `git status`-style whole-tree removal.
//!
//! Missing paths whose literal identity has no graph footprint are also
//! retargeted through graph evidence: when a prefix-stripped candidate is
//! absent from disk but still present in the graph, the candidate is the
//! deleted file (`other-checkout/src/gone.rs` -> `src/gone.rs`). The
//! retarget never lands on a file that still exists on disk.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use atlas_core::model::{ChangeType, ChangedFile};
use atlas_repo::{missing_change_path_candidates, normalize_repo_change_path};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

/// Classify an explicit update path as modified or deleted.
///
/// Missing paths keep their canonical literal identity unless the graph proves
/// a better reading: a prefixed copy such as `other-checkout/src/gone.rs` is
/// retargeted onto its stripped tail when that tail has a graph footprint and
/// is itself absent from disk. Live files are never retargeted onto.
pub(crate) fn explicit_file_change(
    store: &Store,
    repo_root: &Utf8Path,
    source_repo_id: &str,
    raw: &str,
) -> Result<ChangedFile> {
    let rel = resolve_explicit_change_path(store, repo_root, source_repo_id, raw)?;
    let exists = repo_root.join(&rel).exists();
    Ok(ChangedFile {
        path: rel,
        change_type: if exists {
            ChangeType::Modified
        } else {
            ChangeType::Deleted
        },
        old_path: None,
    })
}

fn resolve_explicit_change_path(
    store: &Store,
    repo_root: &Utf8Path,
    source_repo_id: &str,
    raw: &str,
) -> Result<String> {
    let resolved = normalize_repo_change_path(repo_root, raw)
        .with_context(|| format!("invalid explicit update path '{raw}'"))?;
    let canonical = resolved.canonical;
    if repo_root.join(&canonical).exists() {
        return Ok(canonical);
    }
    // Absolute inputs are exact user intent (reason string matched to
    // `path.rs` by the unit tests below): never retarget them onto stripped
    // tails. Callers that pre-normalize to relative form (CLI/MCP) convey
    // relative intent instead, where the disk guard still bounds any retarget
    // to stale slices of already-missing paths.
    if resolved.reason == "absolute" {
        return Ok(canonical);
    }
    // A real first-level directory marks the input as an ordinary
    // repo-relative path, not a prefixed copy: no candidate scanning.
    if canonical
        .split('/')
        .next()
        .is_some_and(|segment| repo_root.join(segment).is_dir())
    {
        return Ok(canonical);
    }
    if graph_has_change_target(store, source_repo_id, &canonical)? {
        return Ok(canonical);
    }

    // Graph evidence resolves prefixed copies that the existence-based
    // resolver must leave literal. Candidates arrive longest-tail first, so
    // the most conservative reading wins; the disk guard only ever selects
    // stale slices of already-missing files and directories, never live ones.
    for candidate in missing_change_path_candidates(repo_root, &canonical) {
        if repo_root.join(&candidate.canonical).exists() {
            continue;
        }
        if graph_has_change_target(store, source_repo_id, &candidate.canonical)? {
            return Ok(candidate.canonical);
        }
    }
    Ok(canonical)
}

/// True when the graph knows the path as a file slice or as a directory with
/// indexed descendants (deleted whole-tree target).
fn graph_has_change_target(store: &Store, source_repo_id: &str, path: &str) -> Result<bool> {
    if store
        .file_graph_exists_for_repo(source_repo_id, path)
        .with_context(|| format!("cannot inspect graph for '{path}'"))?
    {
        return Ok(true);
    }
    Ok(!store
        .file_paths_under_dir_for_repo(source_repo_id, path)
        .with_context(|| format!("cannot list graph files under '{path}'"))?
        .is_empty())
}

/// Deletion paths that still have (or contain) graph data for
/// `source_repo_id`, deduplicated and sorted.
///
/// A missing path may be a file or a deleted directory. Exact footprints
/// cover files; indexed descendants of `path/` cover directory deletions.
/// Paths with no remaining footprint are dropped so repeated explicit
/// `--files` updates converge to a no-op summary.
pub(crate) fn existing_file_graph_deletions(
    store: &Store,
    source_repo_id: &str,
    paths: &[String],
) -> Result<Vec<String>> {
    let mut existing = BTreeSet::new();
    for path in paths {
        if store
            .file_graph_exists_for_repo(source_repo_id, path)
            .with_context(|| format!("cannot inspect graph for '{path}'"))?
        {
            existing.insert(path.clone());
        }
        // Directory deletion: expand to every indexed file below `path/`.
        for child in store
            .file_paths_under_dir_for_repo(source_repo_id, path)
            .with_context(|| format!("cannot list graph files under '{path}'"))?
        {
            existing.insert(child);
        }
    }
    Ok(existing.into_iter().collect())
}

/// Delete graph slices for paths that still have one, returning the count of
/// slices actually removed.
pub(crate) fn delete_existing_file_graphs(
    store: &mut Store,
    source_repo_id: &str,
    paths: &[String],
) -> Result<usize> {
    let existing = existing_file_graph_deletions(store, source_repo_id, paths)?;
    for path in &existing {
        store
            .delete_file_graph_for_repo(source_repo_id, path)
            .with_context(|| format!("cannot delete graph for '{path}'"))?;
    }
    Ok(existing.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{Node, NodeId, NodeKind, model::ParsedFile};

    fn function_node(qn: &str, path: &str) -> Node {
        Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: "x".to_string(),
            qualified_name: qn.to_string(),
            file_path: path.to_string(),
            line_start: 1,
            line_end: 10,
            language: "rust".to_string(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "h1".to_string(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    fn parsed_file(path: &str) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: Some("rust".to_string()),
            hash: "h1".to_string(),
            size: None,
            nodes: vec![function_node(&format!("{}::fn::x", path), path)],
            edges: vec![],
        }
    }

    #[test]
    fn explicit_file_change_marks_missing_paths_as_deleted() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();
        std::fs::write(root.join("src/lib.rs").as_std_path(), "").unwrap();

        let existing = explicit_file_change(&store, root, "repo_a", "src/lib.rs").unwrap();
        assert_eq!(existing.path, "src/lib.rs");
        assert_eq!(existing.change_type, ChangeType::Modified);

        let missing = explicit_file_change(&store, root, "repo_a", "src/gone.rs").unwrap();
        assert_eq!(missing.path, "src/gone.rs");
        assert_eq!(missing.change_type, ChangeType::Deleted);
    }

    #[test]
    fn explicit_file_change_does_not_retarget_under_a_real_directory() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        store
            .replace_files_transactional_for_repo("repo_a", &[parsed_file("gone.rs")])
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();

        // `src` is a real directory, so `src/gone.rs` is an ordinary missing
        // repo-relative path; the stale root `gone.rs` slice must stay put.
        let change = explicit_file_change(&store, root, "repo_a", "src/gone.rs").unwrap();
        assert_eq!(change.path, "src/gone.rs");
        assert_eq!(change.change_type, ChangeType::Deleted);
    }

    #[test]
    fn explicit_file_change_keeps_absolute_missing_paths_literal() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        store
            .replace_files_transactional_for_repo("repo_a", &[parsed_file("gone.rs")])
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();

        // Absolute path whose first segment is not a directory; without the
        // absolute guard the stale `gone.rs` candidate would win.
        let absolute = root.join("other-checkout/gone.rs");
        let change = explicit_file_change(&store, root, "repo_a", absolute.as_str()).unwrap();
        assert_eq!(change.path, "other-checkout/gone.rs");
        assert_eq!(change.change_type, ChangeType::Deleted);
    }

    #[test]
    fn explicit_file_change_retargets_prefixed_copy_via_graph() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        store
            .replace_files_transactional_for_repo("repo_a", &[parsed_file("src/gone.rs")])
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();

        let change =
            explicit_file_change(&store, root, "repo_a", "other-checkout/src/gone.rs").unwrap();
        assert_eq!(change.path, "src/gone.rs");
        assert_eq!(change.change_type, ChangeType::Deleted);
    }

    #[test]
    fn explicit_file_change_retargets_prefixed_deleted_directory() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        store
            .replace_files_transactional_for_repo(
                "repo_a",
                &[parsed_file("old_dir/a.rs"), parsed_file("old_dir/b.rs")],
            )
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();

        // `other-checkout/old_dir` has no exact footprint, but `old_dir` has
        // indexed descendants, so the stripped candidate is the deleted tree.
        let change =
            explicit_file_change(&store, root, "repo_a", "other-checkout/old_dir").unwrap();
        assert_eq!(change.path, "old_dir");
        assert_eq!(change.change_type, ChangeType::Deleted);

        // Literal directory deletion still wins when the literal has children.
        let direct = explicit_file_change(&store, root, "repo_a", "old_dir").unwrap();
        assert_eq!(direct.path, "old_dir");
        assert_eq!(direct.change_type, ChangeType::Deleted);
    }

    #[test]
    fn explicit_file_change_prefers_literal_graph_footprint() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        store
            .replace_files_transactional_for_repo(
                "repo_a",
                &[parsed_file("old_dir/gone.rs"), parsed_file("gone.rs")],
            )
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("old_dir").as_std_path()).unwrap();

        // Both readings have graph footprints; the literal (deleted directory
        // tree) wins over the stripped candidate.
        let change = explicit_file_change(&store, root, "repo_a", "old_dir/gone.rs").unwrap();
        assert_eq!(change.path, "old_dir/gone.rs");
        assert_eq!(change.change_type, ChangeType::Deleted);
    }

    #[test]
    fn repeated_deletion_of_same_path_is_a_no_op() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        store
            .replace_files_transactional_for_repo("repo_a", &[parsed_file("src/a.rs")])
            .unwrap();

        let paths = vec!["src/a.rs".to_string()];
        assert_eq!(
            delete_existing_file_graphs(&mut store, "repo_a", &paths).unwrap(),
            1,
            "first explicit deletion removes the slice"
        );
        assert_eq!(
            delete_existing_file_graphs(&mut store, "repo_a", &paths).unwrap(),
            0,
            "repeating the deletion must not re-report it"
        );
        assert!(
            existing_file_graph_deletions(&store, "repo_a", &paths)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn missing_directory_deletion_expands_to_indexed_children() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        let files = vec![
            parsed_file("src/old_dir/a.rs"),
            parsed_file("src/old_dir/b.rs"),
            // Range-scan neighbours: `_` is a LIKE wildcard, `X` and `0` sort
            // just outside the `src/old_dir/` .. `src/old_dir0` window.
            parsed_file("src/oldXdir/c.rs"),
            parsed_file("src/old_dirX/d.rs"),
            parsed_file("src/old_dir0.rs"),
            parsed_file("src/keep.rs"),
        ];
        store
            .replace_files_transactional_for_repo("repo_a", &files)
            .unwrap();

        let paths = vec!["src/old_dir".to_string()];
        assert_eq!(
            existing_file_graph_deletions(&store, "repo_a", &paths).unwrap(),
            ["src/old_dir/a.rs", "src/old_dir/b.rs"]
        );
        assert_eq!(
            delete_existing_file_graphs(&mut store, "repo_a", &paths).unwrap(),
            2
        );
        assert!(
            !store
                .file_graph_exists_for_repo("repo_a", "src/old_dir/a.rs")
                .unwrap()
        );
        for survivor in [
            "src/oldXdir/c.rs",
            "src/old_dirX/d.rs",
            "src/old_dir0.rs",
            "src/keep.rs",
        ] {
            assert!(
                store
                    .file_graph_exists_for_repo("repo_a", survivor)
                    .unwrap(),
                "{survivor} must survive directory deletion"
            );
        }
        assert_eq!(
            delete_existing_file_graphs(&mut store, "repo_a", &paths).unwrap(),
            0,
            "repeating a directory deletion must not re-report it"
        );
    }
}
