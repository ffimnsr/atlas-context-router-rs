use super::*;

pub fn reconstruct_snapshot(
    store: &Store,
    canonical_root: &str,
    commit_sha: &str,
) -> Result<HistoricalSnapshot> {
    let repo_id = store
        .find_repo_id(canonical_root)?
        .ok_or_else(|| anyhow::anyhow!("repo not yet registered for history: {canonical_root}"))?;
    let snapshot = store
        .find_snapshot(repo_id, commit_sha)?
        .ok_or_else(|| anyhow::anyhow!("indexed snapshot not found for commit {commit_sha}"))?;
    reconstruct_snapshot_from_row(store, snapshot)
}

pub fn diff_snapshots(
    repo: &Path,
    store: &Store,
    canonical_root: &str,
    commit_a: &str,
    commit_b: &str,
) -> Result<GraphDiffReport> {
    let snapshot_a = reconstruct_snapshot(store, canonical_root, commit_a)?;
    let snapshot_b = reconstruct_snapshot(store, canonical_root, commit_b)?;

    let files_a = snapshot_a
        .files
        .iter()
        .map(|file| (file.file_path.clone(), file.clone()))
        .collect::<BTreeMap<_, _>>();
    let files_b = snapshot_b
        .files
        .iter()
        .map(|file| (file.file_path.clone(), file.clone()))
        .collect::<BTreeMap<_, _>>();

    let added_files = files_b
        .iter()
        .filter(|(path, _)| !files_a.contains_key(*path))
        .map(|(_, file)| file.clone())
        .collect::<Vec<_>>();
    let removed_files = files_a
        .iter()
        .filter(|(path, _)| !files_b.contains_key(*path))
        .map(|(_, file)| file.clone())
        .collect::<Vec<_>>();
    let modified_files = files_a
        .iter()
        .filter_map(|(path, before)| {
            let after = files_b.get(path)?;
            if before.file_hash == after.file_hash
                && before.language == after.language
                && before.size == after.size
            {
                return None;
            }
            Some(FileChange {
                file_path: path.clone(),
                old_hash: Some(before.file_hash.clone()),
                new_hash: Some(after.file_hash.clone()),
                old_language: before.language.clone(),
                new_language: after.language.clone(),
                old_size: before.size,
                new_size: after.size,
            })
        })
        .collect::<Vec<_>>();

    let renamed_files = git::diff_tree_files(repo, commit_b, Some(commit_a))
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, _, status)| *status == 'R')
        .map(|(old_path, new_path, _)| FileRename { old_path, new_path })
        .collect::<Vec<_>>();

    let nodes_a = snapshot_a
        .nodes
        .iter()
        .map(|node| {
            (
                NodeKey {
                    qualified_name: node.qualified_name.clone(),
                    file_path: node.file_path.clone(),
                    kind: node.kind.clone(),
                },
                node,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let nodes_b = snapshot_b
        .nodes
        .iter()
        .map(|node| {
            (
                NodeKey {
                    qualified_name: node.qualified_name.clone(),
                    file_path: node.file_path.clone(),
                    kind: node.kind.clone(),
                },
                node,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let added_nodes = nodes_b
        .iter()
        .filter(|(key, _)| !nodes_a.contains_key(*key))
        .map(|(_, node)| node_added(node))
        .collect::<Vec<_>>();
    let removed_nodes = nodes_a
        .iter()
        .filter(|(key, _)| !nodes_b.contains_key(*key))
        .map(|(_, node)| node_removed(node))
        .collect::<Vec<_>>();
    let changed_nodes = nodes_a
        .iter()
        .filter_map(|(key, before)| {
            let after = nodes_b.get(key)?;
            let diff = node_changed(before, after);
            if diff.changed_fields.is_empty() {
                None
            } else {
                Some(diff)
            }
        })
        .collect::<Vec<_>>();

    let edges_a = snapshot_a
        .edges
        .iter()
        .map(|edge| {
            (
                EdgeKey {
                    source_qn: edge.source_qn.clone(),
                    target_qn: edge.target_qn.clone(),
                    kind: edge.kind.clone(),
                    file_path: edge.file_path.clone(),
                },
                edge,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let edges_b = snapshot_b
        .edges
        .iter()
        .map(|edge| {
            (
                EdgeKey {
                    source_qn: edge.source_qn.clone(),
                    target_qn: edge.target_qn.clone(),
                    kind: edge.kind.clone(),
                    file_path: edge.file_path.clone(),
                },
                edge,
            )
        })
        .collect::<BTreeMap<_, _>>();

    let added_edges = edges_b
        .iter()
        .filter(|(key, _)| !edges_a.contains_key(*key))
        .map(|(_, edge)| edge_added(edge))
        .collect::<Vec<_>>();
    let removed_edges = edges_a
        .iter()
        .filter(|(key, _)| !edges_b.contains_key(*key))
        .map(|(_, edge)| edge_removed(edge))
        .collect::<Vec<_>>();
    let changed_edges = edges_a
        .iter()
        .filter_map(|(key, before)| {
            let after = edges_b.get(key)?;
            let diff = edge_changed(before, after);
            if diff.changed_fields.is_empty() {
                None
            } else {
                Some(diff)
            }
        })
        .collect::<Vec<_>>();

    let module_counts_a = module_counts(&snapshot_a);
    let module_counts_b = module_counts(&snapshot_b);
    let mut module_names = module_counts_a.keys().cloned().collect::<BTreeSet<_>>();
    module_names.extend(module_counts_b.keys().cloned());
    let module_changes = module_names
        .into_iter()
        .filter_map(|module| {
            let before = module_counts_a.get(&module).copied().unwrap_or((0, 0));
            let after = module_counts_b.get(&module).copied().unwrap_or((0, 0));
            if before == after {
                return None;
            }
            Some(ModuleDiffEntry {
                module,
                node_count_before: before.0,
                node_count_after: after.0,
                edge_count_before: before.1,
                edge_count_after: after.1,
            })
        })
        .collect::<Vec<_>>();

    let architecture = architecture_diff(&snapshot_a, &snapshot_b);
    let evidence = build_evidence(
        [
            snapshot_a.summary.snapshot_id,
            snapshot_b.summary.snapshot_id,
        ],
        [commit_a.to_owned(), commit_b.to_owned()],
        snapshot_a
            .nodes
            .iter()
            .map(node_identifier_for)
            .chain(snapshot_b.nodes.iter().map(node_identifier_for)),
        snapshot_a
            .edges
            .iter()
            .map(edge_identifier_for)
            .chain(snapshot_b.edges.iter().map(edge_identifier_for)),
        snapshot_a
            .files
            .iter()
            .map(|file| file.file_path.clone())
            .chain(snapshot_b.files.iter().map(|file| file.file_path.clone())),
    );

    Ok(GraphDiffReport {
        summary: GraphDiffSummary {
            added_file_count: added_files.len(),
            removed_file_count: removed_files.len(),
            modified_file_count: modified_files.len(),
            renamed_file_count: renamed_files.len(),
            added_node_count: added_nodes.len(),
            removed_node_count: removed_nodes.len(),
            changed_node_count: changed_nodes.len(),
            added_edge_count: added_edges.len(),
            removed_edge_count: removed_edges.len(),
            changed_edge_count: changed_edges.len(),
            module_change_count: module_changes.len(),
            new_cycle_count: architecture.new_cycles.len(),
            broken_cycle_count: architecture.broken_cycles.len(),
        },
        evidence,
        commit_a: commit_a.to_owned(),
        commit_b: commit_b.to_owned(),
        snapshot_a,
        snapshot_b,
        added_files,
        removed_files,
        modified_files,
        renamed_files,
        added_nodes,
        removed_nodes,
        changed_nodes,
        added_edges,
        removed_edges,
        changed_edges,
        module_changes,
        architecture,
    })
}

fn reconstruct_snapshot_from_row(
    store: &Store,
    snapshot: StoredSnapshot,
) -> Result<HistoricalSnapshot> {
    let files = store
        .list_snapshot_files(snapshot.snapshot_id)
        .context("list snapshot files")?;
    let nodes = store
        .reconstruct_snapshot_nodes(snapshot.snapshot_id)
        .context("reconstruct snapshot nodes")?;
    let edges = store
        .reconstruct_snapshot_edges(snapshot.snapshot_id)
        .context("reconstruct snapshot edges")?;
    validate_snapshot_materialization(&snapshot, &files, &nodes, &edges)?;
    let evidence = build_evidence(
        [snapshot.snapshot_id],
        [snapshot.commit_sha.clone()],
        nodes.iter().map(node_identifier_for),
        edges.iter().map(edge_identifier_for),
        files.iter().map(|file| file.file_path.clone()),
    );
    Ok(HistoricalSnapshot {
        summary: HistoricalSnapshotSummary {
            snapshot_id: snapshot.snapshot_id,
            commit_sha: snapshot.commit_sha.clone(),
            node_count: snapshot.node_count,
            edge_count: snapshot.edge_count,
            file_count: snapshot.file_count,
            completeness: snapshot.completeness,
            parse_error_count: snapshot.parse_error_count,
        },
        evidence,
        repo_id: snapshot.repo_id,
        commit_sha: snapshot.commit_sha,
        root_tree_hash: snapshot.root_tree_hash,
        node_count: snapshot.node_count,
        edge_count: snapshot.edge_count,
        file_count: snapshot.file_count,
        completeness: snapshot.completeness,
        parse_error_count: snapshot.parse_error_count,
        files,
        nodes,
        edges,
    })
}

pub(crate) fn validate_snapshot_materialization(
    snapshot: &StoredSnapshot,
    files: &[StoredSnapshotFile],
    nodes: &[HistoricalNode],
    edges: &[HistoricalEdge],
) -> Result<()> {
    let mut mismatches = Vec::new();

    if snapshot.file_count != files.len() as i64 {
        mismatches.push(format!(
            "files expected={} actual={}",
            snapshot.file_count,
            files.len()
        ));
    }
    if snapshot.node_count != nodes.len() as i64 {
        mismatches.push(format!(
            "nodes expected={} actual={}",
            snapshot.node_count,
            nodes.len()
        ));
    }
    if snapshot.edge_count != edges.len() as i64 {
        mismatches.push(format!(
            "edges expected={} actual={}",
            snapshot.edge_count,
            edges.len()
        ));
    }

    if mismatches.is_empty() {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "snapshot membership rows appear corrupted for commit {} (snapshot_id={}): {}",
        snapshot.commit_sha,
        snapshot.snapshot_id,
        mismatches.join(", ")
    )
    .into())
}
