//! Full-build pipeline: scan tracked files, parse, persist to SQLite.

use std::collections::HashMap;
use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use atlas_core::{BudgetReport, BuildUpdateBudgetCounters, PackageOwner, model::ParsedFile};
use atlas_parser::ParserRegistry;
use atlas_repo::{
    collect_supported_files_with_stats, discover_package_owners, find_repo_root, hash_file,
};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use rayon::prelude::*;

use crate::build_budget::{BuildBudgetDecision, BuildBudgetTracker};
use crate::call_resolution::reconcile_call_targets;
use crate::config::BuildRunBudget;
use crate::owner_graph::refresh_owner_graphs;

/// Options controlling the build pipeline.
pub struct BuildOptions {
    /// Abort on the first parse or I/O failure instead of continuing.
    pub fail_fast: bool,
    /// Parse and summarize build work without mutating graph state.
    pub dry_run: bool,
    /// Number of files parsed per parallel batch.
    pub batch_size: usize,
    /// Centralized operational budget for build work.
    pub budget: BuildRunBudget,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            fail_fast: false,
            dry_run: false,
            batch_size: crate::config::DEFAULT_PARSE_BATCH_SIZE,
            budget: BuildRunBudget::default(),
        }
    }
}

/// Summary returned by `build_graph`.
#[derive(Debug, Default)]
pub struct BuildSummary {
    pub scanned: usize,
    pub skipped_unsupported: usize,
    pub skipped_unchanged: usize,
    pub parsed: usize,
    pub parse_errors: usize,
    pub nodes_inserted: usize,
    pub edges_inserted: usize,
    pub budget_counters: BuildUpdateBudgetCounters,
    pub budget: BudgetReport,
    pub elapsed_ms: u128,
}

