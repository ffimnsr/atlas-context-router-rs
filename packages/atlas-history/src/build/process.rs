use super::types::*;
use super::*;

pub(super) fn emit_file_progress<P>(
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

pub(super) fn emit_persist_progress<P>(
    ctx: &mut BuildContext<'_, P>,
    meta: &git::GitCommitMeta,
    step: BuildPersistProgressKind,
) where
    P: FnMut(BuildProgressEvent),
{
    (ctx.progress)(BuildProgressEvent::CommitPersistStepStarted {
        commit_index: ctx.commit_index,
        total_commits: ctx.total_commits,
        commit_sha: meta.sha.clone(),
        step_index: step.step_index(),
        total_steps: BuildPersistProgressKind::total_steps(),
        step,
    });
}

pub(super) fn load_blob_bytes(
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

/// Prefetch blob bytes for all supported, not-yet-cached files in a single
/// `git cat-file --batch` call.  Populates `cache.blob_bytes` so the
/// per-file loop in `process_commit` hits the in-memory cache and skips the
/// individual `git show` subprocess spawns.
pub(super) fn prefetch_blob_bytes(
    cache: &mut BuildRunCache,
    repo: &Path,
    tracked_files: &[StoredSnapshotFile],
    registry: &ParserRegistry,
) -> Result<()> {
    // Collect deduplicated hashes that need content:
    // - parser supports the file extension
    // - not already in the blob_bytes or blob_graphs in-run cache
    // Already-indexed files are detected per-file in the main loop via
    // `has_historical_file_graph`; we intentionally over-fetch here so that
    // the prefetch does not duplicate those per-file store queries.
    let mut to_fetch: Vec<&str> = tracked_files
        .iter()
        .filter(|f| registry.supports(&f.file_path))
        .filter(|f| {
            !cache.blob_bytes.contains_key(&f.file_hash)
                && !cache.blob_graphs.contains_key(&f.file_hash)
                && !cache.binary_blobs.contains(&f.file_hash)
        })
        .map(|f| f.file_hash.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    if to_fetch.is_empty() {
        return Ok(());
    }

    // Sort for deterministic ordering (BTreeSet already sorted, but make
    // explicit for readability).
    to_fetch.sort_unstable();

    let fetched = git::cat_file_blob_batch(repo, &to_fetch).context("batch-fetch blob bytes")?;
    for (hash, bytes) in fetched {
        cache.blob_bytes.insert(hash, bytes);
    }
    Ok(())
}

pub(super) fn load_cached_file_graph(
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

pub(super) fn full_snapshot_files(
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

pub(super) fn replay_snapshot_files(
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

pub(super) fn persist_parsed_file(
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
