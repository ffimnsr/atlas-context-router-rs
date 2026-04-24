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

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use atlas_core::ParsedFile;
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::{HistoricalEdge, HistoricalNode, Store, StoredSnapshotFile};
use tracing::{debug, warn};

use crate::git;
use crate::ingest::IngestError;
use crate::select::CommitSelector;

/// Number of bytes inspected for binary detection.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// Summary produced by [`build_historical_graph`].
#[derive(Debug, Default)]
pub struct BuildSummary {
    /// Total commits enumerated from the selector.
    pub commits_processed: usize,
    /// Commits skipped because a snapshot already existed.
    pub commits_already_indexed: usize,
    /// Files whose graph was loaded from existing storage (no parse needed).
    pub files_reused: usize,
    /// Files newly parsed during this build run.
    pub files_parsed: usize,
    /// Nodes loaded from already-indexed blobs (reuse count).
    pub nodes_reused: usize,
    /// Nodes written from newly parsed blobs.
    pub nodes_written: usize,
    /// Edges written from newly parsed blobs.
    pub edges_written: usize,
    /// Files skipped as binary or unsupported extension.
    pub files_skipped: usize,
    /// Per-commit or per-file errors that did not abort the run.
    pub errors: Vec<IngestError>,
    /// Wall-clock duration for the entire build.
    pub elapsed_secs: f64,
}

/// Build historical graph snapshots for every commit selected by `selector`.
///
/// For each commit:
/// 1. Enumerate tracked files via `git ls-tree`.
/// 2. For each blob: reuse existing parsed graph when `file_hash` is already
///    indexed; otherwise fetch bytes, detect binary, parse, and persist.
/// 3. Write snapshot metadata + file/node/edge membership.
///
/// Errors on individual commits or files are collected and returned in
/// `BuildSummary::errors`; the run continues.
pub fn build_historical_graph(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    selector: &CommitSelector,
    registry: &ParserRegistry,
) -> Result<BuildSummary> {
    let started = Instant::now();
    let mut summary = BuildSummary::default();

    let commits = selector.resolve(repo).context("resolve commit selector")?;

    if commits.is_empty() {
        summary.elapsed_secs = started.elapsed().as_secs_f64();
        return Ok(summary);
    }

    let repo_id = store
        .upsert_repo(canonical_root)
        .context("upsert repo row")?;

    for meta in &commits {
        summary.commits_processed += 1;

        // Skip already-indexed commits.
        if store.find_snapshot(repo_id, &meta.sha)?.is_some() {
            summary.commits_already_indexed += 1;
            debug!(sha = %meta.sha, "snapshot already indexed, skipping");
            continue;
        }

        if let Err(e) = process_commit(repo, repo_id, store, registry, meta, &mut summary) {
            warn!("error processing commit {}: {e:#}", meta.sha);
            summary.errors.push(IngestError {
                commit_sha: Some(meta.sha.clone()),
                message: format!("{e:#}"),
            });
        }
    }

    summary.elapsed_secs = started.elapsed().as_secs_f64();
    Ok(summary)
}

