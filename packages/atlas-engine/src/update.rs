//! Incremental update pipeline: detect changes, parse changed + dependent files,
//! persist updated graph slices to SQLite.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use atlas_core::{
    BudgetReport, BuildUpdateBudgetCounters, PackageOwner,
    model::{ChangeType, ParsedFile},
};
use atlas_parser::{ParserRegistry, TreeCache};
use atlas_repo::{
    CanonicalRepoPath, DiffTarget, changed_files, discover_package_owners, hash_file,
};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use rayon::prelude::*;

use crate::build_budget::{BuildBudgetDecision, BuildBudgetTracker};
use crate::call_resolution::reconcile_call_targets;
use crate::config::BuildRunBudget;
use crate::owner_graph::refresh_owner_graphs;

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
pub enum UpdateTarget {
    /// Unstaged working-tree changes.
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
    /// Number of files parsed per parallel batch.
    pub batch_size: usize,
    /// Which changes to process.
    pub target: UpdateTarget,
    /// Centralized operational budget for update work.
    pub budget: BuildRunBudget,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        Self {
            fail_fast: false,
            batch_size: crate::config::DEFAULT_PARSE_BATCH_SIZE,
            target: UpdateTarget::WorkingTree,
            budget: BuildRunBudget::default(),
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
    pub budget_counters: BuildUpdateBudgetCounters,
    pub budget: BudgetReport,
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
    let mut budget = BuildBudgetTracker::new(opts.budget);

    let mut store =
        Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let owners = discover_package_owners(repo_root).context("cannot discover package owners")?;

    // ── Determine which files changed ────────────────────────────────────────
    let _diff_span = tracing::info_span!("update.detect_changes").entered();

    let git_changes: Vec<atlas_core::model::ChangedFile> = match &opts.target {
        UpdateTarget::Files(paths) => paths
            .iter()
            .map(|p| {
                let rel = CanonicalRepoPath::from_cli_argument(repo_root, Utf8Path::new(p))
                    .with_context(|| format!("invalid explicit update path '{p}'"))?;
                Ok(atlas_core::model::ChangedFile {
                    path: rel.as_str().to_owned(),
                    change_type: ChangeType::Modified,
                    old_path: None,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        UpdateTarget::Batch(changes) => changes
            .iter()
            .map(canonicalize_batch_change)
            .collect::<Result<Vec<_>>>()?,
        other => {
            let diff_target = match other {
                UpdateTarget::Staged => DiffTarget::Staged,
                UpdateTarget::BaseRef(r) => DiffTarget::BaseRef(r.clone()),
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
                    let new_hash = atlas_repo::hash_file(&new_abs).ok();
                    let stored_hash = store.file_hash(old).ok().flatten();
                    let old_owner_id = store.file_owner_id(old).ok().flatten();
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
            store
                .upsert_file_owner(new, owners.owner_for_path(new))
                .with_context(|| format!("cannot update owner metadata for '{new}'"))?;
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
                match registry.parse(rel_str, &hash, &source, old_tree.as_ref()) {
                    Some((mut pf, tree)) => {
                        annotate_parsed_file_owner(&mut pf, owners.owner_for_path(rel_str));
                        (rel_str.clone(), Ok((pf, tree)))
                    }
                    None => (rel_str.clone(), Err("unsupported (skipped)".into())),
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
                match registry.parse(rel_str, &hash, &source, old_tree.as_ref()) {
                    Some((mut pf, tree)) => {
                        annotate_parsed_file_owner(&mut pf, owners.owner_for_path(rel_str));
                        (rel_str.clone(), Ok((pf, tree)))
                    }
                    None => (rel_str.clone(), Err("unsupported (skipped)".into())),
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

    let all_parsed: Vec<&ParsedFile> = parsed_changed.iter().chain(parsed_deps.iter()).collect();
    for chunk in all_parsed.chunks(opts.batch_size) {
        let chunk_owned: Vec<ParsedFile> = chunk.iter().map(|pf| (*pf).clone()).collect();
        let (n, e) = store
            .replace_files_transactional(&chunk_owned)
            .context("cannot store parsed files")?;
        for pf in &chunk_owned {
            store
                .upsert_file_owner(&pf.path, owners.owner_for_path(&pf.path))
                .with_context(|| format!("cannot store owner metadata for {}", pf.path))?;
        }
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

    refresh_owner_graphs(&mut store, repo_root, &owners)
        .context("cannot refresh package/workspace nodes")?;

    let resolved_paths: Vec<String> = all_parsed.iter().map(|pf| pf.path.clone()).collect();
    if !resolved_paths.is_empty()
        && let Err(err) = reconcile_call_targets(&mut store, repo_root, &resolved_paths)
    {
        tracing::warn!("late call-target resolution failed during update: {err:#}");
    }

    let parsed_count = parsed_changed.len() + parsed_deps.len();

    let (budget_counters, budget_report) = budget.finish();

    Ok(UpdateSummary {
        deleted: deleted_count,
        renamed: renamed_count,
        parsed: parsed_count,
        skipped_unsupported,
        parse_errors,
        nodes_updated: total_nodes,
        edges_updated: total_edges,
        budget_counters,
        budget: budget_report,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn annotate_parsed_file_owner(parsed_file: &mut ParsedFile, owner: Option<&PackageOwner>) {
    let Some(owner) = owner else {
        return;
    };
    for node in &mut parsed_file.nodes {
        let mut extra = node.extra_json.as_object().cloned().unwrap_or_default();
        extra.insert(
            "owner_id".to_owned(),
            serde_json::Value::String(owner.owner_id.clone()),
        );
        extra.insert(
            "owner_kind".to_owned(),
            serde_json::Value::String(owner.kind.as_str().to_owned()),
        );
        extra.insert(
            "owner_root".to_owned(),
            serde_json::Value::String(owner.root.clone()),
        );
        extra.insert(
            "owner_manifest_path".to_owned(),
            serde_json::Value::String(owner.manifest_path.clone()),
        );
        if let Some(package_name) = &owner.package_name {
            extra.insert(
                "owner_name".to_owned(),
                serde_json::Value::String(package_name.clone()),
            );
        }
        node.extra_json = serde_json::Value::Object(extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildOptions, BuildRunBudget, build_graph};
    use atlas_core::BudgetStatus;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Atlas Test")
            .env("GIT_AUTHOR_EMAIL", "test@atlas")
            .env("GIT_COMMITTER_NAME", "Atlas Test")
            .env("GIT_COMMITTER_EMAIL", "test@atlas")
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn update_graph_blocks_when_parse_failure_budget_is_exceeded() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();

        git(repo_root, &["init", "--quiet"]);
        std::fs::write(repo_root.join("lib.rs"), "pub fn ok() {}\n").unwrap();
        git(repo_root, &["add", "lib.rs"]);
        git(repo_root, &["commit", "--quiet", "-m", "init"]);

        let db_path = repo_root.join("worldtree.db");
        build_graph(
            Utf8Path::from_path(repo_root).unwrap(),
            db_path.to_str().unwrap(),
            &BuildOptions {
                fail_fast: true,
                batch_size: 16,
                budget: BuildRunBudget::default(),
            },
        )
        .unwrap();

        let lib_path = repo_root.join("lib.rs");
        let mut perms = std::fs::metadata(&lib_path).unwrap().permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&lib_path, perms).unwrap();

        let budget = BuildRunBudget {
            max_parse_failures: 0,
            max_parse_failure_ratio_bps: 10_000,
            ..BuildRunBudget::default()
        };

        let summary = update_graph(
            Utf8Path::from_path(repo_root).unwrap(),
            db_path.to_str().unwrap(),
            &UpdateOptions {
                fail_fast: false,
                batch_size: 16,
                target: UpdateTarget::WorkingTree,
                budget,
            },
        )
        .unwrap();

        assert_eq!(summary.budget.budget_status, BudgetStatus::Blocked);
        assert_eq!(summary.parse_errors, 1);
        assert_eq!(
            summary.budget_counters.budget_stop_reason.as_deref(),
            Some("max_parse_failures")
        );
    }
}
