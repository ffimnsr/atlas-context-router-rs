//! Incremental update pipeline: detect changes, parse changed + dependent files,
//! persist updated graph slices to SQLite.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use atlas_core::{
    BudgetReport, BuildUpdateBudgetCounters,
    model::{ChangeType, ParsedFile},
};
use atlas_parser::{ExternalParserConfig, ParserRegistry, TreeCache};
use atlas_repo::{
    CanonicalRepoPath, DiffTarget, changed_files, discover_package_owners, hash_file, head_ref,
    stable_repo_id,
};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use rayon::prelude::*;

use crate::annotate::{annotate_parsed_file_owner, annotate_parsed_file_repo};
use crate::build_budget::{BuildBudgetDecision, BuildBudgetTracker};
use crate::call_resolution::reconcile_call_targets;
use crate::config::BuildRunBudget;
use crate::owner_graph::refresh_owner_graphs;
use crate::update_files::{
    delete_existing_file_graphs, existing_file_graph_deletions, explicit_file_change,
};

type ParseWorkItem = (String, camino::Utf8PathBuf, Option<tree_sitter::Tree>);
type ParseOutcome = Result<(ParsedFile, Option<tree_sitter::Tree>), String>;
type ParseResultRow = (String, ParseOutcome);

fn canonicalize_batch_change(
    change: &atlas_core::model::ChangedFile,
) -> Result<atlas_core::model::ChangedFile> {
    let path = CanonicalRepoPath::from_repo_relative(&change.path)
        .with_context(|| format!("invalid batch update path '{}'", change.path))?;
    let old_path = change
        .old_path
        .as_deref()
        .map(|old| {
            CanonicalRepoPath::from_repo_relative(old)
                .with_context(|| format!("invalid batch update old_path '{old}'"))
                .map(|path| path.as_str().to_owned())
        })
        .transpose()?;
    Ok(atlas_core::model::ChangedFile {
        path: path.as_str().to_owned(),
        change_type: change.change_type,
        old_path,
    })
}

/// Specifies which set of changes to process.
#[derive(Debug, Clone)]
pub enum UpdateTarget {
    /// Default target: everything changed since the last indexed ref
    /// (committed, staged, and unstaged). Falls back to index-only diffing
    /// when the database has no recorded indexed ref.
    WorkingTree,
    /// Changes staged for commit.
    Staged,
    /// Changes relative to the given git ref (e.g. `"origin/main"`).
    BaseRef(String),
    /// Explicit set of repo-relative file paths (all treated as Modified).
    Files(Vec<String>),
    /// Pre-classified batch of changes (used by watch mode).
    Batch(Vec<atlas_core::model::ChangedFile>),
}

/// Options controlling the incremental update pipeline.
pub struct UpdateOptions {
    /// Abort on the first parse or I/O failure instead of continuing.
    pub fail_fast: bool,
    /// Parse and summarize update work without mutating graph state.
    pub dry_run: bool,
    /// Number of files parsed per parallel batch.
    pub batch_size: usize,
    /// Which changes to process.
    pub target: UpdateTarget,
    /// Centralized operational budget for update work.
    pub budget: BuildRunBudget,
    /// Stable multi-repo registry id for rows written by this update.
    pub source_repo_id: Option<String>,
    /// Prefix qualified names so equal paths/symbols from separate repos cannot collide.
    pub namespace_qualified_names: bool,
    /// External (config-driven) parsers registered before the scan.
    pub external_parsers: Vec<ExternalParserConfig>,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        Self {
            fail_fast: false,
            dry_run: false,
            batch_size: crate::config::DEFAULT_PARSE_BATCH_SIZE,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
            source_repo_id: None,
            namespace_qualified_names: false,
            external_parsers: Vec::new(),
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
    pub chunk_upsert_failures: usize,
    pub call_target_reconcile_failures: usize,
    pub nodes_updated: usize,
    pub edges_updated: usize,
    pub warnings: Vec<String>,
    pub budget_counters: BuildUpdateBudgetCounters,
    pub budget: BudgetReport,
    pub reachability: crate::reachability::ReachabilitySummary,
    pub elapsed_ms: u128,
}

impl UpdateSummary {
    pub fn is_degraded(&self) -> bool {
        self.budget.partial
            || self.chunk_upsert_failures > 0
            || self.call_target_reconcile_failures > 0
    }
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
    let mut budget = BuildBudgetTracker::new(opts.budget);

    let mut store =
        Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let source_repo_id = opts
        .source_repo_id
        .clone()
        .unwrap_or_else(|| stable_repo_id(repo_root));
    store
        .upsert_repo(repo_root.as_str())
        .context("cannot register repo root for build state and history")?;