fn process_commit(
    repo: &Path,
    repo_id: i64,
    store: &Store,
    registry: &ParserRegistry,
    meta: &git::GitCommitMeta,
    summary: &mut BuildSummary,
) -> Result<()> {
    // Resolve the root tree hash for this commit.
    let tree_ref = format!("{}^{{tree}}", meta.sha);
    let root_tree_hash = git::rev_parse(repo, &tree_ref).ok();

    // List all blobs tracked at this commit.
    let entries =
        git::ls_tree(repo, &meta.sha).with_context(|| format!("ls-tree for {}", meta.sha))?;

    let mut snapshot_files: Vec<StoredSnapshotFile> = Vec::new();
    type EdgeKey = (String, String, String);
    let mut file_node_qns: Vec<(String, Vec<String>)> = Vec::new(); // (file_hash, [qn])
    #[allow(clippy::type_complexity)]
    let mut file_edge_keys: Vec<(String, Vec<EdgeKey>)> = Vec::new(); // (file_hash, [(src, tgt, kind)])
    let mut total_node_count: i64 = 0;
    let mut total_edge_count: i64 = 0;
    let mut parse_error_count: i64 = 0;

    for entry in &entries {
        if entry.object_type != "blob" {
            continue;
        }

        // Canonicalize the repo-relative file path.
        let rel_path = &entry.file_path;
        let file_hash = &entry.object_hash;

        // Try to detect language early via parser support.
        if !registry.supports(rel_path) {
            summary.files_skipped += 1;
            debug!(path = %rel_path, "no parser support, skipping");
            continue;
        }

        let already_indexed = store
            .has_historical_file_graph(file_hash)
            .with_context(|| format!("has_historical_file_graph for {file_hash}"))?;

        if already_indexed {
            // Count existing nodes/edges for summary.
            let n = store.count_historical_nodes(file_hash)?;
            let e = store.count_historical_edges(file_hash)?;
            summary.files_reused += 1;
            summary.nodes_reused += n as usize;
            total_node_count += n;
            total_edge_count += e;

            // Determine language from existing node rows (best effort).
            let lang: Option<String> = store.get_historical_file_language(file_hash)?;

            snapshot_files.push(StoredSnapshotFile {
                snapshot_id: 0, // filled after snapshot insert
                file_path: rel_path.clone(),
                file_hash: file_hash.clone(),
                language: lang,
                size: None,
            });

            // Collect QNs for membership attachment.
            let qns = store.list_historical_node_qns(file_hash)?;
            let edges = store.list_historical_edge_keys(file_hash)?;
            file_node_qns.push((file_hash.clone(), qns));
            file_edge_keys.push((file_hash.clone(), edges));
            continue;
        }

        // Fetch file bytes from git object store.
        let bytes = match git::show_file(repo, &meta.sha, rel_path)
            .with_context(|| format!("git show {}:{}", meta.sha, rel_path))?
        {
            Some(b) => b,
            None => {
                // File was deleted or not present; skip gracefully.
                summary.files_skipped += 1;
                continue;
            }
        };

        // Binary detection: null byte in first BINARY_SNIFF_BYTES.
        if is_binary_bytes(&bytes) {
            summary.files_skipped += 1;
            debug!(path = %rel_path, "binary file, skipping");
            continue;
        }

        // Parse the file.
        let parsed = match registry.parse(rel_path, file_hash, &bytes, None) {
            Some((pf, _tree)) => pf,
            None => {
                summary.files_skipped += 1;
                continue;
            }
        };

        let node_count = parsed.nodes.len();
        let edge_count = parsed.edges.len();
        let size = bytes.len() as i64;
        let language = parsed.language.clone();

        // Persist content-addressed nodes + edges.
        if let Err(e) = persist_parsed_file(store, file_hash, rel_path, &parsed) {
            warn!("parse persist error for {rel_path} at {}: {e:#}", meta.sha);
            parse_error_count += 1;
            summary.errors.push(IngestError {
                commit_sha: Some(meta.sha.clone()),
                message: format!("persist {rel_path}: {e:#}"),
            });
            continue;
        }

        summary.files_parsed += 1;
        summary.nodes_written += node_count;
        summary.edges_written += edge_count;
        total_node_count += node_count as i64;
        total_edge_count += edge_count as i64;

        snapshot_files.push(StoredSnapshotFile {
            snapshot_id: 0,
            file_path: rel_path.clone(),
            file_hash: file_hash.clone(),
            language,
            size: Some(size),
        });

        let qns: Vec<String> = parsed
            .nodes
            .iter()
            .map(|n| n.qualified_name.clone())
            .collect();
        let edge_keys: Vec<(String, String, String)> = parsed
            .edges
            .iter()
            .map(|e| {
                (
                    e.source_qn.clone(),
                    e.target_qn.clone(),
                    e.kind.as_str().to_owned(),
                )
            })
            .collect();
        file_node_qns.push((file_hash.clone(), qns));
        file_edge_keys.push((file_hash.clone(), edge_keys));
    }

    let file_count = snapshot_files.len() as i64;
    let completeness = if parse_error_count == 0 || file_count == 0 {
        1.0
    } else {
        1.0 - (parse_error_count as f64 / file_count as f64)
    };

    // Insert commit metadata (idempotent).
    let stored_commit = atlas_store_sqlite::StoredCommit {
        commit_sha: meta.sha.clone(),
        repo_id,
        parent_sha: meta.parent_sha.clone(),
        author_name: Some(meta.author_name.clone()),
        author_email: Some(meta.author_email.clone()),
        author_time: meta.author_time,
        committer_time: meta.committer_time,
        subject: meta.subject.clone(),
        message: if meta.body.is_empty() {
            None
        } else {
            Some(format!("{}\n\n{}", meta.subject, meta.body))
        },
        indexed_at: String::new(),
    };
    store
        .upsert_commit(&stored_commit)
        .context("upsert commit")?;

    // Insert graph snapshot.
    let snapshot_id = store.insert_snapshot(
        repo_id,
        &meta.sha,
        root_tree_hash.as_deref(),
        total_node_count,
        total_edge_count,
        file_count,
        completeness,
        parse_error_count,
    )?;

    // Fix snapshot_id in file rows and insert membership.
    let fixed_files: Vec<StoredSnapshotFile> = snapshot_files
        .into_iter()
        .map(|mut f| {
            f.snapshot_id = snapshot_id;
            f
        })
        .collect();
    store
        .insert_snapshot_files(&fixed_files)
        .context("insert snapshot files")?;

    // Attach node and edge membership.
    for (file_hash, qns) in &file_node_qns {
        store
            .attach_snapshot_nodes(snapshot_id, file_hash, qns)
            .with_context(|| format!("attach snapshot nodes for {file_hash}"))?;
    }
    for (file_hash, edges) in &file_edge_keys {
        store
            .attach_snapshot_edges(snapshot_id, file_hash, edges)
            .with_context(|| format!("attach snapshot edges for {file_hash}"))?;
    }

    Ok(())
}

