use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use atlas_core::SearchQuery;
use atlas_core::model::{
    ChangeType, ImpactResult, ParsedFile, ReviewContext, ReviewImpactOverview, RiskSummary,
};
use atlas_impact::analyze as advanced_impact;
use atlas_parser::ParserRegistry;
use atlas_repo::{
    DiffTarget, changed_files, collect_supported_files, find_repo_root, hash_file, repo_relative,
};
use atlas_search as search;
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use rayon::prelude::*;

use crate::cli::{Cli, Command};

/// Default parse-worker batch size.  Override with `ATLAS_PARSE_BATCH_SIZE`.
const DEFAULT_PARSE_BATCH_SIZE: usize = 64;
const MACHINE_SCHEMA_VERSION: &str = "atlas_cli.v1";

/// Read the effective batch size from the environment, falling back to the
/// compile-time default.  Clamps to [1, 4096] to prevent extreme values.
fn parse_batch_size() -> usize {
    std::env::var("ATLAS_PARSE_BATCH_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|n| n.clamp(1, 4096))
        .unwrap_or(DEFAULT_PARSE_BATCH_SIZE)
}

/// Compute a content-signature string for `node`.
///
/// Captures the interface attributes that, when changed, indicate dependents
/// may be affected.  Line positions are excluded intentionally — moving a
/// symbol within a file does not change its interface.
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

/// Collect the qualified names of symbols whose interface signature changed or
/// that were added/removed between `old_sigs` and the freshly parsed `nodes`.
fn changed_qnames(old_sigs: &HashMap<String, String>, nodes: &[atlas_core::Node]) -> Vec<String> {
    let mut changed: Vec<String> = Vec::new();

    // Added or signature-changed symbols.
    for n in nodes {
        let new_sig = node_signature(n);
        match old_sigs.get(&n.qualified_name) {
            Some(old_sig) if old_sig == &new_sig => {}
            _ => changed.push(n.qualified_name.clone()),
        }
    }

    // Removed symbols (in old but not in new).
    let new_qns: HashSet<&str> = nodes.iter().map(|n| n.qualified_name.as_str()).collect();
    for qn in old_sigs.keys() {
        if !new_qns.contains(qn.as_str()) {
            changed.push(qn.clone());
        }
    }
    changed
}

fn json_envelope(command: &str, data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION,
        "command": command,
        "data": data,
    })
}

fn print_json(command: &str, data: serde_json::Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&json_envelope(command, data))?
    );
    Ok(())
}

fn detect_changes_target(base: &Option<String>, staged: bool) -> DiffTarget {
    if staged {
        DiffTarget::Staged
    } else if let Some(base_ref) = base {
        DiffTarget::BaseRef(base_ref.clone())
    } else {
        DiffTarget::WorkingTree
    }
}

fn supported_update_candidates(
    repo_root: &Utf8Path,
    registry: &ParserRegistry,
    paths: &[String],
) -> (Vec<(String, camino::Utf8PathBuf)>, usize) {
    let mut skipped_unsupported = 0usize;
    let candidates = paths
        .iter()
        .filter_map(|rel_str| {
            if !registry.supports(rel_str) {
                skipped_unsupported += 1;
                return None;
            }
            Some((rel_str.clone(), repo_root.join(rel_str)))
        })
        .collect();
    (candidates, skipped_unsupported)
}

fn change_tag(change_type: ChangeType) -> &'static str {
    match change_type {
        ChangeType::Added => "A",
        ChangeType::Modified => "M",
        ChangeType::Deleted => "D",
        ChangeType::Renamed => "R",
        ChangeType::Copied => "C",
    }
}

fn augment_changes_with_node_counts(
    changes: &[atlas_core::model::ChangedFile],
    store: Option<&Store>,
) -> Vec<serde_json::Value> {
    changes
        .iter()
        .map(|cf| {
            let node_count = store
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| ns.len());
            let mut value = serde_json::to_value(cf).unwrap_or_default();
            if let Some(count) = node_count {
                value["node_count"] = serde_json::json!(count);
            }
            value
        })
        .collect()
}