/// Scan `repo_root`, parse all supported changed files, persist graph to `db_path`.
///
/// `repo_root` must be the directory returned by `find_repo_root` (absolute path
/// string).  `db_path` is the path to the Atlas SQLite database, which must
/// already exist (run `atlas init` first).
pub fn build_graph(
    repo_root: &Utf8Path,
    db_path: &str,
    opts: &BuildOptions,
) -> Result<BuildSummary> {
    let started = Instant::now();
    let mut budget = BuildBudgetTracker::new(opts.budget);

    let mut store =
        Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    store
        .upsert_repo(repo_root.as_str())
        .context("cannot register repo root for build state and history")?;

    if !opts.dry_run {
        for path in store
            .file_paths_with_prefix("")
            .context("cannot list existing graph files")?
        {
            store
                .delete_file_graph(&path)
                .with_context(|| format!("cannot clear stale graph for '{path}'"))?;
        }
    }

    let registry = ParserRegistry::with_defaults();
    let owners = discover_package_owners(repo_root).context("cannot discover package owners")?;

    let _scan_span = tracing::info_span!("build.scan").entered();

    // `file_hashes()` returns canonical graph file identities, so unchanged-file
    // reuse in full builds stays aligned with persisted graph keys and later
    // historical snapshot/file-hash reuse.
    let stored_hashes: HashMap<String, String> =
        store.file_hashes().context("cannot read stored hashes")?;

    let (all_files, scan_stats) = collect_supported_files_with_stats(
        repo_root,
        Some(opts.budget.max_file_bytes),
        |rel_path| registry.supports(rel_path.as_str()),
    )
    .context("cannot collect tracked files")?;

    let mut skipped_unsupported = scan_stats.skipped_unsupported;
    let scanned = all_files.len() + skipped_unsupported + scan_stats.skipped_by_size;
    budget.set_files_discovered(all_files.len());
    budget.note_scanned_file_byte_skips(scan_stats.skipped_by_size);
    let mut skipped_unchanged = 0usize;
    let mut parse_errors = 0usize;

    type Candidate = (String, camino::Utf8PathBuf, String);
    let mut candidates: Vec<Candidate> = Vec::new();

    for rel_path in &all_files {
        if matches!(
            budget.maybe_stop_for_time(started),
            BuildBudgetDecision::Degraded
        ) {
            break;
        }

        let rel_str = rel_path.as_str().to_owned();
        let abs_path = repo_root.join(rel_path);
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

        match budget.try_accept_file(file_bytes) {
            BuildBudgetDecision::Continue => {}
            BuildBudgetDecision::Degraded => break,
            BuildBudgetDecision::Blocked => {
                unreachable!("build file acceptance does not hard-block")
            }
        }

        let hash = match hash_file(&abs_path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("hashing '{}' failed: {e}", rel_str);
                parse_errors += 1;
                if opts.fail_fast {
                    return Err(e.context(format!("hashing '{rel_str}' failed (--fail-fast)")));
                }
                continue;
            }
        };

        if stored_hashes.get(&rel_str).is_some_and(|h| h == &hash) {
            skipped_unchanged += 1;
            continue;
        }

        candidates.push((rel_str, abs_path, hash));
    }

    drop(_scan_span);

    let _parse_span = tracing::info_span!("build.parse_and_write").entered();

    let mut parsed_count = 0usize;
    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;
    let mut resolved_paths: Vec<String> = Vec::new();
    let mut attempted_files = 0usize;
    let mut budget_blocked = false;

    for chunk in candidates.chunks(opts.batch_size) {
        if matches!(
            budget.maybe_stop_for_time(started),
            BuildBudgetDecision::Degraded
        ) {
            break;
        }

        // SQLite work stays outside this Rayon closure. Workers only read and
        // parse files; `Store` writes happen sequentially after collection.
        let results: Vec<(String, Result<ParsedFile, String>)> = chunk
            .par_iter()
            .map(|(rel_str, abs_path, hash)| {
                let source = match fs::read(abs_path.as_std_path()) {
                    Ok(b) => b,
                    Err(e) => return (rel_str.clone(), Err(format!("read error: {e}"))),
                };
                match registry.parse(rel_str, hash, &source, None) {
                    Some((mut pf, _tree)) => {
                        annotate_parsed_file_owner(&mut pf, owners.owner_for_path(rel_str));
                        (rel_str.clone(), Ok(pf))
                    }
                    None => (rel_str.clone(), Err("unsupported (skipped)".into())),
                }
            })
            .collect();

        let mut parsed_files: Vec<ParsedFile> = Vec::with_capacity(chunk.len());
        for (rel_str, outcome) in results {
            attempted_files += 1;
            match outcome {
                Ok(pf) => {
                    parsed_count += 1;
                    resolved_paths.push(pf.path.clone());
                    parsed_files.push(pf);
                }
                Err(msg) if msg == "unsupported (skipped)" => {
                    skipped_unsupported += 1;
                }
                Err(msg) => {
                    tracing::warn!("parsing '{}' failed: {msg}", rel_str);
                    parse_errors += 1;
                    if matches!(
                        budget.note_parse_failure(attempted_files),
                        BuildBudgetDecision::Blocked
                    ) {
                        budget_blocked = true;
                    }
                    if opts.fail_fast {
                        return Err(anyhow::anyhow!(
                            "parsing '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }

            if budget_blocked {
                break;
            }
        }

        if !parsed_files.is_empty() && !opts.dry_run {
            // Single-connection persistence phase: DB writes happen after the
            // parallel parse batch completes.
            let (n, e) = store
                .replace_files_transactional(&parsed_files)
                .context("cannot store parsed files")?;
            for pf in &parsed_files {
                store
                    .upsert_file_owner(&pf.path, owners.owner_for_path(&pf.path))
                    .with_context(|| format!("cannot store owner metadata for {}", pf.path))?;
            }
            total_nodes += n;
            total_edges += e;

            // Index chunk text for retrieval (embeddings generated separately).
            for pf in &parsed_files {
                for node in &pf.nodes {
                    if let Err(err) =
                        store.upsert_chunk(&node.qualified_name, 0, &node.chunk_text())
                    {
                        tracing::warn!("chunk upsert failed for {}: {err:#}", node.qualified_name);
                    }
                }
            }
        }

        if opts.dry_run {
            total_nodes += parsed_files.iter().map(|pf| pf.nodes.len()).sum::<usize>();
            total_edges += parsed_files.iter().map(|pf| pf.edges.len()).sum::<usize>();
        }

        if budget_blocked {
            break;
        }
    }

    drop(_parse_span);

    if !opts.dry_run {
        refresh_owner_graphs(&mut store, repo_root, &owners)
            .context("cannot refresh package/workspace nodes")?;
    }

    if !resolved_paths.is_empty()
        && !opts.dry_run
        && let Err(err) = reconcile_call_targets(&mut store, repo_root, &resolved_paths)
    {
        tracing::warn!("late call-target resolution failed during build: {err:#}");
    }

    let (budget_counters, budget_report) = budget.finish();

    Ok(BuildSummary {
        scanned,
        skipped_unsupported,
        skipped_unchanged,
        parsed: parsed_count,
        parse_errors,
        nodes_inserted: total_nodes,
        edges_inserted: total_edges,
        budget_counters,
        budget: budget_report,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// Detect and return the repo root for `start_dir`, delegating to git.
///
/// Convenience wrapper exposed for callers that only have a raw path string.
#[allow(dead_code)]
pub fn resolve_repo_root(start_dir: &str) -> Result<camino::Utf8PathBuf> {
    find_repo_root(Utf8Path::new(start_dir)).context("cannot find git repo root")
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
    use atlas_core::{BudgetStatus, Node, NodeId, NodeKind};
    use atlas_store_sqlite::Store;
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
    fn build_graph_replaces_same_hash_stale_file_graphs() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();

        git(repo_root, &["init", "--quiet"]);
        std::fs::write(
            repo_root.join("lib.rs"),
            "pub struct Greeter;\nimpl Greeter { pub fn greet(&self) {} }\n",
        )
        .unwrap();
        git(repo_root, &["add", "lib.rs"]);
        git(repo_root, &["commit", "--quiet", "-m", "init"]);

        let db_path = repo_root.join("worldtree.db");
        let lib_path = repo_root.join("lib.rs");
        let file_path = Utf8Path::from_path(&lib_path).unwrap();
        let file_hash = atlas_repo::hash_file(file_path).unwrap();

        let mut store = Store::open(db_path.to_str().unwrap()).unwrap();
        store
            .replace_file_graph(
                "lib.rs",
                &file_hash,
                Some("rust"),
                None,
                &[Node {
                    id: NodeId::UNSET,
                    kind: NodeKind::Function,
                    name: "stale".to_owned(),
                    qualified_name: "lib.rs::fn::stale".to_owned(),
                    file_path: "lib.rs".to_owned(),
                    line_start: 1,
                    line_end: 1,
                    language: "rust".to_owned(),
                    parent_name: Some("lib.rs".to_owned()),
                    params: Some("()".to_owned()),
                    return_type: None,
                    modifiers: None,
                    is_test: false,
                    file_hash: file_hash.clone(),
                    extra_json: serde_json::Value::Null,
                }],
                &[],
            )
            .unwrap();

        let summary = build_graph(
            Utf8Path::from_path(repo_root).unwrap(),
            db_path.to_str().unwrap(),
            &BuildOptions {
                fail_fast: true,
                dry_run: false,
                batch_size: 16,
                budget: BuildRunBudget::default(),
            },
        )
        .unwrap();

        assert_eq!(
            summary.skipped_unchanged, 0,
            "full build must not skip stored hashes"
        );
        assert_eq!(summary.parsed, 1, "full build must reparse tracked files");

        let refreshed = Store::open(db_path.to_str().unwrap()).unwrap();
        let qnames = refreshed.node_signatures_by_file("lib.rs").unwrap();
        assert!(!qnames.contains_key("lib.rs::fn::stale"));
        assert!(qnames.contains_key("lib.rs::struct::Greeter"));
        assert!(qnames.contains_key("lib.rs::method::Greeter::greet"));
        assert!(refreshed.dangling_edges(20).unwrap().is_empty());
        assert!(
            refreshed
                .find_repo_id(repo_root.to_str().unwrap())
                .unwrap()
                .is_some(),
            "full build must register repo root in repos table"
        );
    }

    #[test]
    fn build_graph_marks_partial_when_max_files_per_run_hit() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();

        git(repo_root, &["init", "--quiet"]);
        std::fs::write(repo_root.join("a.rs"), "pub fn a() {}\n").unwrap();
        std::fs::write(repo_root.join("b.rs"), "pub fn b() {}\n").unwrap();
        git(repo_root, &["add", "a.rs", "b.rs"]);
        git(repo_root, &["commit", "--quiet", "-m", "init"]);

        let budget = BuildRunBudget {
            max_files_per_run: 1,
            max_parse_failure_ratio_bps: 10_000,
            ..BuildRunBudget::default()
        };
        let db_path = repo_root.join("worldtree.db");

        let summary = build_graph(
            Utf8Path::from_path(repo_root).unwrap(),
            db_path.to_str().unwrap(),
            &BuildOptions {
                fail_fast: true,
                dry_run: false,
                batch_size: 16,
                budget,
            },
        )
        .unwrap();

        assert_eq!(summary.budget.budget_status, BudgetStatus::PartialResult);
        assert_eq!(summary.budget_counters.files_accepted, 1);
        assert_eq!(
            summary.budget_counters.budget_stop_reason.as_deref(),
            Some("max_files_per_run")
        );
        assert_eq!(summary.parsed, 1);
    }

    #[test]
    fn build_graph_marks_partial_when_total_byte_budget_hit() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();

        git(repo_root, &["init", "--quiet"]);
        std::fs::write(repo_root.join("a.rs"), "pub fn a() { let _x = 1; }\n").unwrap();
        std::fs::write(repo_root.join("b.rs"), "pub fn b() { let _y = 2; }\n").unwrap();
        git(repo_root, &["add", "a.rs", "b.rs"]);
        git(repo_root, &["commit", "--quiet", "-m", "init"]);

        let first_size = std::fs::metadata(repo_root.join("a.rs")).unwrap().len();
        let budget = BuildRunBudget {
            max_total_bytes_per_run: first_size,
            max_parse_failure_ratio_bps: 10_000,
            ..BuildRunBudget::default()
        };
        let db_path = repo_root.join("worldtree.db");

        let summary = build_graph(
            Utf8Path::from_path(repo_root).unwrap(),
            db_path.to_str().unwrap(),
            &BuildOptions {
                fail_fast: true,
                dry_run: false,
                batch_size: 16,
                budget,
            },
        )
        .unwrap();

        assert_eq!(summary.budget.budget_status, BudgetStatus::PartialResult);
        assert_eq!(summary.budget_counters.files_accepted, 1);
        assert_eq!(summary.budget_counters.files_skipped_by_byte_budget, 1);
        assert_eq!(summary.budget_counters.bytes_accepted, first_size);
        assert_eq!(
            summary.budget_counters.budget_stop_reason.as_deref(),
            Some("max_total_bytes_per_run")
        );
    }

    // --- parallel parse → sequential write boundary --------------------------
    //
    // Verifies that the full build pipeline collects all parallel parse results
    // before writing anything to the store.  A batch_size of 1 forces the
    // Rayon closure to run once per file; all resulting `ParsedFile` values are
    // collected into `results` before the sequential store write phase begins.
    // Every parsed function node must appear in the store after the build
    // completes, regardless of which Rayon thread produced it.
    #[test]
    fn build_parallel_parse_completes_before_store_write() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();

        git(repo_root, &["init", "--quiet"]);
        std::fs::write(repo_root.join("a.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(repo_root.join("b.rs"), "pub fn beta() {}\n").unwrap();
        std::fs::write(repo_root.join("c.rs"), "pub fn gamma() {}\n").unwrap();
        git(repo_root, &["add", "a.rs", "b.rs", "c.rs"]);
        git(repo_root, &["commit", "--quiet", "-m", "init"]);

        let db_path = repo_root.join("worldtree.db");
        let summary = build_graph(
            Utf8Path::from_path(repo_root).unwrap(),
            db_path.to_str().unwrap(),
            // batch_size=1 forces one file per Rayon batch; all results must
            // be collected before the sequential write phase processes them.
            &BuildOptions {
                fail_fast: true,
                dry_run: false,
                batch_size: 1,
                budget: BuildRunBudget::default(),
            },
        )
        .unwrap();

        assert_eq!(summary.parsed, 3, "all three files must be parsed");

        // Every function from every parallel-parsed file must land in the store.
        // If any write phase ran before parse collection finished, nodes from
        // later batches would be missing.
        let store = Store::open(db_path.to_str().unwrap()).unwrap();
        assert!(
            store
                .node_signatures_by_file("a.rs")
                .unwrap()
                .contains_key("a.rs::fn::alpha"),
            "a.rs::fn::alpha must be in the store"
        );
        assert!(
            store
                .node_signatures_by_file("b.rs")
                .unwrap()
                .contains_key("b.rs::fn::beta"),
            "b.rs::fn::beta must be in the store"
        );
        assert!(
            store
                .node_signatures_by_file("c.rs")
                .unwrap()
                .contains_key("c.rs::fn::gamma"),
            "c.rs::fn::gamma must be in the store"
        );
    }
}