    let expected_fingerprint = crate::parser_fingerprint::compute(&opts.external_parsers);
    match store
        .parser_fingerprint(&source_repo_id)
        .context("cannot read parser fingerprint")?
    {
        Some(stored) if stored == expected_fingerprint => {}
        Some(_) => anyhow::bail!(
            "parser configuration changed since the last build; run `atlas build` to rebuild the graph for '{repo_root}'"
        ),
        None => anyhow::bail!(
            "graph has no parser fingerprint; run `atlas build` before `atlas update` for '{repo_root}'"
        ),
    }

    let owners = discover_package_owners(repo_root).context("cannot discover package owners")?;

    // ── Determine which files changed ────────────────────────────────────────
    let _diff_span = tracing::info_span!("update.detect_changes").entered();

    let git_changes: Vec<atlas_core::model::ChangedFile> = match &opts.target {
        UpdateTarget::Files(paths) => paths
            .iter()
            .map(|p| explicit_file_change(&store, repo_root, &source_repo_id, p))
            .collect::<Result<Vec<_>>>()?,
        UpdateTarget::Batch(changes) => changes
            .iter()
            .map(canonicalize_batch_change)
            .collect::<Result<Vec<_>>>()?,
        other => {
            let diff_target = match other {
                UpdateTarget::Staged => DiffTarget::Staged,
                UpdateTarget::BaseRef(r) => DiffTarget::BaseRef(r.clone()),
                UpdateTarget::WorkingTree => {
                    // Diff against the ref this repo was last synced from so
                    // committed changes (e.g. refactors split into new module
                    // files) are picked up, not just index-unstaged edits.
                    match store.last_indexed_ref(repo_root.as_str())? {
                        Some(commit) if !commit.is_empty() => DiffTarget::BaseRef(commit),
                        _ => DiffTarget::WorkingTree,
                    }
                }
                _ => DiffTarget::WorkingTree,
            };
            changed_files(repo_root, &diff_target).context("cannot detect changed files")?
        }
    };
    budget.set_files_discovered(git_changes.len());

    drop(_diff_span);

    let mut to_delete: Vec<String> = Vec::new();
    let mut to_parse_paths: Vec<String> = Vec::new();
    let mut to_rename: Vec<(String, String)> = Vec::new(); // (old_path, new_path)

    for cf in &git_changes {
        if matches!(
            budget.maybe_stop_for_time(started),
            BuildBudgetDecision::Degraded
        ) {
            break;
        }

        match cf.change_type {
            ChangeType::Deleted => to_delete.push(cf.path.clone()),
            ChangeType::Renamed => {
                if let Some(old) = &cf.old_path {
                    // Check whether content is unchanged: if so, preserve node ids.
                    let new_abs = repo_root.join(&cf.path);
                    let new_hash = hash_file(&new_abs).ok();
                    let stored_hash = store
                        .file_hash_for_repo(&source_repo_id, old)
                        .ok()
                        .flatten();
                    let old_owner_id = store
                        .file_owner_id_for_repo(&source_repo_id, old)
                        .ok()
                        .flatten();
                    let new_owner_id = owners
                        .owner_for_path(&cf.path)
                        .map(|owner| owner.owner_id.clone());
                    if let (Some(nh), Some(sh)) = (&new_hash, &stored_hash)
                        && nh == sh
                        && old_owner_id == new_owner_id
                    {
                        to_rename.push((old.clone(), cf.path.clone()));
                        continue;
                    }
                    to_delete.push(old.clone());
                }
                to_parse_paths.push(cf.path.clone());
            }
            ChangeType::Copied => {
                to_parse_paths.push(cf.path.clone());
            }
            _ => to_parse_paths.push(cf.path.clone()),
        }
    }

    // ── Stable renames (hash-unchanged) ─────────────────────────────────────
    let renamed_count = to_rename.len();
    if !to_rename.is_empty() && !opts.dry_run {
        let _rename_span = tracing::info_span!("update.rename_stable").entered();
        for (old, new) in &to_rename {
            store
                .rename_file_graph_for_repo(&source_repo_id, old, new)
                .with_context(|| format!("cannot rename graph '{old}' -> '{new}'"))?;
            store
                .upsert_file_owner_for_repo(&source_repo_id, new, owners.owner_for_path(new))
                .with_context(|| format!("cannot update owner metadata for '{new}'"))?;
        }
    }

