//! Incremental update pipeline: detect changes, parse changed + dependent files,
//! persist updated graph slices to SQLite.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use atlas_core::model::{ChangeType, ParsedFile};
use atlas_parser::ParserRegistry;
use atlas_repo::{DiffTarget, changed_files, hash_file, repo_relative};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use rayon::prelude::*;

use crate::call_resolution::reconcile_call_targets;

/// Specifies which set of changes to process.
pub enum UpdateTarget {
    /// Unstaged working-tree changes.
    WorkingTree,
    /// Changes staged for commit.
    Staged,
    /// Changes relative to the given git ref (e.g. `"origin/main"`).
    BaseRef(String),
    /// Explicit set of repo-relative file paths.
    Files(Vec<String>),
}

/// Options controlling the incremental update pipeline.
pub struct UpdateOptions {
    /// Abort on the first parse or I/O failure instead of continuing.
    pub fail_fast: bool,
    /// Number of files parsed per parallel batch.
    pub batch_size: usize,
    /// Which changes to process.
    pub target: UpdateTarget,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        Self {
            fail_fast: false,
            batch_size: crate::config::DEFAULT_PARSE_BATCH_SIZE,
            target: UpdateTarget::WorkingTree,
        }
    }
}

/// Summary returned by `update_graph`.
#[derive(Debug, Default)]
pub struct UpdateSummary {
    pub deleted: usize,
    pub renamed: usize,
    pub parsed: usize,
    pub skipped_unsupported: usize,
    pub parse_errors: usize,
    pub nodes_updated: usize,
    pub edges_updated: usize,
    pub elapsed_ms: u128,
}

/// Compute a content-signature string for `node` (excludes line positions).
fn node_signature(n: &atlas_core::Node) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        n.kind.as_str(),
        n.params.as_deref().unwrap_or(""),
        n.return_type.as_deref().unwrap_or(""),
        n.modifiers.as_deref().unwrap_or(""),
        n.is_test as u8,
    )
}

/// Collect QNs of symbols whose interface signature changed or were added/removed.
fn changed_qnames(old_sigs: &HashMap<String, String>, nodes: &[atlas_core::Node]) -> Vec<String> {
    let mut changed: Vec<String> = Vec::new();
    for n in nodes {
        let new_sig = node_signature(n);
        match old_sigs.get(&n.qualified_name) {
            Some(old_sig) if old_sig == &new_sig => {}
            _ => changed.push(n.qualified_name.clone()),
        }
    }
    let new_qns: HashSet<&str> = nodes.iter().map(|n| n.qualified_name.as_str()).collect();
    for qn in old_sigs.keys() {
        if !new_qns.contains(qn.as_str()) {
            changed.push(qn.clone());
        }
    }
    changed
}

fn supported_candidates(
    repo_root: &Utf8Path,
    registry: &ParserRegistry,
    paths: &[String],
) -> (Vec<(String, camino::Utf8PathBuf)>, usize) {
    let mut skipped = 0usize;
    let candidates = paths
        .iter()
        .filter_map(|rel_str| {
            if !registry.supports(rel_str) {
                skipped += 1;
                return None;
            }
            Some((rel_str.clone(), repo_root.join(rel_str)))
        })
        .collect();
    (candidates, skipped)
}