fn persist_parsed_file(
    store: &Store,
    file_hash: &str,
    file_path: &str,
    parsed: &ParsedFile,
) -> Result<()> {
    let nodes: Vec<HistoricalNode> = parsed
        .nodes
        .iter()
        .map(|n| HistoricalNode {
            file_hash: file_hash.to_owned(),
            qualified_name: n.qualified_name.clone(),
            kind: n.kind.as_str().to_owned(),
            name: n.name.clone(),
            file_path: file_path.to_owned(),
            line_start: Some(n.line_start as i64),
            line_end: Some(n.line_end as i64),
            language: Some(n.language.clone()),
            parent_name: n.parent_name.clone(),
            params: n.params.clone(),
            return_type: n.return_type.clone(),
            modifiers: n.modifiers.clone(),
            is_test: n.is_test,
            extra_json: Some(n.extra_json.to_string()),
        })
        .collect();

    let edges: Vec<HistoricalEdge> = parsed
        .edges
        .iter()
        .map(|e| HistoricalEdge {
            file_hash: file_hash.to_owned(),
            source_qn: e.source_qn.clone(),
            target_qn: e.target_qn.clone(),
            kind: e.kind.as_str().to_owned(),
            file_path: file_path.to_owned(),
            line: e.line.map(|l| l as i64),
            confidence: e.confidence as f64,
            confidence_tier: e.confidence_tier.clone(),
            extra_json: Some(e.extra_json.to_string()),
        })
        .collect();

    store.insert_historical_nodes(&nodes)?;
    store.insert_historical_edges(&edges)?;
    Ok(())
}

/// Detect binary content by scanning the first [`BINARY_SNIFF_BYTES`] for
/// null bytes — the same heuristic used by `atlas-repo` for live indexing.
fn is_binary_bytes(bytes: &[u8]) -> bool {
    let sniff = &bytes[..bytes.len().min(BINARY_SNIFF_BYTES)];
    sniff.contains(&0u8)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