fn status_payload(
    repo: &str,
    db_path: &str,
    stats: &atlas_core::GraphStats,
    base: &Option<String>,
    staged: bool,
    changes: &[atlas_core::model::ChangedFile],
    store: Option<&Store>,
) -> serde_json::Value {
    serde_json::json!({
        "repo_root": repo,
        "db_path": db_path,
        "diff_target": {
            "base": base,
            "staged": staged,
            "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
        },
        "indexed_file_count": stats.file_count,
        "node_count": stats.node_count,
        "edge_count": stats.edge_count,
        "nodes_by_kind": stats.nodes_by_kind,
        "languages": stats.languages,
        "last_indexed_at": stats.last_indexed_at,
        "changed_file_count": changes.len(),
        "changed_files": augment_changes_with_node_counts(changes, store),
    })
}

pub fn run_init(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let atlas_dir = crate::paths::atlas_dir(&repo);
    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;

    let db_path = db_path(cli, &repo);
    Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    if cli.json {
        print_json(
            "init",
            serde_json::json!({
                "atlas_dir": atlas_dir.display().to_string(),
                "db_path": db_path,
            }),
        )?;
    } else {
        println!("Initialized atlas in {}", atlas_dir.display());
        println!("Database: {db_path}");
    }
    Ok(())
}

pub fn run_status(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let (base, staged) = match &cli.command {
        Command::Status { base, staged } => (base.clone(), *staged),
        _ => unreachable!(),
    };

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let stats = store.stats().context("cannot read stats")?;
    let changes = changed_files(repo_root, &detect_changes_target(&base, staged))
        .context("cannot detect changed files")?;

    if cli.json {
        print_json(
            "status",
            status_payload(
                &repo,
                &db_path,
                &stats,
                &base,
                staged,
                &changes,
                Some(&store),
            ),
        )?;
    } else {
        println!("Repo root : {repo}");
        println!("Database  : {db_path}");
        println!("Files     : {}", stats.file_count);
        println!("Nodes     : {}", stats.node_count);
        println!("Edges     : {}", stats.edge_count);
        if !stats.languages.is_empty() {
            println!("Languages : {}", stats.languages.join(", "));
        }
        if !stats.nodes_by_kind.is_empty() {
            println!("Nodes by kind:");
            for (kind, count) in &stats.nodes_by_kind {
                println!("  {kind:<14} {count}");
            }
        }
        if let Some(ts) = &stats.last_indexed_at {
            println!("Last indexed: {ts}");
        }
        if base.is_some() || staged || !changes.is_empty() {
            println!("Changed files: {}", changes.len());
            for cf in &changes {
                let node_info = store
                    .nodes_by_file(&cf.path)
                    .ok()
                    .map(|nodes| format!(" [{} nodes]", nodes.len()))
                    .unwrap_or_default();
                if let Some(old) = &cf.old_path {
                    println!(
                        "  {}  {old} -> {}{node_info}",
                        change_tag(cf.change_type),
                        cf.path
                    );
                } else {
                    println!("  {}  {}{node_info}", change_tag(cf.change_type), cf.path);
                }
            }
        }
    }
    Ok(())
}