/// Process incremental graph update for `repo_root`, writing to `db_path`.
pub fn update_graph(
    repo_root: &Utf8Path,
    db_path: &str,
    opts: &UpdateOptions,
) -> Result<UpdateSummary> {
    let started = Instant::now();

    let mut store =
        Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    // ── Determine which files changed ────────────────────────────────────────
    let _diff_span = tracing::info_span!("update.detect_changes").entered();

    let git_changes: Vec<atlas_core::model::ChangedFile> = match &opts.target {
        UpdateTarget::Files(paths) => paths
            .iter()
            .map(|p| {
                let abs = Utf8Path::new(p);
                let rel = if abs.is_absolute() {
                    repo_relative(repo_root, abs).unwrap_or_else(|_| abs.to_owned())
                } else {
                    abs.to_owned()
                };
                atlas_core::model::ChangedFile {
                    path: rel.to_string(),
                    change_type: ChangeType::Modified,
                    old_path: None,
                }
            })
            .collect(),
        other => {
            let diff_target = match other {
                UpdateTarget::Staged => DiffTarget::Staged,
                UpdateTarget::BaseRef(r) => DiffTarget::BaseRef(r.clone()),
                _ => DiffTarget::WorkingTree,
            };
            changed_files(repo_root, &diff_target).context("cannot detect changed files")?
        }
    };

    drop(_diff_span);

    let mut to_delete: Vec<String> = Vec::new();
    let mut to_parse_paths: Vec<String> = Vec::new();
    let mut to_rename: Vec<(String, String)> = Vec::new(); // (old_path, new_path)

    for cf in &git_changes {
        match cf.change_type {
            ChangeType::Deleted => to_delete.push(cf.path.clone()),
            ChangeType::Renamed => {
                if let Some(old) = &cf.old_path {
                    // Check whether content is unchanged: if so, preserve node ids.
                    let new_abs = repo_root.join(&cf.path);
                    let new_hash = atlas_repo::hash_file(&new_abs).ok();
                    let stored_hash = store.file_hash(old).ok().flatten();
                    if let (Some(nh), Some(sh)) = (&new_hash, &stored_hash)
                        && nh == sh
                    {
                        to_rename.push((old.clone(), cf.path.clone()));
                        continue;
                    }
                    to_delete.push(old.clone());
                }
                to_parse_paths.push(cf.path.clone());
            }
            ChangeType::Copied => {
                if let Some(old) = &cf.old_path {
                    to_delete.push(old.clone());
                }
                to_parse_paths.push(cf.path.clone());
            }
            _ => to_parse_paths.push(cf.path.clone()),
        }
    }

    // ── Stable renames (hash-unchanged) ─────────────────────────────────────
    let renamed_count = to_rename.len();
    if !to_rename.is_empty() {
        let _rename_span = tracing::info_span!("update.rename_stable").entered();
        for (old, new) in &to_rename {
            store
                .rename_file_graph(old, new)
                .with_context(|| format!("cannot rename graph '{old}' -> '{new}'"))?;
        }
    }

    // ── Load old signatures before deleting ──────────────────────────────────
    let _sig_span = tracing::info_span!("update.load_signatures").entered();
    let old_sigs: HashMap<String, HashMap<String, String>> = to_parse_paths
        .iter()
        .filter_map(|p| {
            store
                .node_signatures_by_file(p)
                .ok()
                .map(|sigs| (p.clone(), sigs))
        })
        .collect();
    drop(_sig_span);

    // ── Delete stale graphs ───────────────────────────────────────────────────
    let deleted_count = to_delete.len();
    {
        let _del_span = tracing::info_span!("update.delete_stale").entered();
        for path in &to_delete {
            store
                .delete_file_graph(path)
                .with_context(|| format!("cannot delete graph for '{path}'"))?;
        }
    }

    let registry = ParserRegistry::with_defaults();
    let mut parse_errors = 0usize;
    let mut skipped_unsupported = 0usize;

    // ── Phase 1: parse directly-changed files ────────────────────────────────
    let (changed_candidates, changed_unsupported) =
        supported_candidates(repo_root, &registry, &to_parse_paths);
    skipped_unsupported += changed_unsupported;

    let _parse_span = tracing::info_span!("update.parse_changed").entered();
    let mut parsed_changed: Vec<ParsedFile> = Vec::new();

    for chunk in changed_candidates.chunks(opts.batch_size) {
        let results: Vec<(String, Result<ParsedFile, String>)> = chunk
            .par_iter()
            .map(|(rel_str, abs_path)| {
                let hash = match hash_file(abs_path) {
                    Ok(h) => h,
                    Err(e) => return (rel_str.clone(), Err(format!("hash error: {e}"))),
                };
                let source = match fs::read(abs_path.as_std_path()) {
                    Ok(b) => b,
                    Err(e) => return (rel_str.clone(), Err(format!("read error: {e}"))),
                };
                match registry.parse(rel_str, &hash, &source) {
                    Some(pf) => (rel_str.clone(), Ok(pf)),
                    None => (rel_str.clone(), Err("unsupported (skipped)".into())),
                }
            })
            .collect();

        for (rel_str, outcome) in results {
            match outcome {
                Ok(pf) => parsed_changed.push(pf),
                Err(msg) if msg == "unsupported (skipped)" => skipped_unsupported += 1,
                Err(msg) => {
                    tracing::warn!("processing '{}' failed: {msg}", rel_str);
                    parse_errors += 1;
                    if opts.fail_fast {
                        return Err(anyhow::anyhow!(
                            "processing '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }
        }
    }
    drop(_parse_span);

    // ── Dependency invalidation ───────────────────────────────────────────────
    let _deps_span = tracing::info_span!("update.find_dependents").entered();
    let mut all_changed_qnames: Vec<String> = Vec::new();
    for pf in &parsed_changed {
        let empty = HashMap::new();
        let old = old_sigs.get(&pf.path).unwrap_or(&empty);
        all_changed_qnames.extend(changed_qnames(old, &pf.nodes));
    }

    let changed_qn_refs: Vec<&str> = all_changed_qnames.iter().map(String::as_str).collect();
    let dependents = store
        .find_dependents_for_qnames(&changed_qn_refs)
        .context("cannot query dependents")?;
    drop(_deps_span);

    let changed_paths_set: HashSet<&str> = to_parse_paths.iter().map(String::as_str).collect();
    let dependent_paths: Vec<String> = dependents
        .iter()
        .filter(|d| !changed_paths_set.contains(d.as_str()))
        .cloned()
        .collect();

    let (dep_candidates, dep_unsupported) =
        supported_candidates(repo_root, &registry, &dependent_paths);
    skipped_unsupported += dep_unsupported;

    // ── Phase 2: parse dependent files ───────────────────────────────────────
    let _dep_parse_span = tracing::info_span!("update.parse_dependents").entered();
    let mut parsed_deps: Vec<ParsedFile> = Vec::new();

    for chunk in dep_candidates.chunks(opts.batch_size) {
        let results: Vec<(String, Result<ParsedFile, String>)> = chunk
            .par_iter()
            .map(|(rel_str, abs_path)| {
                let hash = match hash_file(abs_path) {
                    Ok(h) => h,
                    Err(e) => return (rel_str.clone(), Err(format!("hash error: {e}"))),
                };
                let source = match fs::read(abs_path.as_std_path()) {
                    Ok(b) => b,
                    Err(e) => return (rel_str.clone(), Err(format!("read error: {e}"))),
                };
                match registry.parse(rel_str, &hash, &source) {
                    Some(pf) => (rel_str.clone(), Ok(pf)),
                    None => (rel_str.clone(), Err("unsupported (skipped)".into())),
                }
            })
            .collect();

        for (rel_str, outcome) in results {
            match outcome {
                Ok(pf) => parsed_deps.push(pf),
                Err(msg) if msg == "unsupported (skipped)" => skipped_unsupported += 1,
                Err(msg) => {
                    tracing::warn!("processing dependent '{}' failed: {msg}", rel_str);
                    parse_errors += 1;
                    if opts.fail_fast {
                        return Err(anyhow::anyhow!(
                            "processing dependent '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }
        }
    }
    drop(_dep_parse_span);

    // ── Write all parsed files ────────────────────────────────────────────────
    let _write_span = tracing::info_span!("update.write").entered();
    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;

    let all_parsed: Vec<&ParsedFile> = parsed_changed.iter().chain(parsed_deps.iter()).collect();
    for chunk in all_parsed.chunks(opts.batch_size) {
        let chunk_owned: Vec<ParsedFile> = chunk.iter().map(|pf| (*pf).clone()).collect();
        let (n, e) = store
            .replace_files_transactional(&chunk_owned)
            .context("cannot store parsed files")?;
        total_nodes += n;
        total_edges += e;

        // Refresh chunk text; delete stale entries first, then re-insert.
        for pf in &chunk_owned {
            if let Err(err) = store.delete_chunks_for_file(&pf.path) {
                tracing::warn!("chunk delete failed for {}: {err:#}", pf.path);
            }
            for node in &pf.nodes {
                if let Err(err) = store.upsert_chunk(&node.qualified_name, 0, &node.chunk_text()) {
                    tracing::warn!("chunk upsert failed for {}: {err:#}", node.qualified_name);
                }
            }
        }
    }
    drop(_write_span);

    let resolved_paths: Vec<String> = all_parsed.iter().map(|pf| pf.path.clone()).collect();
    if !resolved_paths.is_empty()
        && let Err(err) = reconcile_call_targets(&mut store, repo_root, &resolved_paths)
    {
        tracing::warn!("late call-target resolution failed during update: {err:#}");
    }

    let parsed_count = parsed_changed.len() + parsed_deps.len();

    Ok(UpdateSummary {
        deleted: deleted_count,
        renamed: renamed_count,
        parsed: parsed_count,
        skipped_unsupported,
        parse_errors,
        nodes_updated: total_nodes,
        edges_updated: total_edges,
        elapsed_ms: started.elapsed().as_millis(),
    })
}
