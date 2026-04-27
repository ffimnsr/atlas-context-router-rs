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

#[derive(Debug, Clone)]
struct CachedFileGraph {
    language: Option<String>,
    size: Option<i64>,
    node_count: usize,
    edge_count: usize,
    qualified_names: Vec<String>,
    edge_keys: Vec<EdgeKey>,
}

#[derive(Debug, Clone, Default)]
struct BuildRunCache {
    blob_bytes: BTreeMap<String, Vec<u8>>,
    blob_graphs: BTreeMap<String, CachedFileGraph>,
    binary_blobs: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct SnapshotFileMembership {
    file_path: String,
    file_hash: String,
    qualified_names: Vec<String>,
    edge_keys: Vec<EdgeKey>,
}

struct BuildContext<'a, P: FnMut(BuildProgressEvent)> {
    repo: &'a Path,
    repo_id: i64,
    store: &'a Store,
    registry: &'a ParserRegistry,
    indexed_ref: Option<&'a str>,
    cache: &'a mut BuildRunCache,
    summary: &'a mut BuildSummary,
    progress: &'a mut P,
    commit_index: usize,
    total_commits: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BuildFileProgressKind {
    Reused,
    Parsed,
    SkippedUnsupported,
    SkippedBinary,
    SkippedMissing,
    SkippedNoParserOutput,
    PersistError,
}

impl fmt::Display for BuildFileProgressKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Reused => "reused",
            Self::Parsed => "parsed",
            Self::SkippedUnsupported => "skipped unsupported",
            Self::SkippedBinary => "skipped binary",
            Self::SkippedMissing => "skipped missing",
            Self::SkippedNoParserOutput => "skipped empty-parse",
            Self::PersistError => "persist error",
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BuildProgressEvent {
    RunStarted {
        total_commits: usize,
    },
    CommitStarted {
        commit_index: usize,
        total_commits: usize,
        commit_sha: String,
        total_files: usize,
    },
    CommitSkipped {
        commit_index: usize,
        total_commits: usize,
        commit_sha: String,
    },
    FileProcessed {
        commit_index: usize,
        total_commits: usize,
        commit_sha: String,
        file_index: usize,
        total_files: usize,
        file_path: String,
        outcome: BuildFileProgressKind,
    },
}

/// Summary produced by [`build_historical_graph`].
#[derive(Debug, Clone, Default, Serialize)]
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
    /// Non-fatal diagnostics the caller should surface to the user.
    pub warnings: Vec<String>,
    /// Wall-clock duration for the entire build.
    pub elapsed_secs: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SnapshotRebuildSummary {
    pub commit_sha: String,
    pub replaced_snapshot_id: i64,
    pub rebuilt_snapshot_id: i64,
    pub reclaimed_file_hashes: u64,
    pub reclaimed_historical_nodes: u64,
    pub reclaimed_historical_edges: u64,
    pub build: BuildSummary,
    pub lifecycle: LifecycleSummary,
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
    indexed_ref: Option<&str>,
) -> Result<BuildSummary> {
    build_historical_graph_with_progress(
        repo,
        canonical_root,
        store,
        selector,
        registry,
        indexed_ref,
        |_| {},
    )
}

pub fn build_historical_graph_with_progress<P>(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    selector: &CommitSelector,
    registry: &ParserRegistry,
    indexed_ref: Option<&str>,
    mut progress: P,
) -> Result<BuildSummary>
where
    P: FnMut(BuildProgressEvent),
{
    let started = Instant::now();
    let mut summary = BuildSummary::default();
    let mut cache = BuildRunCache::default();

    if let Some(warning) = shallow_build_warning(repo, selector) {
        summary.warnings.push(warning);
    }

    let mut commits = selector.resolve(repo).context("resolve commit selector")?;
    if selector.prefers_oldest_first() && commits.len() > 1 {
        commits.reverse();
    }

    if commits.is_empty() {
        summary.elapsed_secs = started.elapsed().as_secs_f64();
        return Ok(summary);
    }

    let total_commits = commits.len();
    progress(BuildProgressEvent::RunStarted { total_commits });

    let repo_id = store
        .upsert_repo(canonical_root)
        .context("upsert repo row")?;

    for (index, meta) in commits.iter().enumerate() {
        let commit_index = index + 1;
        summary.commits_processed += 1;

        // Skip already-indexed commits.
        if store.find_snapshot(repo_id, &meta.sha)?.is_some() {
            summary.commits_already_indexed += 1;
            progress(BuildProgressEvent::CommitSkipped {
                commit_index,
                total_commits,
                commit_sha: meta.sha.clone(),
            });
            debug!(sha = %meta.sha, "snapshot already indexed, skipping");
            continue;
        }

        let selector_indexed_ref = selector.source_ref_label();
        let mut ctx = BuildContext {
            repo,
            repo_id,
            store,
            registry,
            indexed_ref: indexed_ref.or(selector_indexed_ref.as_deref()),
            cache: &mut cache,
            summary: &mut summary,
            progress: &mut progress,
            commit_index,
            total_commits,
        };
        if let Err(e) = process_commit(&mut ctx, meta) {
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

pub fn rebuild_historical_snapshot(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    commit_sha: &str,
    registry: &ParserRegistry,
    indexed_ref: Option<&str>,
) -> Result<SnapshotRebuildSummary> {
    rebuild_historical_snapshot_with_progress(
        repo,
        canonical_root,
        store,
        commit_sha,
        registry,
        indexed_ref,
        |_| {},
    )
}

pub fn rebuild_historical_snapshot_with_progress<P>(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    commit_sha: &str,
    registry: &ParserRegistry,
    indexed_ref: Option<&str>,
    mut progress: P,
) -> Result<SnapshotRebuildSummary>
where
    P: FnMut(BuildProgressEvent),
{
    let repo_id = store.find_repo_id(canonical_root)?.ok_or_else(|| {
        anyhow::anyhow!("history not initialized; run `atlas history build` first")
    })?;
    let existing = store
        .find_snapshot(repo_id, commit_sha)?
        .ok_or_else(|| anyhow::anyhow!("snapshot not indexed for commit {commit_sha}"))?;

    let mut commits = git::log_commits_explicit(repo, &[commit_sha.to_owned()])
        .with_context(|| format!("resolve commit {commit_sha} for rebuild"))?;
    let meta = commits
        .pop()
        .ok_or_else(|| anyhow::anyhow!("commit metadata missing for {commit_sha}"))?;

    store.delete_history_snapshots(&[existing.snapshot_id])?;
    let (reclaimed_file_hashes, reclaimed_historical_nodes, reclaimed_historical_edges) =
        store.prune_orphan_historical_file_graphs()?;

    let mut build = BuildSummary {
        commits_processed: 1,
        ..BuildSummary::default()
    };
    let mut cache = BuildRunCache::default();
    progress(BuildProgressEvent::RunStarted { total_commits: 1 });
    let mut ctx = BuildContext {
        repo,
        repo_id,
        store,
        registry,
        indexed_ref,
        cache: &mut cache,
        summary: &mut build,
        progress: &mut progress,
        commit_index: 1,
        total_commits: 1,
    };
    process_commit(&mut ctx, &meta).with_context(|| format!("rebuild snapshot {commit_sha}"))?;

    let rebuilt_snapshot_id = store
        .find_snapshot(repo_id, commit_sha)?
        .map(|snapshot| snapshot.snapshot_id)
        .ok_or_else(|| anyhow::anyhow!("rebuilt snapshot missing for {commit_sha}"))?;
    let lifecycle = recompute_lifecycle(canonical_root, store).context("recompute lifecycle")?;

    Ok(SnapshotRebuildSummary {
        commit_sha: commit_sha.to_owned(),
        replaced_snapshot_id: existing.snapshot_id,
        rebuilt_snapshot_id,
        reclaimed_file_hashes,
        reclaimed_historical_nodes,
        reclaimed_historical_edges,
        build,
        lifecycle,
    })
}

fn shallow_build_warning(repo: &Path, selector: &CommitSelector) -> Option<String> {
    match git::is_shallow(repo) {
        Ok(true) => match selector {
            CommitSelector::Explicit { .. } => None,
            _ => Some(
                "shallow clone detected; build may omit older commits beyond fetched history. Fetch more history or use --commits with reachable SHAs.".to_owned(),
            ),
        },
        Ok(false) => None,
        Err(_) => None,
    }
}

fn process_commit<P>(ctx: &mut BuildContext<'_, P>, meta: &git::GitCommitMeta) -> Result<()>
where
    P: FnMut(BuildProgressEvent),
{
    // Resolve the root tree hash for this commit.
    let tree_ref = format!("{}^{{tree}}", meta.sha);
    let root_tree_hash = git::rev_parse(ctx.repo, &tree_ref).ok();

    let tracked_files = replay_snapshot_files(ctx.repo, ctx.repo_id, ctx.store, meta, ctx.summary)?
        .unwrap_or(full_snapshot_files(ctx.repo, &meta.sha, ctx.summary)?);
    let total_files = tracked_files.len();
    (ctx.progress)(BuildProgressEvent::CommitStarted {
        commit_index: ctx.commit_index,
        total_commits: ctx.total_commits,
        commit_sha: meta.sha.clone(),
        total_files,
    });

    let mut snapshot_files: Vec<StoredSnapshotFile> = Vec::new();
    let mut file_memberships: Vec<SnapshotFileMembership> = Vec::new();
    let mut total_node_count: i64 = 0;
    let mut total_edge_count: i64 = 0;
    let mut parse_error_count: i64 = 0;

    for (file_offset, tracked_file) in tracked_files.into_iter().enumerate() {
        let file_index = file_offset + 1;
        let rel_path = &tracked_file.file_path;
        let file_hash = &tracked_file.file_hash;

        // Try to detect language early via parser support.
        if !ctx.registry.supports(rel_path) {
            ctx.summary.files_skipped += 1;
            emit_file_progress(
                ctx,
                meta,
                total_files,
                file_index,
                rel_path,
                BuildFileProgressKind::SkippedUnsupported,
            );
            debug!(path = %rel_path, "no parser support, skipping");
            continue;
        }

        let already_indexed = ctx
            .store
            .has_historical_file_graph(file_hash)
            .with_context(|| format!("has_historical_file_graph for {file_hash}"))?;

        if already_indexed {
            let cached_graph = load_cached_file_graph(ctx.store, ctx.cache, file_hash)?;
            ctx.summary.files_reused += 1;
            ctx.summary.nodes_reused += cached_graph.node_count;
            total_node_count += cached_graph.node_count as i64;
            total_edge_count += cached_graph.edge_count as i64;

            snapshot_files.push(StoredSnapshotFile {
                snapshot_id: 0, // filled after snapshot insert
                file_path: rel_path.clone(),
                file_hash: file_hash.clone(),
                language: tracked_file
                    .language
                    .or_else(|| cached_graph.language.clone()),
                size: tracked_file.size.or(cached_graph.size),
            });
            file_memberships.push(SnapshotFileMembership {
                file_path: rel_path.clone(),
                file_hash: file_hash.clone(),
                qualified_names: cached_graph.qualified_names.clone(),
                edge_keys: cached_graph.edge_keys.clone(),
            });
            emit_file_progress(
                ctx,
                meta,
                total_files,
                file_index,
                rel_path,
                BuildFileProgressKind::Reused,
            );
            continue;
        }

        let Some(bytes) = load_blob_bytes(ctx.cache, ctx.repo, &meta.sha, rel_path, file_hash)?
        else {
            ctx.summary.files_skipped += 1;
            emit_file_progress(
                ctx,
                meta,
                total_files,
                file_index,
                rel_path,
                BuildFileProgressKind::SkippedMissing,
            );
            continue;
        };

        // Binary detection: null byte in first BINARY_SNIFF_BYTES.
        if ctx.cache.binary_blobs.contains(file_hash) || is_binary_bytes(&bytes) {
            ctx.cache.binary_blobs.insert(file_hash.clone());
            ctx.summary.files_skipped += 1;
            emit_file_progress(
                ctx,
                meta,
                total_files,
                file_index,
                rel_path,
                BuildFileProgressKind::SkippedBinary,
            );
            debug!(path = %rel_path, "binary file, skipping");
            continue;
        }

        // Parse the file.
        let parsed = match ctx.registry.parse(rel_path, file_hash, &bytes, None) {
            Some((pf, _tree)) => pf,
            None => {
                ctx.summary.files_skipped += 1;
                emit_file_progress(
                    ctx,
                    meta,
                    total_files,
                    file_index,
                    rel_path,
                    BuildFileProgressKind::SkippedNoParserOutput,
                );
                continue;
            }
        };

        let node_count = parsed.nodes.len();
        let edge_count = parsed.edges.len();
        let size = bytes.len() as i64;
        let language = parsed.language.clone();

        // Persist content-addressed nodes + edges.
        if let Err(e) = persist_parsed_file(ctx.store, file_hash, rel_path, &parsed) {
            warn!("parse persist error for {rel_path} at {}: {e:#}", meta.sha);
            parse_error_count += 1;
            ctx.summary.errors.push(IngestError {
                commit_sha: Some(meta.sha.clone()),
                message: format!("persist {rel_path}: {e:#}"),
            });
            emit_file_progress(
                ctx,
                meta,
                total_files,
                file_index,
                rel_path,
                BuildFileProgressKind::PersistError,
            );
            continue;
        }

        ctx.summary.files_parsed += 1;
        ctx.summary.nodes_written += node_count;
        ctx.summary.edges_written += edge_count;
        total_node_count += node_count as i64;
        total_edge_count += edge_count as i64;

        snapshot_files.push(StoredSnapshotFile {
            snapshot_id: 0,
            file_path: rel_path.clone(),
            file_hash: file_hash.clone(),
            language: language.clone(),
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
        ctx.cache.blob_graphs.insert(
            file_hash.clone(),
            CachedFileGraph {
                language,
                size: Some(size),
                node_count,
                edge_count,
                qualified_names: qns.clone(),
                edge_keys: edge_keys.clone(),
            },
        );
        file_memberships.push(SnapshotFileMembership {
            file_path: rel_path.clone(),
            file_hash: file_hash.clone(),
            qualified_names: qns,
            edge_keys,
        });
        emit_file_progress(
            ctx,
            meta,
            total_files,
            file_index,
            rel_path,
            BuildFileProgressKind::Parsed,
        );
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
        repo_id: ctx.repo_id,
        parent_sha: meta.parent_sha.clone(),
        indexed_ref: ctx.indexed_ref.map(str::to_owned),
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
    ctx.store
        .upsert_commit(&stored_commit)
        .context("upsert commit")?;

    // Insert graph snapshot.
    let snapshot_id = ctx.store.insert_snapshot(
        ctx.repo_id,
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
    ctx.store
        .insert_snapshot_files(&fixed_files)
        .context("insert snapshot files")?;

    // Attach node and edge membership.
    for membership in &file_memberships {
        ctx.store
            .attach_snapshot_nodes(
                snapshot_id,
                &membership.file_hash,
                &membership.qualified_names,
            )
            .with_context(|| format!("attach snapshot nodes for {}", membership.file_hash))?;
    }
    for membership in &file_memberships {
        ctx.store
            .attach_snapshot_edges(snapshot_id, &membership.file_hash, &membership.edge_keys)
            .with_context(|| format!("attach snapshot edges for {}", membership.file_hash))?;
    }

    let membership_blobs = file_memberships
        .into_iter()
        .map(|membership| StoredSnapshotMembershipBlob {
            snapshot_id,
            file_path: membership.file_path,
            file_hash: membership.file_hash,
            node_membership: encode_node_membership(&membership.qualified_names),
            edge_membership: encode_edge_membership(&membership.edge_keys),
        })
        .collect::<Vec<_>>();
    ctx.store
        .insert_snapshot_membership_blobs(&membership_blobs)
        .context("insert snapshot membership blobs")?;

    Ok(())
}

fn emit_file_progress<P>(
    ctx: &mut BuildContext<'_, P>,
    meta: &git::GitCommitMeta,
    total_files: usize,
    file_index: usize,
    file_path: &str,
    outcome: BuildFileProgressKind,
) where
    P: FnMut(BuildProgressEvent),
{
    (ctx.progress)(BuildProgressEvent::FileProcessed {
        commit_index: ctx.commit_index,
        total_commits: ctx.total_commits,
        commit_sha: meta.sha.clone(),
        file_index,
        total_files,
        file_path: file_path.to_owned(),
        outcome,
    });
}

fn load_blob_bytes(
    cache: &mut BuildRunCache,
    repo: &Path,
    commit_sha: &str,
    rel_path: &str,
    file_hash: &str,
) -> Result<Option<Vec<u8>>> {
    if let Some(bytes) = cache.blob_bytes.get(file_hash) {
        return Ok(Some(bytes.clone()));
    }

    let Some(bytes) = git::show_file(repo, commit_sha, rel_path)
        .with_context(|| format!("git show {commit_sha}:{rel_path}"))?
    else {
        return Ok(None);
    };
    cache.blob_bytes.insert(file_hash.to_owned(), bytes.clone());
    Ok(Some(bytes))
}

fn load_cached_file_graph(
    store: &Store,
    cache: &mut BuildRunCache,
    file_hash: &str,
) -> Result<CachedFileGraph> {
    if let Some(graph) = cache.blob_graphs.get(file_hash) {
        return Ok(graph.clone());
    }

    let graph = CachedFileGraph {
        language: store.get_historical_file_language(file_hash)?,
        size: None,
        node_count: store.count_historical_nodes(file_hash)? as usize,
        edge_count: store.count_historical_edges(file_hash)? as usize,
        qualified_names: store.list_historical_node_qns(file_hash)?,
        edge_keys: store.list_historical_edge_keys(file_hash)?,
    };
    cache
        .blob_graphs
        .insert(file_hash.to_owned(), graph.clone());
    Ok(graph)
}

fn full_snapshot_files(
    repo: &Path,
    commit_sha: &str,
    summary: &mut BuildSummary,
) -> Result<Vec<StoredSnapshotFile>> {
    let entries =
        git::ls_tree(repo, commit_sha).with_context(|| format!("ls-tree for {commit_sha}"))?;
    let mut files = Vec::new();
    for entry in entries {
        if entry.object_type == "blob" {
            files.push(StoredSnapshotFile {
                snapshot_id: 0,
                file_path: entry.file_path,
                file_hash: entry.object_hash,
                language: None,
                size: None,
            });
            continue;
        }
        if entry.object_type == "commit" {
            push_warning(
                summary,
                format!(
                    "commit {commit_sha} contains submodule {} which historical indexing skips",
                    entry.file_path
                ),
            );
        }
    }
    Ok(files)
}

fn replay_snapshot_files(
    repo: &Path,
    repo_id: i64,
    store: &Store,
    meta: &git::GitCommitMeta,
    summary: &mut BuildSummary,
) -> Result<Option<Vec<StoredSnapshotFile>>> {
    let Some(parent_sha) = meta.parent_sha.as_deref() else {
        return Ok(None);
    };
    let Some(parent_snapshot) = store.find_snapshot(repo_id, parent_sha)? else {
        return Ok(None);
    };

    let mut files = store
        .list_snapshot_files(parent_snapshot.snapshot_id)?
        .into_iter()
        .map(|file| (file.file_path.clone(), file))
        .collect::<BTreeMap<_, _>>();
    let changes = git::diff_tree_files(repo, &meta.sha, Some(parent_sha))
        .with_context(|| format!("diff-tree for {parent_sha}..{}", meta.sha))?;
    if changes.is_empty() {
        return Ok(Some(sorted_snapshot_files(files)));
    }

    let changed_paths = changes
        .iter()
        .filter_map(|(_, new_path, status)| match status {
            'A' | 'M' | 'R' | 'C' => Some(new_path.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let entry_map = git::ls_tree_paths(repo, &meta.sha, &changed_paths)?
        .into_iter()
        .map(|entry| (entry.file_path.clone(), entry))
        .collect::<BTreeMap<_, _>>();

    for (old_path, new_path, status) in changes {
        match status {
            'D' => {
                files.remove(&old_path);
            }
            'R' => {
                files.remove(&old_path);
                apply_replayed_entry(&mut files, &entry_map, &new_path, &meta.sha, summary);
            }
            'C' => {
                apply_replayed_entry(&mut files, &entry_map, &new_path, &meta.sha, summary);
            }
            _ => {
                apply_replayed_entry(&mut files, &entry_map, &new_path, &meta.sha, summary);
            }
        }
    }

    Ok(Some(sorted_snapshot_files(files)))
}

fn apply_replayed_entry(
    files: &mut BTreeMap<String, StoredSnapshotFile>,
    entry_map: &BTreeMap<String, git::TreeEntry>,
    path: &str,
    commit_sha: &str,
    summary: &mut BuildSummary,
) {
    let Some(entry) = entry_map.get(path) else {
        files.remove(path);
        return;
    };
    if entry.object_type != "blob" {
        files.remove(path);
        if entry.object_type == "commit" {
            push_warning(
                summary,
                format!(
                    "commit {commit_sha} contains submodule {path} which historical indexing skips"
                ),
            );
        }
        return;
    }

    let (language, size) = files
        .get(path)
        .map(|existing| {
            if existing.file_hash == entry.object_hash {
                (existing.language.clone(), existing.size)
            } else {
                (None, None)
            }
        })
        .unwrap_or((None, None));

    files.insert(
        path.to_owned(),
        StoredSnapshotFile {
            snapshot_id: 0,
            file_path: path.to_owned(),
            file_hash: entry.object_hash.clone(),
            language,
            size,
        },
    );
}

fn sorted_snapshot_files(files: BTreeMap<String, StoredSnapshotFile>) -> Vec<StoredSnapshotFile> {
    files.into_values().collect()
}

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
}