    // ── Load old signatures before deleting ──────────────────────────────────
    let _sig_span = tracing::info_span!("update.load_signatures").entered();
    let old_sigs: HashMap<String, HashMap<String, String>> = to_parse_paths
        .iter()
        .filter_map(|p| {
            store
                .node_signatures_by_file_for_repo(&source_repo_id, p)
                .ok()
                .map(|sigs| (p.clone(), sigs))
        })
        .collect();
    drop(_sig_span);

    // ── Delete stale graphs ───────────────────────────────────────────────────
    // Only slices that still exist in the graph count as deletions, so a
    // repeated explicit `--files` update of the same removed path is a no-op.
    let deleted_count = if opts.dry_run {
        existing_file_graph_deletions(&store, &source_repo_id, &to_delete)?.len()
    } else {
        let _del_span = tracing::info_span!("update.delete_stale").entered();
        delete_existing_file_graphs(&mut store, &source_repo_id, &to_delete)?
    };

    let mut registry = ParserRegistry::with_defaults();
    registry
        .register_externals(&opts.external_parsers)
        .context("cannot register external parsers")?;
    let mut parse_errors = 0usize;
    let mut skipped_unsupported = 0usize;
    // In-process tree cache: trees from this run are reused when a file is
    // re-parsed as a dependent later in the same update.
    let mut tree_cache = TreeCache::new();

    // Evict trees for files that were deleted so stale entries don't linger.
    for path in &to_delete {
        tree_cache.evict(path);
    }

    // ── Phase 1: parse directly-changed files ────────────────────────────────
    let (changed_candidates, changed_unsupported) =
        supported_candidates(repo_root, &registry, &to_parse_paths);
    skipped_unsupported += changed_unsupported;

    let _parse_span = tracing::info_span!("update.parse_changed").entered();
    let mut parsed_changed: Vec<ParsedFile> = Vec::new();
    let mut changed_budgeted: Vec<(String, camino::Utf8PathBuf)> = Vec::new();
    let mut budget_blocked = false;
    let mut attempted_files = 0usize;

    for (rel_str, abs_path) in changed_candidates {
        let file_bytes = match fs::metadata(abs_path.as_std_path()) {
            Ok(meta) => meta.len(),
            Err(e) => {
                tracing::warn!("stat '{}' failed: {e}", rel_str);
                parse_errors += 1;
                if opts.fail_fast {
                    return Err(anyhow::Error::from(e)
                        .context(format!("stat '{rel_str}' failed (--fail-fast)")));
                }
                continue;
            }
        };
        if file_bytes > opts.budget.max_file_bytes {
            budget.note_skipped_by_file_bytes(file_bytes);
            continue;
        }
        match budget.try_accept_file(file_bytes) {
            BuildBudgetDecision::Continue => changed_budgeted.push((rel_str, abs_path)),
            BuildBudgetDecision::Degraded => break,
            BuildBudgetDecision::Blocked => {
                unreachable!("update file acceptance does not hard-block")
            }
        }
    }

