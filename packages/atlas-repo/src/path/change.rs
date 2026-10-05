//! Missing-path resolution for change workflows.
//!
//! Existing files are resolved by [`super::normalize_repo_file_path`], which
//! uses on-disk existence to disambiguate root-like prefixes. Deleted files
//! cannot offer that signal for the full input, so this module strips prefixes
//! only when a deterministic, non-destructive signal marks them as copies
//! rather than repo-relative paths.

use camino::{Utf8Component, Utf8Path};

use super::{CanonicalRepoPath, NormalizedRepoPath, RepoPathError, normalize_identity};

/// Resolve a path that does not exist on disk to canonical repo-relative
/// identity.
///
/// Resolution order:
/// 1. literal repo-relative identity, reason `"missing_path"`;
/// 2. a leading segment equal to the repo directory name is stripped when that
///    name is *not* also a real directory in the repo (reason
///    `"missing_path_stripped_duplicated_root_prefix"`). When the name *is* a
///    real directory the repo-relative reading wins — the same preference the
///    existing-path resolver applies to its direct candidate;
/// 3. when the leading segment is not a real directory, an embedded repo
///    directory name is stripped if the segments before it are a suffix of the
///    repo root's ancestor chain, i.e. the input is an ancestor-relative copy
///    of a path inside the repo (reason
///    `"missing_path_stripped_embedded_root_prefix"`).
///
/// A missing leading segment without either signal stays literal: it may be a
/// foreign-root prefix or a deleted directory, and only graph state (which
/// this crate does not depend on) can tell those apart.
pub(super) fn resolve_missing_change_path(
    repo_root: &Utf8Path,
    raw: &str,
) -> std::result::Result<NormalizedRepoPath, RepoPathError> {
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err(RepoPathError::Empty);
    }

    // Absolute inputs stay lexical; callers verify they are under the root.
    if Utf8Path::new(&normalized).is_absolute() {
        return missing_path_from_segments(
            repo_root,
            &normalized
                .split('/')
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>(),
            "missing_path",
        );
    }

    if normalized.ends_with('/') {
        return Err(RepoPathError::TrailingSlash(normalized));
    }

    let segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return Err(RepoPathError::Empty);
    }

    let repo_name = repo_root.file_name().unwrap_or_default();
    let first_segment_is_dir = repo_root.join(segments[0]).is_dir();

    // Duplicated repo-dir prefix, unless that name is also a real subdirectory:
    // then the repo-relative reading wins instead of a possibly destructive
    // mis-strip.
    if segments.len() > 1 && segments_equal(segments[0], repo_name) && !first_segment_is_dir {
        return missing_path_from_segments(
            repo_root,
            &segments[1..],
            "missing_path_stripped_duplicated_root_prefix",
        );
    }

    // Ancestor-relative copies embed the repo dir name behind its ancestor
    // chain (`projects/myrepo/src/gone.rs`). Require both the embedded name and
    // the ancestor-chain evidence so a deleted directory that merely shares the
    // repo name is not stripped.
    if !first_segment_is_dir {
        for (index, segment) in segments
            .iter()
            .enumerate()
            .take(segments.len().saturating_sub(1))
            .skip(1)
        {
            if segments_equal(segment, repo_name)
                && ancestor_chain_suffix(repo_root, &segments[..index])
            {
                return missing_path_from_segments(
                    repo_root,
                    &segments[index + 1..],
                    "missing_path_stripped_embedded_root_prefix",
                );
            }
        }
    }

    missing_path_from_segments(repo_root, &segments, "missing_path")
}

/// Enumerate deterministic prefix-stripped candidates for a missing
/// repo-relative change path, longest tail first (one leading segment removed
/// first).
///
/// Existence and graph checks are intentionally absent: this crate has no
/// graph state. Callers that do (for example the update engine) use a
/// candidate when its path is absent from disk but still present in the graph,
/// which resolves prefixed copies such as `other-checkout/src/gone.rs` without
/// guessing at live files.
///
/// Absolute inputs have no relative prefix to strip and yield no candidates.
/// Escaping tails (`..` prefixes) are skipped.
pub fn missing_change_path_candidates(repo_root: &Utf8Path, raw: &str) -> Vec<NormalizedRepoPath> {
    let mut candidates: Vec<NormalizedRepoPath> = Vec::new();
    let normalized = raw.trim().replace('\\', "/");
    if normalized.is_empty() || Utf8Path::new(&normalized).is_absolute() {
        return candidates;
    }
    let segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    for strip_count in 1..segments.len() {
        let tail = segments[strip_count..].join("/");
        let Ok(canonical) = CanonicalRepoPath::from_cli_argument(repo_root, Utf8Path::new(&tail))
        else {
            continue;
        };
        if candidates
            .iter()
            .any(|candidate| candidate.canonical == canonical.as_str())
        {
            continue;
        }
        candidates.push(NormalizedRepoPath {
            canonical: canonical.as_str().to_owned(),
            reason: if strip_count == 1 {
                "missing_path_stripped_foreign_root_prefix"
            } else {
                "missing_path_stripped_nested_subdir_prefix"
            },
        });
    }
    candidates
}

