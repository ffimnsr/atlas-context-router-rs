use super::process::emit_file_progress;
use super::process::emit_persist_progress;
use super::process::full_snapshot_files;
use super::process::load_blob_bytes;
use super::process::load_cached_file_graph;
use super::process::persist_parsed_file;
use super::process::prefetch_blob_bytes;
use super::process::replay_snapshot_files;
use super::types::*;
use super::*;

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
    progress(BuildProgressEvent::RunPhaseChanged {
        message: "recomputing lifecycle history".to_owned(),
    });
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

    // Batch-fetch all blob bytes that will be needed for this commit in a
    // single `git cat-file --batch` call rather than one subprocess per file.
    prefetch_blob_bytes(ctx.cache, ctx.repo, &tracked_files, ctx.registry)?;

    // Begin a single write transaction covering all store inserts for this
    // commit.  Without it every INSERT auto-commits (one fsync each), which
    // turns ~100k writes per large commit into minutes of wall-clock time.
    ctx.store
        .begin_write()
        .context("begin commit write transaction")?;

    let mut snapshot_files: Vec<StoredSnapshotFile> = Vec::new();
    let mut file_memberships: Vec<SnapshotFileMembership> = Vec::new();
    let mut total_node_count: i64 = 0;
    let mut total_edge_count: i64 = 0;
    let mut parse_error_count: i64 = 0;

    // --- Phase 1: sequential classification (touches store and cache) --------
    //
    // Reused and skipped files are handled immediately.  Files that need a
    // fresh parse are collected into `parse_batch` so phase 2 can run them
    // in parallel without touching the SQLite store.
    let mut parse_batch: Vec<ParseWork> = Vec::new();

    for (file_offset, tracked_file) in tracked_files.into_iter().enumerate() {
        let file_index = file_offset + 1;
        let rel_path = tracked_file.file_path.clone();
        let file_hash = tracked_file.file_hash.clone();

        // Try to detect language early via parser support.
        if !ctx.registry.supports(&rel_path) {
            ctx.summary.files_skipped += 1;
            emit_file_progress(
                ctx,
                meta,
                total_files,
                file_index,
                &rel_path,
                BuildFileProgressKind::SkippedUnsupported,
            );
            debug!(path = %rel_path, "no parser support, skipping");
            continue;
        }

        let already_indexed = ctx
            .store
            .has_historical_file_graph(&file_hash)
            .with_context(|| format!("has_historical_file_graph for {file_hash}"))?;

        if already_indexed {
            let cached_graph = load_cached_file_graph(ctx.store, ctx.cache, &file_hash)?;
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
                &rel_path,
                BuildFileProgressKind::Reused,
            );
            continue;
        }

        let Some(bytes) = load_blob_bytes(ctx.cache, ctx.repo, &meta.sha, &rel_path, &file_hash)?
        else {
            ctx.summary.files_skipped += 1;
            emit_file_progress(
                ctx,
                meta,
                total_files,
                file_index,
                &rel_path,
                BuildFileProgressKind::SkippedMissing,
            );
            continue;
        };

        // Binary detection: null byte in first BINARY_SNIFF_BYTES.
        if ctx.cache.binary_blobs.contains(&file_hash) || is_binary_bytes(&bytes) {
            ctx.cache.binary_blobs.insert(file_hash.clone());
            ctx.summary.files_skipped += 1;
            emit_file_progress(
                ctx,
                meta,
                total_files,
                file_index,
                &rel_path,
                BuildFileProgressKind::SkippedBinary,
            );
            debug!(path = %rel_path, "binary file, skipping");
            continue;
        }

        parse_batch.push(ParseWork {
            file_index,
            file_hash,
            rel_path,
            bytes,
            orig_language: tracked_file.language,
        });
    }

    // --- Phase 2: parallel parse (CPU-bound, no store access) ---------------
    //
    // SQLite writes stay outside this Rayon closure.  Workers only parse bytes;
    // `Store` writes happen sequentially in phase 3 after collection.
    let registry = ctx.registry;
    let parse_outcomes: Vec<ParseOutcome> = parse_batch
        .into_par_iter()
        .map(|work| {
            let size = work.bytes.len() as i64;
            let parsed = registry
                .parse(&work.rel_path, &work.file_hash, &work.bytes, None)
                .map(|(pf, _)| pf);
            ParseOutcome {
                file_index: work.file_index,
                file_hash: work.file_hash,
                rel_path: work.rel_path,
                orig_language: work.orig_language,
                size,
                parsed,
            }
        })
        .collect();

    // --- Phase 3: sequential persist + progress emission --------------------
    for outcome in parse_outcomes {
        let file_index = outcome.file_index;
        let rel_path = &outcome.rel_path;
        let file_hash = &outcome.file_hash;

        let parsed = match outcome.parsed {
            Some(pf) => pf,
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
        let size = outcome.size;
        let language = parsed.language.clone().or(outcome.orig_language);

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
    emit_persist_progress(ctx, meta, BuildPersistProgressKind::CommitMetadata);
    ctx.store
        .upsert_commit(&stored_commit)
        .context("upsert commit")
        .inspect_err(|_| ctx.store.rollback_write())?;

    // Insert graph snapshot.
    emit_persist_progress(ctx, meta, BuildPersistProgressKind::SnapshotRows);
    let snapshot_id = ctx
        .store
        .insert_snapshot(
            ctx.repo_id,
            &meta.sha,
            root_tree_hash.as_deref(),
            total_node_count,
            total_edge_count,
            file_count,
            completeness,
            parse_error_count,
        )
        .inspect_err(|_| ctx.store.rollback_write())?;

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
        .context("insert snapshot files")
        .inspect_err(|_| ctx.store.rollback_write())?;

    // Attach node and edge membership.
    emit_persist_progress(ctx, meta, BuildPersistProgressKind::SnapshotMembership);
    for membership in &file_memberships {
        ctx.store
            .attach_snapshot_nodes(
                snapshot_id,
                &membership.file_hash,
                &membership.qualified_names,
            )
            .with_context(|| format!("attach snapshot nodes for {}", membership.file_hash))
            .inspect_err(|_| ctx.store.rollback_write())?;
    }
    for membership in &file_memberships {
        ctx.store
            .attach_snapshot_edges(snapshot_id, &membership.file_hash, &membership.edge_keys)
            .with_context(|| format!("attach snapshot edges for {}", membership.file_hash))
            .inspect_err(|_| ctx.store.rollback_write())?;
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
        .context("insert snapshot membership blobs")
        .inspect_err(|_| ctx.store.rollback_write())?;

    emit_persist_progress(ctx, meta, BuildPersistProgressKind::TransactionCommit);
    ctx.store
        .commit_write()
        .context("commit write transaction")?;

    Ok(())
}