    for chunk in changed_budgeted.chunks(opts.batch_size) {
        if matches!(
            budget.maybe_stop_for_time(started),
            BuildBudgetDecision::Degraded
        ) {
            break;
        }

        // Move old trees out of the cache for each file in this chunk so
        // they can be owned by the parallel closure (Tree: Send, not Sync).
        let mut work: Vec<ParseWorkItem> = chunk
            .iter()
            .map(|(rel_str, abs_path)| {
                let old_tree = tree_cache.remove(rel_str);
                (rel_str.clone(), abs_path.clone(), old_tree)
            })
            .collect();

        // SQLite work stays outside this Rayon closure. Workers only hash,
        // read, and parse; store mutation resumes after collection.
        let results: Vec<ParseResultRow> = work
            .par_iter_mut()
            .map(|(rel_str, abs_path, old_tree)| {
                let hash = match hash_file(abs_path) {
                    Ok(h) => h,
                    Err(e) => return (rel_str.clone(), Err(format!("hash error: {e}"))),
                };
                let source = match fs::read(abs_path.as_std_path()) {
                    Ok(b) => b,
                    Err(e) => return (rel_str.clone(), Err(format!("read error: {e}"))),
                };
                match registry.parse_checked(rel_str, &hash, &source, old_tree.as_ref()) {
                    Ok(Some((mut pf, tree))) => {
                        annotate_parsed_file_owner(&mut pf, owners.owner_for_path(rel_str));
                        annotate_parsed_file_repo(
                            &mut pf,
                            &source_repo_id,
                            repo_root.as_str(),
                            opts.namespace_qualified_names,
                        );
                        (rel_str.clone(), Ok((pf, tree)))
                    }
                    Ok(None) => (rel_str.clone(), Err("unsupported (skipped)".into())),
                    Err(msg) => (rel_str.clone(), Err(msg)),
                }
            })
            .collect();

        for (rel_str, outcome) in results {
            attempted_files += 1;
            match outcome {
                Ok((pf, tree)) => {
                    if let Some(t) = tree {
                        tree_cache.insert(rel_str.clone(), t);
                    }
                    parsed_changed.push(pf);
                }
                Err(msg) if msg == "unsupported (skipped)" => skipped_unsupported += 1,
                Err(msg) => {
                    tracing::warn!("processing '{}' failed: {msg}", rel_str);
                    parse_errors += 1;
                    if matches!(
                        budget.note_parse_failure(attempted_files),
                        BuildBudgetDecision::Blocked
                    ) {
                        budget_blocked = true;
                    }
                    if opts.fail_fast {
                        return Err(anyhow::anyhow!(
                            "processing '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }

            if budget_blocked {
                break;
            }
        }

        if budget_blocked {
            break;
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
        .find_dependents_for_qnames_for_repo(&source_repo_id, &changed_qn_refs)
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
    let mut dep_budgeted: Vec<(String, camino::Utf8PathBuf)> = Vec::new();

    for (rel_str, abs_path) in dep_candidates {
        let file_bytes = match fs::metadata(abs_path.as_std_path()) {
            Ok(meta) => meta.len(),
            Err(e) => {
                tracing::warn!("stat dependent '{}' failed: {e}", rel_str);
                parse_errors += 1;
                if opts.fail_fast {
                    return Err(anyhow::Error::from(e)
                        .context(format!("stat dependent '{rel_str}' failed (--fail-fast)")));
                }
                continue;
            }
        };
        if file_bytes > opts.budget.max_file_bytes {
            budget.note_skipped_by_file_bytes(file_bytes);
            continue;
        }
        match budget.try_accept_file(file_bytes) {
            BuildBudgetDecision::Continue => dep_budgeted.push((rel_str, abs_path)),
            BuildBudgetDecision::Degraded => break,
            BuildBudgetDecision::Blocked => {
                unreachable!("update file acceptance does not hard-block")
            }
        }
    }

    for chunk in dep_budgeted.chunks(opts.batch_size) {
        if matches!(
            budget.maybe_stop_for_time(started),
            BuildBudgetDecision::Degraded
        ) {
            break;
        }

        let mut work: Vec<ParseWorkItem> = chunk
            .iter()
            .map(|(rel_str, abs_path)| {
                let old_tree = tree_cache.remove(rel_str);
                (rel_str.clone(), abs_path.clone(), old_tree)
            })
            .collect();

        // Same boundary here: dependent-file parsing is parallel, DB access is
        // deferred until the sequential write phase below.
        let results: Vec<ParseResultRow> = work
            .par_iter_mut()
            .map(|(rel_str, abs_path, old_tree)| {
                let hash = match hash_file(abs_path) {
                    Ok(h) => h,
                    Err(e) => return (rel_str.clone(), Err(format!("hash error: {e}"))),
                };
                let source = match fs::read(abs_path.as_std_path()) {
                    Ok(b) => b,
                    Err(e) => return (rel_str.clone(), Err(format!("read error: {e}"))),
                };
                match registry.parse_checked(rel_str, &hash, &source, old_tree.as_ref()) {
                    Ok(Some((mut pf, tree))) => {
                        annotate_parsed_file_owner(&mut pf, owners.owner_for_path(rel_str));
                        annotate_parsed_file_repo(
                            &mut pf,
                            &source_repo_id,
                            repo_root.as_str(),
                            opts.namespace_qualified_names,
                        );
                        (rel_str.clone(), Ok((pf, tree)))
                    }
                    Ok(None) => (rel_str.clone(), Err("unsupported (skipped)".into())),
                    Err(msg) => (rel_str.clone(), Err(msg)),
                }
            })
            .collect();

        for (rel_str, outcome) in results {
            attempted_files += 1;
            match outcome {
                Ok((pf, tree)) => {
                    if let Some(t) = tree {
                        tree_cache.insert(rel_str.clone(), t);
                    }
                    parsed_deps.push(pf);
                }
                Err(msg) if msg == "unsupported (skipped)" => skipped_unsupported += 1,
                Err(msg) => {
                    tracing::warn!("processing dependent '{}' failed: {msg}", rel_str);
                    parse_errors += 1;
                    if matches!(
                        budget.note_parse_failure(attempted_files),
                        BuildBudgetDecision::Blocked
                    ) {
                        budget_blocked = true;
                    }
                    if opts.fail_fast {
                        return Err(anyhow::anyhow!(
                            "processing dependent '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }

            if budget_blocked {
                break;
            }
        }

        if budget_blocked {
            break;
        }
    }
    drop(_dep_parse_span);

    // ── Write all parsed files ────────────────────────────────────────────────
    let _write_span = tracing::info_span!("update.write").entered();
    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;
    let mut chunk_upsert_failures = 0usize;

    let all_parsed: Vec<&ParsedFile> = parsed_changed.iter().chain(parsed_deps.iter()).collect();
    for chunk in all_parsed.chunks(opts.batch_size) {
        if opts.dry_run {
            total_nodes += chunk.iter().map(|pf| pf.nodes.len()).sum::<usize>();
            total_edges += chunk.iter().map(|pf| pf.edges.len()).sum::<usize>();
            continue;
        }

        // Sequential persistence through the store's single owned connection.
        let chunk_owned: Vec<ParsedFile> = chunk.iter().map(|pf| (*pf).clone()).collect();
        let (n, e) = store
            .replace_files_transactional_for_repo(&source_repo_id, &chunk_owned)
            .context("cannot store parsed files")?;
        for pf in &chunk_owned {
            store
                .upsert_file_owner_for_repo(
                    &source_repo_id,
                    &pf.path,
                    owners.owner_for_path(&pf.path),
                )
                .with_context(|| format!("cannot store owner metadata for {}", pf.path))?;
        }
        total_nodes += n;
        total_edges += e;

        // Refresh chunk text in one transaction for this parsed batch.
        if let Err(err) = store.replace_chunks_for_parsed_files(&source_repo_id, &chunk_owned) {
            chunk_upsert_failures += 1;
            tracing::warn!("chunk batch upsert failed: {err:#}");
        }
    }
    drop(_write_span);

    if !opts.dry_run {
        refresh_owner_graphs(
            &mut store,
            repo_root,
            &owners,
            &source_repo_id,
            opts.namespace_qualified_names,
        )
        .context("cannot refresh package/workspace nodes")?;
    }

    let resolved_paths: Vec<String> = all_parsed.iter().map(|pf| pf.path.clone()).collect();
    let mut call_target_reconcile_failures = 0usize;
    let mut reachability = crate::reachability::ReachabilitySummary::default();
    if !resolved_paths.is_empty() && !opts.dry_run {
        match reconcile_call_targets(&mut store, repo_root, &source_repo_id, &resolved_paths) {
            Ok(reconcile) => {
                tracing::debug!(
                    touched_files = reconcile.touched_files,
                    "call-target reconcile complete"
                );
                reachability = reconcile.reachability;
            }
            Err(err) => {
                call_target_reconcile_failures += 1;
                tracing::warn!("late call-target resolution failed during update: {err:#}");
            }
        }
    }

    let parsed_count = parsed_changed.len() + parsed_deps.len();

    let (budget_counters, budget_report) = budget.finish();
    let mut warnings = Vec::new();
    if chunk_upsert_failures > 0 {
        let suffix = if chunk_upsert_failures == 1 { "" } else { "s" };
        warnings.push(format!(
            "retrieval chunk indexing degraded ({chunk_upsert_failures} failure{suffix})"
        ));
    }
    if call_target_reconcile_failures > 0 {
        let suffix = if call_target_reconcile_failures == 1 {
            ""
        } else {
            "s"
        };
        warnings.push(format!(
            "call target reconciliation degraded ({call_target_reconcile_failures} failure{suffix})"
        ));
    }

    let summary = UpdateSummary {
        deleted: deleted_count,
        renamed: renamed_count,
        parsed: parsed_count,
        skipped_unsupported,
        parse_errors,
        chunk_upsert_failures,
        call_target_reconcile_failures,
        nodes_updated: total_nodes,
        edges_updated: total_edges,
        warnings,
        budget_counters,
        budget: budget_report,
        reachability,
        elapsed_ms: started.elapsed().as_millis(),
    };

    // Record the synced git ref on clean runs. Degraded/partial runs keep
    // the old ref so a later update re-diffs against it and catches any
    // changes this run missed.
    if !opts.dry_run
        && !summary.is_degraded()
        && let Some(head) = head_ref(repo_root)
        && let Err(error) = store.set_last_indexed_ref(repo_root.as_str(), &source_repo_id, &head)
    {
        tracing::warn!("cannot record indexed ref: {error:#}");
    }

    Ok(summary)
}

#[cfg(test)]
mod tests;
