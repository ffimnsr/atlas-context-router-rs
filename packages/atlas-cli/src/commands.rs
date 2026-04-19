use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use atlas_core::SearchQuery;
use atlas_core::model::{ChangeType, ImpactResult, ParsedFile, ReviewContext, RiskSummary};
use atlas_parser::ParserRegistry;
use atlas_repo::{
    DiffTarget, changed_files, collect_files, find_repo_root, hash_file, repo_relative,
};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use rayon::prelude::*;

use crate::cli::{Cli, Command};

/// Parse-worker batch size: number of files sent to rayon in one chunk.
/// Keeps memory bounded — only this many parsed files reside in memory at once
/// before being handed to the SQLite writer.
const PARSE_BATCH_SIZE: usize = 64;

pub fn run_init(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let atlas_dir = crate::paths::atlas_dir(&repo);
    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;

    let db_path = db_path(cli, &repo);
    Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    println!("Initialized atlas in {}", atlas_dir.display());
    println!("Database: {db_path}");
    Ok(())
}

pub fn run_status(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let stats = store.stats().context("cannot read stats")?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
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

    let all_files = collect_files(repo_root, None).context("cannot collect tracked files")?;

    let mut scanned = 0usize;
    let mut skipped_unsupported = 0usize;
    let mut skipped_unchanged = 0usize;
    let mut parse_errors = 0usize;

    // Candidates: (rel_path_string, abs_path, hash)
    type Candidate = (String, camino::Utf8PathBuf, String);
    let mut candidates: Vec<Candidate> = Vec::new();

    for rel_path in &all_files {
        scanned += 1;
        let rel_str = rel_path.as_str().to_owned();

        if !registry.supports(&rel_str) {
            skipped_unsupported += 1;
            continue;
        }

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

    // --- Parallel parse in bounded chunks, sequential DB write ---------------
    let _parse_span = tracing::info_span!("build.parse_and_write").entered();

    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;

    for chunk in candidates.chunks(PARSE_BATCH_SIZE) {
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

        // Write sequentially from the parsed results.
        for (rel_str, outcome) in results {
            match outcome {
                Ok(pf) => {
                    total_nodes += pf.nodes.len();
                    total_edges += pf.edges.len();
                    store.replace_file_graph(
                        &pf.path,
                        &pf.hash,
                        pf.language.as_deref(),
                        pf.size,
                        &pf.nodes,
                        &pf.edges,
                    ).with_context(|| format!("cannot store '{rel_str}'"))?;
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
    }

    drop(_parse_span);

    let parsed_count = candidates.len().saturating_sub(parse_errors).saturating_sub(skipped_unsupported);
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
        println!("{}", serde_json::to_string_pretty(&summary)?);
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

    let _deps_span = tracing::info_span!("update.find_dependents").entered();

    // Find files that depend on any of the changed files.
    let changed_ref_strs: Vec<&str> = to_parse_paths.iter().map(String::as_str).collect();
    let dependents = store
        .find_dependents(&changed_ref_strs)
        .context("cannot query dependents")?;

    drop(_deps_span);

    // Merge + deduplicate.
    let mut all_to_parse: Vec<String> = to_parse_paths.clone();
    for dep in dependents {
        if !all_to_parse.contains(&dep) {
            all_to_parse.push(dep);
        }
    }

    // Remove stale graphs first.
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

    // Candidates: (rel_str, abs_path)  — no hash pre-check in update path.
    type UpdateCandidate = (String, camino::Utf8PathBuf);
    let candidates: Vec<UpdateCandidate> = all_to_parse
        .iter()
        .filter_map(|rel_str| {
            if !registry.supports(rel_str) {
                return None; // counted below
            }
            let abs_path = repo_root.join(rel_str);
            Some((rel_str.clone(), abs_path))
        })
        .collect();

    skipped_unsupported += all_to_parse.len().saturating_sub(candidates.len());

    // --- Parallel parse in bounded chunks, sequential DB write ---------------
    let _parse_span = tracing::info_span!("update.parse_and_write").entered();

    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;

    for chunk in candidates.chunks(PARSE_BATCH_SIZE) {
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
                Ok(pf) => {
                    total_nodes += pf.nodes.len();
                    total_edges += pf.edges.len();
                    store.replace_file_graph(
                        &pf.path,
                        &pf.hash,
                        pf.language.as_deref(),
                        pf.size,
                        &pf.nodes,
                        &pf.edges,
                    ).with_context(|| format!("cannot store '{rel_str}'"))?;
                }
                Err(msg) if msg == "unsupported (skipped)" => {
                    skipped_unsupported += 1;
                }
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

    let parsed_count = candidates.len().saturating_sub(parse_errors).saturating_sub(skipped_unsupported);
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
        println!("{}", serde_json::to_string_pretty(&summary)?);
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

    let diff_target = match &cli.command {
        Command::DetectChanges { base, staged } => {
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

    let changes = changed_files(repo_root, &diff_target).context("cannot detect changed files")?;

    // Try to open the DB for graph summary — tolerate failure (DB may not exist yet).
    let store_result = Store::open(&db_path);

    if cli.json {
        // Augment each entry with a graph node count if the DB is available.
        let augmented: Vec<serde_json::Value> = changes
            .iter()
            .map(|cf| {
                let node_count = store_result
                    .as_ref()
                    .ok()
                    .and_then(|s| s.nodes_by_file(&cf.path).ok())
                    .map(|ns| ns.len());
                let mut v = serde_json::to_value(cf).unwrap_or_default();
                if let Some(c) = node_count {
                    v["node_count"] = serde_json::json!(c);
                }
                v
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&augmented)?);
    } else if changes.is_empty() {
        println!("No changed files detected.");
    } else {
        for cf in &changes {
            let tag = match cf.change_type {
                ChangeType::Added => "A",
                ChangeType::Modified => "M",
                ChangeType::Deleted => "D",
                ChangeType::Renamed => "R",
                ChangeType::Copied => "C",
            };
            let node_info = store_result
                .as_ref()
                .ok()
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| format!(" [{} nodes]", ns.len()))
                .unwrap_or_default();
            if let Some(old) = &cf.old_path {
                println!("{tag}  {old} -> {}{node_info}", cf.path);
            } else {
                println!("{tag}  {}{node_info}", cf.path);
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

    let (text, kind, language, limit) = match &cli.command {
        Command::Query {
            text,
            kind,
            language,
            limit,
        } => (text.clone(), kind.clone(), language.clone(), *limit),
        _ => unreachable!(),
    };

    let query = SearchQuery {
        text,
        kind,
        language,
        limit,
        ..Default::default()
    };

    let results = store.search(&query).context("search failed")?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&results)?);
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
            println!(
                "{}",
                serde_json::to_string_pretty(&ImpactResult {
                    changed_nodes: vec![],
                    impacted_nodes: vec![],
                    impacted_files: vec![],
                    relevant_edges: vec![],
                })?
            );
        } else {
            println!("No changed files detected.");
        }
        return Ok(());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(&path_refs, max_depth, max_nodes)
        .context("impact radius query failed")?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Changed files : {}", target_files.len());
        println!("Changed nodes : {}", result.changed_nodes.len());
        println!("Impacted nodes: {}", result.impacted_nodes.len());
        println!("Impacted files: {}", result.impacted_files.len());
        println!("Relevant edges: {}", result.relevant_edges.len());
        if !result.impacted_files.is_empty() {
            println!("\nImpacted files:");
            for f in &result.impacted_files {
                println!("  {f}");
            }
        }
        if !result.impacted_nodes.is_empty() {
            println!("\nImpacted nodes (top 20):");
            for n in result.impacted_nodes.iter().take(20) {
                println!(
                    "  {} {} ({}:{})",
                    n.kind.as_str(),
                    n.qualified_name,
                    n.file_path,
                    n.line_start
                );
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

    let (base, explicit_files) = match &cli.command {
        Command::ReviewContext { base, files } => (base.clone(), files.clone()),
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
                impacted_neighbors: vec![],
                critical_edges: vec![],
                risk_summary: RiskSummary {
                    changed_symbol_count: 0,
                    public_api_changes: 0,
                    test_adjacent: false,
                    cross_module_impact: false,
                },
            };
            println!("{}", serde_json::to_string_pretty(&empty)?);
        } else {
            println!("No changed files detected.");
        }
        return Ok(());
    }

    let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
    let impact = store
        .impact_radius(&path_refs, 3, 200)
        .context("impact radius query failed")?;

    let ctx = atlas_review::assemble_review_context(&impact, &target_files);

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&ctx)?);
    } else {
        println!("Changed files ({}):", ctx.changed_files.len());
        for f in &ctx.changed_files {
            println!("  {f}");
        }
        println!(
            "\nChanged symbols: {}",
            ctx.risk_summary.changed_symbol_count
        );
        for n in ctx.changed_symbols.iter().take(20) {
            println!(
                "  {} {} ({}:{})",
                n.kind.as_str(),
                n.qualified_name,
                n.file_path,
                n.line_start
            );
        }
        println!(
            "\nImpacted neighbors (top {}):",
            ctx.impacted_neighbors.len()
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
        println!("  Test adjacent      : {}", ctx.risk_summary.test_adjacent);
        println!(
            "  Cross-module impact: {}",
            ctx.risk_summary.cross_module_impact
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
        println!("{}", serde_json::to_string_pretty(&result)?);
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