fn missing_path_from_segments(
    repo_root: &Utf8Path,
    segments: &[&str],
    reason: &'static str,
) -> std::result::Result<NormalizedRepoPath, RepoPathError> {
    let joined = segments.join("/");
    let canonical = CanonicalRepoPath::from_cli_argument(repo_root, Utf8Path::new(&joined))?;
    Ok(NormalizedRepoPath {
        canonical: canonical.as_str().to_owned(),
        reason,
    })
}

fn segments_equal(left: &str, right: &str) -> bool {
    normalize_identity(left) == normalize_identity(right)
}

/// True when `leading` equals a suffix of the repo root's ancestor chain.
fn ancestor_chain_suffix(repo_root: &Utf8Path, leading: &[&str]) -> bool {
    if leading.is_empty() {
        return false;
    }
    let Some(parent) = repo_root.parent() else {
        return false;
    };
    let parent_parts = parent
        .components()
        .filter_map(|component| match component {
            Utf8Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect::<Vec<_>>();
    if leading.len() > parent_parts.len() {
        return false;
    }
    let start = parent_parts.len() - leading.len();
    parent_parts[start..]
        .iter()
        .zip(leading)
        .all(|(parent_part, leading_part)| segments_equal(parent_part, leading_part))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize_repo_change_path;

    #[test]
    fn duplicated_root_prefix_strips_when_name_is_not_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();
        let repo_name = root.file_name().unwrap();

        let resolved =
            normalize_repo_change_path(root, &format!("{repo_name}/src/gone.rs")).unwrap();
        assert_eq!(resolved.canonical, "src/gone.rs");
        assert_eq!(
            resolved.reason,
            "missing_path_stripped_duplicated_root_prefix"
        );
    }

    #[test]
    fn duplicated_root_prefix_stays_literal_when_name_is_a_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        let repo_name = root.file_name().unwrap();
        std::fs::create_dir_all(root.join(repo_name).join("src").as_std_path()).unwrap();

        let input = format!("{repo_name}/src/gone.rs");
        let resolved = normalize_repo_change_path(root, &input).unwrap();
        assert_eq!(
            resolved.canonical, input,
            "repo-relative reading must win when the root prefix is a real directory"
        );
        assert_eq!(resolved.reason, "missing_path");
    }

    #[test]
    fn embedded_root_prefix_strips_for_ancestor_relative_copies() {
        let outer = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(outer.path())
            .unwrap()
            .join("nested")
            .join("myrepo");
        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();

        let resolved = normalize_repo_change_path(&root, "nested/myrepo/src/gone.rs").unwrap();
        assert_eq!(resolved.canonical, "src/gone.rs");
        assert_eq!(
            resolved.reason,
            "missing_path_stripped_embedded_root_prefix"
        );

        // Same embedded name without ancestor-chain evidence stays literal.
        let unresolved = normalize_repo_change_path(&root, "elsewhere/myrepo/src/gone.rs").unwrap();
        assert_eq!(unresolved.canonical, "elsewhere/myrepo/src/gone.rs");
        assert_eq!(unresolved.reason, "missing_path");
    }

    #[test]
    fn foreign_leading_segment_stays_literal_for_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();

        // A missing leading segment may be a foreign root or a deleted
        // directory; without graph state the literal reading is the only safe
        // choice.
        let resolved = normalize_repo_change_path(root, "other-repo/src/gone.rs").unwrap();
        assert_eq!(resolved.canonical, "other-repo/src/gone.rs");
        assert_eq!(resolved.reason, "missing_path");
    }

    #[test]
    fn missing_change_path_candidates_enumerate_stripped_tails() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();

        let candidates = missing_change_path_candidates(root, "other-checkout/src/gone.rs");
        let paths = candidates
            .iter()
            .map(|candidate| candidate.canonical.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["src/gone.rs", "gone.rs"]);
        assert_eq!(
            candidates[0].reason,
            "missing_path_stripped_foreign_root_prefix"
        );
        assert_eq!(
            candidates[1].reason,
            "missing_path_stripped_nested_subdir_prefix"
        );

        // Absolute inputs have no relative prefix to strip.
        assert!(missing_change_path_candidates(root, "/abs/gone.rs").is_empty());

        // Escaping tails are skipped; the rest stay deterministic.
        let escaping = missing_change_path_candidates(root, "a/../b/gone.rs")
            .iter()
            .map(|candidate| candidate.canonical.clone())
            .collect::<Vec<_>>();
        assert_eq!(escaping, ["b/gone.rs", "gone.rs"]);
    }

    #[test]
    fn missing_paths_still_fail_closed_on_invalid_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src").as_std_path()).unwrap();

        // `..` segments can resolve physically outside the root, so the error
        // is a not-under-root/escape variant rather than a specific one.
        assert!(normalize_repo_change_path(root, "../outside.rs").is_err());
        assert!(matches!(
            normalize_repo_change_path(root, "src/gone/").unwrap_err(),
            RepoPathError::TrailingSlash(_)
        ));
        assert_eq!(
            normalize_repo_change_path(root, "   ").unwrap_err(),
            RepoPathError::Empty
        );

        let absolute = root.join("src/abs_gone.rs");
        let resolved = normalize_repo_change_path(root, absolute.as_str()).unwrap();
        assert_eq!(resolved.canonical, "src/abs_gone.rs");
        assert_eq!(resolved.reason, "absolute");
    }
}
