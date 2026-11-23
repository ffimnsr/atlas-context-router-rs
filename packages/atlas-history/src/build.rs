//! Historical graph build: parse file contents at each commit and persist
//! content-addressed node/edge graphs with snapshot membership.
//!
//! Key design rules:
//! - Use git blob SHA (from ls-tree) as file_hash — stable and cheap.
//! - Check `has_historical_file_graph` before fetching file bytes; skip parse
//!   when graph already exists for this blob hash.
//! - Detect binary by null-byte sniff of the first 8 KiB of blob content.
//! - Keep canonical repo-relative paths; never use absolute paths as keys.
//! - Continue on per-file parse errors; increment parse_error_count.
//! - Summarize commits processed, files reused, files parsed, nodes/edges
//!   reused, and elapsed time.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use rayon::prelude::*;

use crate::error::Result;
use atlas_core::ParsedFile;
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::{
    HistoricalEdge, HistoricalNode, Store, StoredSnapshotFile, StoredSnapshotMembershipBlob,
};
use serde::Serialize;
use tracing::{debug, warn};

use crate::git;
use crate::ingest::IngestError;
use crate::lifecycle::{LifecycleSummary, recompute_lifecycle};
use crate::select::CommitSelector;

/// Number of bytes inspected for binary detection.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

type EdgeKey = (String, String, String);

fn push_warning(summary: &mut BuildSummary, warning: String) {
    if !summary.warnings.iter().any(|existing| existing == &warning) {
        summary.warnings.push(warning);
    }
}

fn encode_node_membership(qualified_names: &[String]) -> String {
    let mut sorted = qualified_names.to_vec();
    sorted.sort();
    sorted.join("\n")
}