pub fn run_build(cli: &Cli) -> Result<()> {
    let started = Instant::now();

    let fail_fast = matches!(&cli.command, Command::Build { fail_fast } if *fail_fast);

    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let mut store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let registry = ParserRegistry::with_defaults();

    let _scan_span = tracing::info_span!("build.scan").entered();

    // Load stored hashes once to skip unchanged files.
    let stored_hashes = store.file_hashes().context("cannot read stored hashes")?;

    let (all_files, mut skipped_unsupported) =
        collect_supported_files(repo_root, None, |rel_path| {
            registry.supports(rel_path.as_str())
        })
        .context("cannot collect tracked files")?;

    let scanned = all_files.len() + skipped_unsupported;
    let mut skipped_unchanged = 0usize;
    let mut parse_errors = 0usize;

    // Candidates: (rel_path_string, abs_path, hash)
    type Candidate = (String, camino::Utf8PathBuf, String);
    let mut candidates: Vec<Candidate> = Vec::new();

    for rel_path in &all_files {
        let rel_str = rel_path.as_str().to_owned();

        let abs_path = repo_root.join(rel_path);
        let hash = match hash_file(&abs_path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("hashing '{}' failed: {e}", rel_str);
                parse_errors += 1;
                if fail_fast {
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

    // --- Parallel parse in bounded chunks, batched DB write ------------------
    let batch_size = parse_batch_size();
    let _parse_span = tracing::info_span!("build.parse_and_write").entered();

    let mut parsed_count = 0usize;
    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;

    for chunk in candidates.chunks(batch_size) {
        // Parse this chunk in parallel.
        let results: Vec<(String, Result<ParsedFile, String>)> = chunk
            .par_iter()
            .map(|(rel_str, abs_path, hash)| {
                let source = match fs::read(abs_path.as_std_path()) {
                    Ok(b) => b,
                    Err(e) => {
                        return (rel_str.clone(), Err(format!("read error: {e}")));
                    }
                };
                match registry.parse(rel_str, hash, &source) {
                    Some(pf) => (rel_str.clone(), Ok(pf)),
                    None => (rel_str.clone(), Err("unsupported (skipped)".into())),
                }
            })
            .collect();

        // Collect successful parses and handle errors.
        let mut parsed_files: Vec<ParsedFile> = Vec::with_capacity(chunk.len());
        for (rel_str, outcome) in results {
            match outcome {
                Ok(pf) => {
                    parsed_count += 1;
                    parsed_files.push(pf);
                }
                Err(msg) if msg == "unsupported (skipped)" => {
                    skipped_unsupported += 1;
                }
                Err(msg) => {
                    tracing::warn!("parsing '{}' failed: {msg}", rel_str);
                    parse_errors += 1;
                    if fail_fast {
                        return Err(anyhow::anyhow!(
                            "parsing '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }
        }

        // Write all successful parses in one transaction per chunk.
        if !parsed_files.is_empty() {
            let (n, e) = store
                .replace_files_transactional(&parsed_files)
                .context("cannot store parsed files")?;
            total_nodes += n;
            total_edges += e;
        }
    }

    drop(_parse_span);
    let elapsed = started.elapsed();

    if cli.json {
        let summary = serde_json::json!({
            "scanned": scanned,
            "skipped_unsupported": skipped_unsupported,
            "skipped_unchanged": skipped_unchanged,
            "parsed": parsed_count,
            "parse_errors": parse_errors,
            "nodes_inserted": total_nodes,
            "edges_inserted": total_edges,
            "elapsed_ms": elapsed.as_millis(),
        });
        print_json("build", summary)?;
    } else {
        println!("Build complete ({:.2}s)", elapsed.as_secs_f64());
        println!("  Scanned             : {scanned}");
        println!("  Unsupported skipped : {skipped_unsupported}");
        println!("  Unchanged skipped   : {skipped_unchanged}");
        println!("  Parsed              : {parsed_count}");
        if parse_errors > 0 {
            println!("  Errors              : {parse_errors}");
        }
        println!("  Nodes inserted      : {total_nodes}");
        println!("  Edges inserted      : {total_edges}");
    }

    Ok(())
}

pub fn run_update(cli: &Cli) -> Result<()> {
    let started = Instant::now();

    let fail_fast = matches!(
        &cli.command,
        Command::Update { fail_fast, .. } if *fail_fast
    );

    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let mut store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let explicit_files: Vec<String> = match &cli.command {
        Command::Update { files, .. } => files.clone(),
        _ => vec![],
    };

    let _diff_span = tracing::info_span!("update.detect_changes").entered();

    let git_changes: Vec<atlas_core::model::ChangedFile> = if explicit_files.is_empty() {
        let target = match &cli.command {
            Command::Update { base, staged, .. } => {
                if *staged {
                    DiffTarget::Staged
                } else if let Some(base_ref) = base {
                    DiffTarget::BaseRef(base_ref.clone())
                } else {
                    DiffTarget::WorkingTree
                }
            }
            _ => DiffTarget::WorkingTree,
        };
        changed_files(repo_root, &target).context("cannot detect changed files")?
    } else {
        explicit_files
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
            .collect()
    };

    drop(_diff_span);

    // Split deleted from to-parse.
    let mut to_delete: Vec<String> = Vec::new();
    let mut to_parse_paths: Vec<String> = Vec::new();

    for cf in &git_changes {
        match cf.change_type {
            ChangeType::Deleted => {
                to_delete.push(cf.path.clone());
            }
            ChangeType::Renamed | ChangeType::Copied => {
                if let Some(old) = &cf.old_path {
                    to_delete.push(old.clone());
                }
                to_parse_paths.push(cf.path.clone());
            }
            _ => {
                to_parse_paths.push(cf.path.clone());
            }
        }
    }

    // -----------------------------------------------------------------------
    // Incremental parsing: load old node signatures before reparsing so we
    // can detect which symbols actually changed.
    // -----------------------------------------------------------------------
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

    // Remove stale graphs first (before parsing so dependent queries are clean).
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
    let batch_size = parse_batch_size();

    // -----------------------------------------------------------------------
    // Phase 1: parse the directly-changed files.
    // -----------------------------------------------------------------------
    let (changed_candidates, changed_unsupported) =
        supported_update_candidates(repo_root, &registry, &to_parse_paths);
    skipped_unsupported += changed_unsupported;

    let _parse_span = tracing::info_span!("update.parse_changed").entered();

    let mut parsed_changed: Vec<ParsedFile> = Vec::new();
    for chunk in changed_candidates.chunks(batch_size) {
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
                    if fail_fast {
                        return Err(anyhow::anyhow!(
                            "processing '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }
        }
    }

    drop(_parse_span);

    // -----------------------------------------------------------------------
    // Dependency invalidation: compute which symbols changed, then use a
    // targeted query instead of a file-level scan to reduce over-invalidation.
    // -----------------------------------------------------------------------
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

    // Build the set of changed paths so we skip those already parsed.
    let changed_paths_set: HashSet<&str> = to_parse_paths.iter().map(String::as_str).collect();

    let dependent_paths: Vec<String> = dependents
        .iter()
        .filter(|d| !changed_paths_set.contains(d.as_str()))
        .cloned()
        .collect();
    let (dep_candidates, dep_unsupported) =
        supported_update_candidates(repo_root, &registry, &dependent_paths);
    skipped_unsupported += dep_unsupported;

    // -----------------------------------------------------------------------
    // Phase 2: parse dependent files.
    // -----------------------------------------------------------------------
    let _dep_parse_span = tracing::info_span!("update.parse_dependents").entered();

    let mut parsed_deps: Vec<ParsedFile> = Vec::new();
    for chunk in dep_candidates.chunks(batch_size) {
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
                    if fail_fast {
                        return Err(anyhow::anyhow!(
                            "processing dependent '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }
        }
    }

    drop(_dep_parse_span);

    // -----------------------------------------------------------------------
    // Write all parsed files (changed + dependents) in batched transactions.
    // -----------------------------------------------------------------------
    let _write_span = tracing::info_span!("update.write").entered();

    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;

    let all_parsed: Vec<&ParsedFile> = parsed_changed.iter().chain(parsed_deps.iter()).collect();

    for chunk in all_parsed.chunks(batch_size) {
        let chunk_owned: Vec<ParsedFile> = chunk.iter().map(|pf| (*pf).clone()).collect();
        let (n, e) = store
            .replace_files_transactional(&chunk_owned)
            .context("cannot store parsed files")?;
        total_nodes += n;
        total_edges += e;
    }

    drop(_write_span);

    let parsed_count = parsed_changed.len() + parsed_deps.len();
    let elapsed = started.elapsed();

    if cli.json {
        let summary = serde_json::json!({
            "deleted": deleted_count,
            "parsed": parsed_count,
            "skipped_unsupported": skipped_unsupported,
            "parse_errors": parse_errors,
            "nodes_updated": total_nodes,
            "edges_updated": total_edges,
            "elapsed_ms": elapsed.as_millis(),
        });
        print_json("update", summary)?;
    } else {
        println!("Update complete ({:.2}s)", elapsed.as_secs_f64());
        println!("  Deleted  : {deleted_count}");
        println!("  Parsed   : {parsed_count}");
        if skipped_unsupported > 0 {
            println!("  Unsupported skipped : {skipped_unsupported}");
        }
        if parse_errors > 0 {
            println!("  Errors   : {parse_errors}");
        }
        println!("  Nodes    : {total_nodes}");
        println!("  Edges    : {total_edges}");
    }

    Ok(())
}

pub fn run_detect_changes(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let (base, staged) = match &cli.command {
        Command::DetectChanges { base, staged } => (base.clone(), *staged),
        _ => unreachable!(),
    };
    let diff_target = detect_changes_target(&base, staged);

    let changes = changed_files(repo_root, &diff_target).context("cannot detect changed files")?;

    // Try to open the DB for graph summary — tolerate failure (DB may not exist yet).
    let store_result = Store::open(&db_path);

    if cli.json {
        print_json(
            "detect_changes",
            serde_json::json!({
                "diff_target": {
                    "base": base,
                    "staged": staged,
                    "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
                },
                "changes": augment_changes_with_node_counts(&changes, store_result.as_ref().ok()),
            }),
        )?;
    } else if changes.is_empty() {
        println!("No changed files detected.");
    } else {
        for cf in &changes {
            let node_info = store_result
                .as_ref()
                .ok()
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| format!(" [{} nodes]", ns.len()))
                .unwrap_or_default();
            if let Some(old) = &cf.old_path {
                println!(
                    "{}  {old} -> {}{node_info}",
                    change_tag(cf.change_type),
                    cf.path
                );
            } else {
                println!("{}  {}{node_info}", change_tag(cf.change_type), cf.path);
            }
        }
        println!("\n{} file(s) changed.", changes.len());

        // Graph-level impact summary when DB is available.
        if let Ok(store) = &store_result {
            let non_deleted: Vec<&str> = changes
                .iter()
                .filter(|cf| cf.change_type != ChangeType::Deleted)
                .map(|cf| cf.path.as_str())
                .collect();
            if !non_deleted.is_empty()
                && let Ok(impact) = store.impact_radius(&non_deleted, 5, 200)
            {
                println!("\nGraph impact summary:");
                println!("  Changed symbols : {}", impact.changed_nodes.len());
                println!("  Impacted nodes  : {}", impact.impacted_nodes.len());
                println!("  Impacted files  : {}", impact.impacted_files.len());
            }
        }
    }

    Ok(())
}

pub fn run_query(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let (text, kind, language, subpath, limit, expand, expand_hops) = match &cli.command {
        Command::Query {
            text,
            kind,
            language,
            subpath,
            limit,
            expand,
            expand_hops,
        } => (
            text.clone(),
            kind.clone(),
            language.clone(),
            subpath.clone(),
            *limit,
            *expand,
            *expand_hops,
        ),
        _ => unreachable!(),
    };

    let query = SearchQuery {
        text,
        kind,
        language,
        subpath,
        limit,
        graph_expand: expand,
        graph_max_hops: expand_hops,
        ..Default::default()
    };

    let results = search::search(&store, &query).context("search failed")?;

    if cli.json {
        print_json(
            "query",
            serde_json::json!({
                "query": {
                    "text": query.text,
                    "kind": query.kind,
                    "language": query.language,
                    "subpath": query.subpath,
                    "limit": query.limit,
                    "graph_expand": query.graph_expand,
                    "graph_max_hops": query.graph_max_hops,
                },
                "results": results,
            }),
        )?;
    } else if results.is_empty() {
        println!("No results.");
    } else {
        for r in &results {
            let n = &r.node;
            println!(
                "[{:.3}] {} {} ({}:{})",
                r.score,
                n.kind.as_str(),
                n.qualified_name,
                n.file_path,
                n.line_start,
            );
        }
        println!("\n{} result(s).", results.len());
    }

    Ok(())
}

pub fn run_impact(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let (base, explicit_files, max_depth, max_nodes) = match &cli.command {
        Command::Impact {
            base,
            files,
            max_depth,
            max_nodes,
        } => (base.clone(), files.clone(), *max_depth, *max_nodes as usize),
        _ => unreachable!(),
    };

    let target_files: Vec<String> = if !explicit_files.is_empty() {
        explicit_files
            .iter()
            .map(|p| {
                let abs = Utf8Path::new(p);
                if abs.is_absolute() {
                    repo_relative(repo_root, abs)
                        .unwrap_or_else(|_| abs.to_owned())
                        .to_string()
                } else {
                    p.clone()
                }
            })
            .collect()
    } else {
        let diff_target = if let Some(base_ref) = &base {
            DiffTarget::BaseRef(base_ref.clone())
        } else {
            DiffTarget::WorkingTree
        };
        changed_files(repo_root, &diff_target)
            .context("cannot detect changed files")?
            .into_iter()
            .filter(|cf| cf.change_type != ChangeType::Deleted)
            .map(|cf| cf.path)
            .collect()
    };

    if target_files.is_empty() {
        if cli.json {
            print_json(
                "impact",
                serde_json::json!({
                    "files": target_files,
                    "analysis": ImpactResult {
                    changed_nodes: vec![],
                    impacted_nodes: vec![],
                    impacted_files: vec![],
                    relevant_edges: vec![],
                    }
                }),
            )?;
        } else {
            println!("No changed files detected.");
        }
        return Ok(());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(&path_refs, max_depth, max_nodes)
        .context("impact radius query failed")?;

    let advanced = advanced_impact(result);

    if cli.json {
        print_json(
            "impact",
            serde_json::json!({
                "files": target_files,
                "analysis": advanced,
            }),
        )?;
    } else {
        println!("Changed files : {}", target_files.len());
        println!("Changed nodes : {}", advanced.base.changed_nodes.len());
        println!("Impacted nodes: {}", advanced.base.impacted_nodes.len());
        println!("Impacted files: {}", advanced.base.impacted_files.len());
        println!("Relevant edges: {}", advanced.base.relevant_edges.len());
        println!("Risk level    : {}", advanced.risk_level);
        if !advanced.base.impacted_files.is_empty() {
            println!("\nImpacted files:");
            for f in &advanced.base.impacted_files {
                println!("  {f}");
            }
        }
        if !advanced.scored_nodes.is_empty() {
            println!("\nTop impacted nodes (by score):");
            for sn in advanced.scored_nodes.iter().take(20) {
                let ck = sn
                    .change_kind
                    .map(|c| format!(" [{c}]"))
                    .unwrap_or_default();
                println!(
                    "  {:>6.2}  {} {}{}",
                    sn.impact_score,
                    sn.node.kind.as_str(),
                    sn.node.qualified_name,
                    ck
                );
            }
        }
        if !advanced.test_impact.affected_tests.is_empty() {
            println!(
                "\nAffected tests: {}",
                advanced.test_impact.affected_tests.len()
            );
        }
        if !advanced.test_impact.uncovered_changed_nodes.is_empty() {
            println!("\nChanged nodes with no test coverage:");
            for n in &advanced.test_impact.uncovered_changed_nodes {
                println!("  {} {}", n.kind.as_str(), n.qualified_name);
            }
        }
        if !advanced.boundary_violations.is_empty() {
            println!("\nBoundary violations:");
            for v in &advanced.boundary_violations {
                println!("  [{}] {}", v.kind, v.description);
            }
        }
    }

    Ok(())
}

pub fn run_review_context(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let (base, explicit_files, max_depth, max_nodes) = match &cli.command {
        Command::ReviewContext {
            base,
            files,
            max_depth,
            max_nodes,
        } => (base.clone(), files.clone(), *max_depth, *max_nodes as usize),
        _ => unreachable!(),
    };

    let target_files: Vec<String> = if !explicit_files.is_empty() {
        explicit_files
            .iter()
            .map(|p| {
                let abs = Utf8Path::new(p);
                if abs.is_absolute() {
                    repo_relative(repo_root, abs)
                        .unwrap_or_else(|_| abs.to_owned())
                        .to_string()
                } else {
                    p.clone()
                }
            })
            .collect()
    } else {
        let diff_target = if let Some(base_ref) = &base {
            DiffTarget::BaseRef(base_ref.clone())
        } else {
            DiffTarget::WorkingTree
        };
        changed_files(repo_root, &diff_target)
            .context("cannot detect changed files")?
            .into_iter()
            .filter(|cf| cf.change_type != ChangeType::Deleted)
            .map(|cf| cf.path)
            .collect()
    };

    if target_files.is_empty() {
        if cli.json {
            let empty = ReviewContext {
                changed_files: vec![],
                changed_symbols: vec![],
                changed_symbol_summaries: vec![],
                impacted_neighbors: vec![],
                critical_edges: vec![],
                impact_overview: ReviewImpactOverview {
                    max_depth,
                    max_nodes,
                    impacted_node_count: 0,
                    impacted_file_count: 0,
                    relevant_edge_count: 0,
                    reached_node_limit: false,
                },
                risk_summary: RiskSummary {
                    changed_symbol_count: 0,
                    public_api_changes: 0,
                    test_adjacent: false,
                    affected_test_count: 0,
                    uncovered_changed_symbol_count: 0,
                    large_function_touched: false,
                    large_function_count: 0,
                    cross_module_impact: false,
                    cross_package_impact: false,
                },
            };
            print_json(
                "review_context",
                serde_json::json!({
                    "files": target_files,
                    "review_context": empty,
                }),
            )?;
        } else {
            println!("No changed files detected.");
        }
        return Ok(());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let impact = store
        .impact_radius(&path_refs, max_depth, max_nodes)
        .context("impact radius query failed")?;

    let ctx = atlas_review::assemble_review_context(&impact, &target_files, max_depth, max_nodes);

    if cli.json {
        print_json(
            "review_context",
            serde_json::json!({
                "files": target_files,
                "review_context": ctx,
            }),
        )?;
    } else {
        println!("Changed files ({}):", ctx.changed_files.len());
        for f in &ctx.changed_files {
            println!("  {f}");
        }
        println!("\nImpact radius:");
        println!("  Max depth         : {}", ctx.impact_overview.max_depth);
        println!("  Max nodes         : {}", ctx.impact_overview.max_nodes);
        println!(
            "  Impacted nodes    : {}",
            ctx.impact_overview.impacted_node_count
        );
        println!(
            "  Impacted files    : {}",
            ctx.impact_overview.impacted_file_count
        );
        println!(
            "  Relevant edges    : {}",
            ctx.impact_overview.relevant_edge_count
        );
        println!(
            "  Node limit reached: {}",
            ctx.impact_overview.reached_node_limit
        );
        println!(
            "\nChanged symbols: {}",
            ctx.risk_summary.changed_symbol_count
        );
        for summary in ctx.changed_symbol_summaries.iter().take(10) {
            println!(
                "  {} {} ({}:{}) | callers {} | callees {} | importers {} | tests {}",
                summary.node.kind.as_str(),
                summary.node.qualified_name,
                summary.node.file_path,
                summary.node.line_start,
                summary.callers.len(),
                summary.callees.len(),
                summary.importers.len(),
                summary.tests.len()
            );
        }
        println!(
            "\nImpacted neighbors (top {}):",
            ctx.impacted_neighbors.len().min(20)
        );
        for n in ctx.impacted_neighbors.iter().take(20) {
            println!(
                "  {} {} ({}:{})",
                n.kind.as_str(),
                n.qualified_name,
                n.file_path,
                n.line_start
            );
        }
        println!("\nRisk summary:");
        println!(
            "  Public API changes : {}",
            ctx.risk_summary.public_api_changes
        );
        println!(
            "  Affected tests     : {}",
            ctx.risk_summary.affected_test_count
        );
        println!(
            "  Uncovered changes  : {}",
            ctx.risk_summary.uncovered_changed_symbol_count
        );
        println!(
            "  Large functions    : {}",
            ctx.risk_summary.large_function_count
        );
        println!("  Test adjacent      : {}", ctx.risk_summary.test_adjacent);
        println!(
            "  Cross-module impact: {}",
            ctx.risk_summary.cross_module_impact
        );
        println!(
            "  Cross-package impact: {}",
            ctx.risk_summary.cross_package_impact
        );
    }

    Ok(())
}

pub fn run_db_check(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let issues = store.integrity_check().context("integrity check failed")?;

    if cli.json {
        let result = serde_json::json!({
            "db_path": db_path,
            "ok": issues.is_empty(),
            "issues": issues,
        });
        print_json("db_check", result)?;
    } else if issues.is_empty() {
        println!("Database integrity OK: {db_path}");
    } else {
        eprintln!("Database integrity FAILED: {db_path}");
        for issue in &issues {
            eprintln!("  {issue}");
        }
        std::process::exit(1);
    }

    Ok(())
}

// --- helpers -----------------------------------------------------------------

fn resolve_repo(cli: &Cli) -> Result<String> {
    if let Some(r) = &cli.repo {
        return Ok(r.clone());
    }
    Ok(std::env::current_dir()
        .context("cannot determine cwd")?
        .to_string_lossy()
        .into_owned())
}

fn db_path(cli: &Cli, repo: &str) -> String {
    if let Some(p) = &cli.db {
        return p.clone();
    }
    crate::paths::default_db_path(repo)
}

// ---------------------------------------------------------------------------
// MCP / JSON-RPC serve
// ---------------------------------------------------------------------------

/// Delegate to the `atlas-mcp` crate's stdin/stdout server.
pub fn run_serve(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    atlas_mcp::run_server(&repo, &db_path)
}