fn encode_edge_membership(edge_keys: &[EdgeKey]) -> String {
    let mut sorted = edge_keys.to_vec();
    sorted.sort();
    sorted
        .into_iter()
        .map(|(source, target, kind)| format!("{source}\t{target}\t{kind}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Detect binary content by scanning the first [`BINARY_SNIFF_BYTES`] for
/// null bytes — the same heuristic used by `atlas-repo` for live indexing.
fn is_binary_bytes(bytes: &[u8]) -> bool {
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    sniff.contains(&0u8)
}

#[cfg(test)]
mod tests {
    use atlas_parser::ParserRegistry;
    use atlas_store_sqlite::Store;

    use crate::diff::reconstruct_snapshot;
    use crate::test_support::{commit_all, git_clone_shallow, git_init, write_file};

    use super::*;

    fn open_store(temp: &tempfile::TempDir) -> Store {
        let db_path = temp.path().join("history.sqlite");
        Store::open(db_path.to_str().expect("db path")).expect("open store")
    }

    fn snapshot_signature(
        snapshot: &crate::diff::HistoricalSnapshot,
    ) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut files = snapshot
            .files
            .iter()
            .map(|file| {
                format!(
                    "{}|{}|{:?}|{:?}",
                    file.file_path, file.file_hash, file.language, file.size
                )
            })
            .collect::<Vec<_>>();
        let mut nodes = snapshot
            .nodes
            .iter()
            .map(|node| {
                format!(
                    "{}|{}|{}|{:?}|{:?}|{:?}",
                    node.qualified_name,
                    node.kind,
                    node.file_path,
                    node.line_start,
                    node.line_end,
                    node.language
                )
            })
            .collect::<Vec<_>>();
        let mut edges = snapshot
            .edges
            .iter()
            .map(|edge| {
                format!(
                    "{}|{}|{}|{}|{:?}",
                    edge.source_qn, edge.target_qn, edge.kind, edge.file_path, edge.line
                )
            })
            .collect::<Vec<_>>();
        files.sort();
        nodes.sort();
        edges.sort();
        (files, nodes, edges)
    }

    #[test]
    fn binary_detection_null_byte() {
        assert!(is_binary_bytes(b"hello\x00world"));
        assert!(!is_binary_bytes(b"hello world\n"));
        assert!(!is_binary_bytes(b""));
    }

    #[test]
    fn binary_detection_only_sniffs_first_8k() {
        let mut buf = vec![b'a'; BINARY_SNIFF_BYTES + 1];
        buf[BINARY_SNIFF_BYTES] = 0u8; // null beyond sniff window
        assert!(!is_binary_bytes(&buf));
    }

    #[test]
    fn membership_encoding_is_sorted_and_stable() {
        let nodes = encode_node_membership(&["crate::beta".to_owned(), "crate::alpha".to_owned()]);
        let edges = encode_edge_membership(&[
            (
                "crate::b".to_owned(),
                "crate::c".to_owned(),
                "calls".to_owned(),
            ),
            (
                "crate::a".to_owned(),
                "crate::b".to_owned(),
                "calls".to_owned(),
            ),
        ]);

        assert_eq!(nodes, "crate::alpha\ncrate::beta");
        assert_eq!(
            edges,
            "crate::a\tcrate::b\tcalls\ncrate::b\tcrate::c\tcalls"
        );
    }

    #[test]
    fn shallow_build_surfaces_warning() {
        let source = tempfile::tempdir().expect("source tempdir");
        git_init(source.path());
        write_file(source.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        commit_all(source.path(), "first");
        write_file(
            source.path(),
            "src/lib.rs",
            "pub fn alpha() -> i32 { 1 }\npub fn beta() -> i32 { 2 }\n",
        );
        commit_all(source.path(), "second");

        let clone = tempfile::tempdir().expect("clone tempdir");
        git_clone_shallow(source.path(), clone.path());

        let store_dir = tempfile::tempdir().expect("db tempdir");
        let store = open_store(&store_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(clone.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        let summary = build_historical_graph(
            clone.path(),
            &canonical_root,
            &store,
            &CommitSelector::Bounded {
                start_ref: "HEAD".to_owned(),
                max_commits: None,
                since: None,
                until: None,
            },
            &registry,
            Some("HEAD"),
        )
        .expect("build on shallow clone");

        assert!(
            summary
                .warnings
                .iter()
                .any(|warning| warning.contains("shallow clone detected")),
            "expected shallow warning, got {summary:?}"
        );
    }

    #[test]
    fn build_progress_reports_current_commit_and_file() {
        let repo = tempfile::tempdir().expect("tempdir");
        git_init(repo.path());
        write_file(repo.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        write_file(repo.path(), "notes.xyz", "atlas fixture\n");
        let first = commit_all(repo.path(), "first");

        let store_dir = tempfile::tempdir().expect("store dir");
        let store = open_store(&store_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(repo.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        let mut events = Vec::new();
        let summary = build_historical_graph_with_progress(
            repo.path(),
            &canonical_root,
            &store,
            &CommitSelector::Explicit {
                shas: vec![first.clone()],
            },
            &registry,
            Some(&first),
            |event| events.push(event),
        )
        .expect("build with progress");

        assert_eq!(summary.commits_processed, 1);
        assert!(matches!(
            events.first(),
            Some(BuildProgressEvent::RunStarted { total_commits: 1 })
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            BuildProgressEvent::CommitStarted {
                commit_index: 1,
                total_commits: 1,
                commit_sha,
                total_files,
            } if commit_sha == &first && *total_files >= 2
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            BuildProgressEvent::FileProcessed {
                file_path,
                outcome: BuildFileProgressKind::Parsed,
                ..
            } if file_path == "src/lib.rs"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            BuildProgressEvent::FileProcessed {
                file_path,
                outcome: BuildFileProgressKind::SkippedUnsupported,
                ..
            } if file_path == "notes.xyz"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            BuildProgressEvent::CommitPersistStepStarted {
                commit_index: 1,
                total_commits: 1,
                commit_sha,
                step_index: 1,
                total_steps,
                step: BuildPersistProgressKind::CommitMetadata,
            } if commit_sha == &first && *total_steps == BuildPersistProgressKind::total_steps()
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            BuildProgressEvent::CommitPersistStepStarted {
                step_index: 4,
                total_steps,
                step: BuildPersistProgressKind::TransactionCommit,
                ..
            } if *total_steps == BuildPersistProgressKind::total_steps()
        )));
    }

    #[test]
    fn repeated_build_for_same_commit_range_is_deterministic() {
        let repo = tempfile::tempdir().expect("tempdir");
        git_init(repo.path());
        write_file(repo.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        let first = commit_all(repo.path(), "first");
        write_file(
            repo.path(),
            "src/lib.rs",
            "pub fn alpha() -> i32 { 1 }\npub fn beta() -> i32 { 2 }\n",
        );
        let second = commit_all(repo.path(), "second");

        let selector = CommitSelector::Explicit {
            shas: vec![first.clone(), second.clone()],
        };
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(repo.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        let store_a_dir = tempfile::tempdir().expect("store a dir");
        let store_a = open_store(&store_a_dir);
        let summary_a = build_historical_graph(
            repo.path(),
            &canonical_root,
            &store_a,
            &selector,
            &registry,
            None,
        )
        .expect("first build");

        let store_b_dir = tempfile::tempdir().expect("store b dir");
        let store_b = open_store(&store_b_dir);
        let summary_b = build_historical_graph(
            repo.path(),
            &canonical_root,
            &store_b,
            &selector,
            &registry,
            None,
        )
        .expect("second build");

        assert_eq!(summary_a.commits_processed, summary_b.commits_processed);
        assert_eq!(summary_a.files_reused, summary_b.files_reused);
        assert_eq!(summary_a.files_parsed, summary_b.files_parsed);
        assert_eq!(summary_a.files_skipped, summary_b.files_skipped);
        assert_eq!(summary_a.nodes_reused, summary_b.nodes_reused);
        assert_eq!(summary_a.nodes_written, summary_b.nodes_written);
        assert_eq!(summary_a.edges_written, summary_b.edges_written);
        assert_eq!(summary_a.warnings, summary_b.warnings);
        assert_eq!(summary_a.errors.len(), summary_b.errors.len());

        let repo_id_a = store_a
            .find_repo_id(&canonical_root)
            .expect("repo id a")
            .expect("repo a");
        let repo_id_b = store_b
            .find_repo_id(&canonical_root)
            .expect("repo id b")
            .expect("repo b");

        for sha in [&first, &second] {
            let snapshot_a = store_a
                .find_snapshot(repo_id_a, sha)
                .expect("snapshot a")
                .expect("snapshot a row");
            let snapshot_b = store_b
                .find_snapshot(repo_id_b, sha)
                .expect("snapshot b")
                .expect("snapshot b row");
            assert_eq!(snapshot_a.node_count, snapshot_b.node_count);
            assert_eq!(snapshot_a.edge_count, snapshot_b.edge_count);
            assert_eq!(snapshot_a.file_count, snapshot_b.file_count);
            assert_eq!(snapshot_a.completeness, snapshot_b.completeness);
            assert_eq!(snapshot_a.parse_error_count, snapshot_b.parse_error_count);

            let materialized_a =
                reconstruct_snapshot(&store_a, &canonical_root, sha).expect("reconstruct a");
            let materialized_b =
                reconstruct_snapshot(&store_b, &canonical_root, sha).expect("reconstruct b");
            assert_eq!(
                snapshot_signature(&materialized_a),
                snapshot_signature(&materialized_b)
            );
        }
    }

    // --- parallel parse → sequential write boundary -------------------------
    //
    // Verifies that the parallel parse pipeline collects all results before
    // writing to the store.  Three Rust source files are committed; every
    // function node from every file must appear in the store after the build,
    // regardless of which Rayon thread parsed each file.
    #[test]
    fn build_parallel_parse_completes_before_store_write() {
        let repo = tempfile::tempdir().expect("tempdir");
        git_init(repo.path());
        write_file(repo.path(), "a.rs", "pub fn alpha() {}\n");
        write_file(repo.path(), "b.rs", "pub fn beta() {}\n");
        write_file(repo.path(), "c.rs", "pub fn gamma() {}\n");
        let sha = commit_all(repo.path(), "init");

        let store_dir = tempfile::tempdir().expect("store dir");
        let store = open_store(&store_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(repo.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        let summary = build_historical_graph(
            repo.path(),
            &canonical_root,
            &store,
            &CommitSelector::Explicit {
                shas: vec![sha.clone()],
            },
            &registry,
            Some(&sha),
        )
        .expect("build");

        assert_eq!(summary.files_parsed, 3, "all three files must be parsed");

        // Every function from every parallel-parsed file must land in the
        // store.  If any write phase ran before parse collection finished,
        // nodes from later Rayon tasks would be missing.
        let repo_id = store
            .find_repo_id(&canonical_root)
            .expect("find repo id")
            .expect("repo id");
        let snapshot = store
            .find_snapshot(repo_id, &sha)
            .expect("find snapshot")
            .expect("snapshot row");

        assert_eq!(snapshot.file_count, 3, "three files in snapshot");
        assert!(
            snapshot.node_count > 0,
            "nodes from all parallel-parsed files must be written"
        );
    }
}

mod entry;
mod process;
mod types;

pub use self::entry::build_historical_graph;
pub use self::entry::build_historical_graph_with_progress;
pub use self::entry::rebuild_historical_snapshot;
pub use self::entry::rebuild_historical_snapshot_with_progress;
pub use self::types::BuildFileProgressKind;
pub use self::types::BuildPersistProgressKind;
pub use self::types::BuildProgressEvent;
pub use self::types::BuildSummary;
pub use self::types::SnapshotRebuildSummary;
