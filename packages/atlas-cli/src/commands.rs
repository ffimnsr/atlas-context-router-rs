use std::fs;

use anyhow::{Context, Result};
use atlas_core::SearchQuery;
use atlas_core::model::{
    ChangeType, ContextIntent, ContextRequest, ContextTarget, ImpactResult, ReviewContext,
    ReviewImpactOverview, RiskSummary,
};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_impact::analyze as advanced_impact;
use atlas_repo::{DiffTarget, changed_files, collect_files, find_repo_root, repo_relative};
use atlas_review::ContextEngine;
use atlas_search as search;
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use crate::cli::{Cli, Command};

const MACHINE_SCHEMA_VERSION: &str = "atlas_cli.v1";

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
    let atlas_dir = atlas_engine::paths::atlas_dir(&repo);
    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;

    let db_path = db_path(cli, &repo);
    Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let config_path = atlas_engine::paths::config_path(&repo);
    let config_created = atlas_engine::Config::write_default(&atlas_dir)
        .with_context(|| format!("cannot write config to {}", config_path.display()))?;

    if cli.json {
        print_json(
            "init",
            serde_json::json!({
                "atlas_dir": atlas_dir.display().to_string(),
                "db_path": db_path,
                "config_path": config_path.display().to_string(),
                "config_created": config_created,
            }),
        )?;
    } else {
        println!("Initialized atlas in {}", atlas_dir.display());
        println!("Database: {db_path}");
        if config_created {
            println!("Config  : {}", config_path.display());
        }
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
    let fail_fast = matches!(&cli.command, Command::Build { fail_fast } if *fail_fast);

    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let db_path = db_path(cli, &repo);

    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;

    let summary = build_graph(
        repo_root_path.as_path(),
        &db_path,
        &BuildOptions {
            fail_fast,
            batch_size: config.parse_batch_size(),
        },
    )?;

    if cli.json {
        print_json(
            "build",
            serde_json::json!({
                "scanned": summary.scanned,
                "skipped_unsupported": summary.skipped_unsupported,
                "skipped_unchanged": summary.skipped_unchanged,
                "parsed": summary.parsed,
                "parse_errors": summary.parse_errors,
                "nodes_inserted": summary.nodes_inserted,
                "edges_inserted": summary.edges_inserted,
                "elapsed_ms": summary.elapsed_ms,
            }),
        )?;
    } else {
        println!(
            "Build complete ({:.2}s)",
            summary.elapsed_ms as f64 / 1000.0
        );
        println!("  Scanned             : {}", summary.scanned);
        println!("  Unsupported skipped : {}", summary.skipped_unsupported);
        println!("  Unchanged skipped   : {}", summary.skipped_unchanged);
        println!("  Parsed              : {}", summary.parsed);
        if summary.parse_errors > 0 {
            println!("  Errors              : {}", summary.parse_errors);
        }
        println!("  Nodes inserted      : {}", summary.nodes_inserted);
        println!("  Edges inserted      : {}", summary.edges_inserted);
    }

    Ok(())
}

pub fn run_update(cli: &Cli) -> Result<()> {
    let fail_fast = matches!(
        &cli.command,
        Command::Update { fail_fast, .. } if *fail_fast
    );

    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let db_path = db_path(cli, &repo);

    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;

    let explicit_files: Vec<String> = match &cli.command {
        Command::Update { files, .. } => files.clone(),
        _ => vec![],
    };

    let target = if !explicit_files.is_empty() {
        UpdateTarget::Files(explicit_files)
    } else {
        match &cli.command {
            Command::Update { base, staged, .. } => {
                if *staged {
                    UpdateTarget::Staged
                } else if let Some(base_ref) = base {
                    UpdateTarget::BaseRef(base_ref.clone())
                } else {
                    UpdateTarget::WorkingTree
                }
            }
            _ => UpdateTarget::WorkingTree,
        }
    };

    let summary = update_graph(
        repo_root_path.as_path(),
        &db_path,
        &UpdateOptions {
            fail_fast,
            batch_size: config.parse_batch_size(),
            target,
        },
    )?;

    if cli.json {
        print_json(
            "update",
            serde_json::json!({
                "deleted": summary.deleted,
                "renamed": summary.renamed,
                "parsed": summary.parsed,
                "skipped_unsupported": summary.skipped_unsupported,
                "parse_errors": summary.parse_errors,
                "nodes_updated": summary.nodes_updated,
                "edges_updated": summary.edges_updated,
                "elapsed_ms": summary.elapsed_ms,
            }),
        )?;
    } else {
        println!(
            "Update complete ({:.2}s)",
            summary.elapsed_ms as f64 / 1000.0
        );
        println!("  Deleted  : {}", summary.deleted);
        if summary.renamed > 0 {
            println!("  Renamed  : {}", summary.renamed);
        }
        println!("  Parsed   : {}", summary.parsed);
        if summary.skipped_unsupported > 0 {
            println!("  Unsupported skipped : {}", summary.skipped_unsupported);
        }
        if summary.parse_errors > 0 {
            println!("  Errors   : {}", summary.parse_errors);
        }
        println!("  Nodes    : {}", summary.nodes_updated);
        println!("  Edges    : {}", summary.edges_updated);
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

    let (text, kind, language, subpath, limit, expand, expand_hops, hybrid) = match &cli.command {
        Command::Query {
            text,
            kind,
            language,
            subpath,
            limit,
            expand,
            expand_hops,
            hybrid,
        } => (
            text.clone(),
            kind.clone(),
            language.clone(),
            subpath.clone(),
            *limit,
            *expand,
            *expand_hops,
            *hybrid,
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
        hybrid,
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

pub fn run_embed(cli: &Cli) -> Result<()> {
    let limit = match &cli.command {
        Command::Embed { limit } => *limit,
        _ => unreachable!(),
    };

    let embed_cfg = atlas_search::embed::EmbeddingConfig::from_env()
        .context("ATLAS_EMBED_URL not set — cannot generate embeddings")?;

    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let chunks = store
        .chunks_missing_embeddings(limit)
        .context("failed to read chunks")?;

    if chunks.is_empty() {
        println!("All chunks already have embeddings.");
        return Ok(());
    }

    let total = chunks.len();
    let mut done = 0usize;
    let mut errors = 0usize;

    for (id, qn, text) in chunks {
        match atlas_search::embed::embed_text(&embed_cfg, &text) {
            Ok(vec) => {
                if let Err(err) = store.set_chunk_embedding(id, &vec) {
                    tracing::warn!("store embedding failed for {qn}: {err:#}");
                    errors += 1;
                } else {
                    done += 1;
                }
            }
            Err(err) => {
                tracing::warn!("embed failed for {qn}: {err:#}");
                errors += 1;
            }
        }
    }

    println!("Embedded {done}/{total} chunks ({errors} errors).");
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

    if cli.json {
        // JSON path: use context engine → ContextResult (new stable schema).
        let request = ContextRequest {
            intent: ContextIntent::Review,
            target: ContextTarget::ChangedFiles {
                paths: target_files.clone(),
            },
            max_nodes: Some(max_nodes),
            depth: Some(max_depth),
            ..ContextRequest::default()
        };
        let engine = ContextEngine::new(&store);
        let result = engine.build(&request).context("context engine failed")?;
        print_json("review_context", serde_json::to_value(&result)?)?;
        return Ok(());
    }

    let impact = store
        .impact_radius(&path_refs, max_depth, max_nodes)
        .context("impact radius query failed")?;

    let ctx = atlas_review::assemble_review_context(&impact, &target_files, max_depth, max_nodes);

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

    Ok(())
}

pub fn run_context(cli: &Cli) -> Result<()> {
    use atlas_core::model::SelectionReason;

    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let (
        qname,
        name,
        file,
        files,
        intent_str,
        max_nodes,
        max_edges,
        max_files,
        depth,
        code_spans,
        tests,
        imports,
        neighbors,
    ) = match &cli.command {
        Command::Context {
            qname,
            name,
            file,
            files,
            intent,
            max_nodes,
            max_edges,
            max_files,
            depth,
            code_spans,
            tests,
            imports,
            neighbors,
        } => (
            qname.clone(),
            name.clone(),
            file.clone(),
            files.clone(),
            intent.clone(),
            *max_nodes,
            *max_edges,
            *max_files,
            *depth,
            *code_spans,
            *tests,
            *imports,
            *neighbors,
        ),
        _ => unreachable!(),
    };

    let intent = match intent_str.as_str() {
        "review" => ContextIntent::Review,
        "impact" => ContextIntent::Impact,
        "file" => ContextIntent::File,
        _ => ContextIntent::Symbol,
    };

    let target = if !files.is_empty() {
        ContextTarget::ChangedFiles { paths: files }
    } else if let Some(qn) = qname {
        ContextTarget::QualifiedName { qname: qn }
    } else if let Some(n) = name {
        ContextTarget::SymbolName { name: n }
    } else if let Some(f) = file {
        ContextTarget::FilePath { path: f }
    } else {
        anyhow::bail!("one of --qname, --name, --file, or --files is required");
    };

    let request = ContextRequest {
        intent,
        target,
        max_nodes,
        max_edges,
        max_files,
        depth,
        include_code_spans: code_spans,
        include_tests: tests,
        include_imports: imports,
        include_neighbors: neighbors,
        ..ContextRequest::default()
    };

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let engine = ContextEngine::new(&store);
    let result = engine.build(&request).context("context engine failed")?;

    if cli.json {
        print_json("context", serde_json::to_value(&result)?)?;
    } else {
        if let Some(ambiguity) = &result.ambiguity {
            println!("Ambiguous target: {}", ambiguity.query);
            println!("Candidates ({}):", ambiguity.candidates.len());
            for c in &ambiguity.candidates {
                println!("  {c}");
            }
            return Ok(());
        }

        println!("Nodes ({}):", result.nodes.len());
        for sn in result.nodes.iter().take(20) {
            println!(
                "  [{:?}] {} {} ({}:{})",
                sn.selection_reason,
                sn.node.kind.as_str(),
                sn.node.qualified_name,
                sn.node.file_path,
                sn.node.line_start,
            );
        }
        println!("\nEdges ({}):", result.edges.len());
        for se in result.edges.iter().take(20) {
            println!(
                "  {} --{}--> {}",
                se.edge.source_qn,
                se.edge.kind.as_str(),
                se.edge.target_qn,
            );
        }
        println!("\nFiles ({}):", result.files.len());
        for sf in &result.files {
            let ranges: Vec<String> = sf
                .line_ranges
                .iter()
                .map(|(s, e)| format!("{s}-{e}"))
                .collect();
            if ranges.is_empty() {
                println!("  {} [{:?}]", sf.path, sf.selection_reason);
            } else {
                println!(
                    "  {} [{:?}] lines {}",
                    sf.path,
                    sf.selection_reason,
                    ranges.join(", ")
                );
            }
        }
        if result.truncation.truncated {
            println!(
                "\n[truncated: {} nodes, {} edges, {} files dropped]",
                result.truncation.nodes_dropped,
                result.truncation.edges_dropped,
                result.truncation.files_dropped,
            );
        }

        // Print counts for nodes tagged as DirectTarget on their own line
        let direct_count = result
            .nodes
            .iter()
            .filter(|n| n.selection_reason == SelectionReason::DirectTarget)
            .count();
        let caller_count = result
            .nodes
            .iter()
            .filter(|n| n.selection_reason == SelectionReason::Caller)
            .count();
        let callee_count = result
            .nodes
            .iter()
            .filter(|n| n.selection_reason == SelectionReason::Callee)
            .count();
        println!(
            "\nSummary: {} target, {} callers, {} callees",
            direct_count, caller_count, callee_count
        );
    }

    Ok(())
}

/// Structured result for a single doctor check.
struct CheckResult {
    name: &'static str,
    ok: bool,
    detail: String,
}

impl CheckResult {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
        }
    }
}

pub fn run_doctor(cli: &Cli) -> Result<()> {
    let mut checks: Vec<CheckResult> = Vec::new();

    // 1. Repo root
    let repo = match resolve_repo(cli) {
        Ok(r) => {
            checks.push(CheckResult::pass("repo_root", &r));
            r
        }
        Err(e) => {
            checks.push(CheckResult::fail("repo_root", e.to_string()));
            return print_doctor_report(cli, &checks, false);
        }
    };

    // 2. Git repo root detection
    match find_repo_root(Utf8Path::new(&repo)) {
        Ok(root) => checks.push(CheckResult::pass("git_root", root.as_str())),
        Err(e) => checks.push(CheckResult::fail("git_root", e.to_string())),
    }

    // 3. .atlas dir
    let atlas_dir = atlas_engine::paths::atlas_dir(&repo);
    if atlas_dir.exists() {
        checks.push(CheckResult::pass(
            "atlas_dir",
            atlas_dir.display().to_string(),
        ));
    } else {
        checks.push(CheckResult::fail(
            "atlas_dir",
            format!("{} not found — run `atlas init`", atlas_dir.display()),
        ));
    }

    // 4. Config file
    let config_path = atlas_engine::paths::config_path(&repo);
    if config_path.exists() {
        checks.push(CheckResult::pass(
            "config_file",
            config_path.display().to_string(),
        ));
    } else {
        checks.push(CheckResult::fail(
            "config_file",
            format!("{} not found — run `atlas init`", config_path.display()),
        ));
    }

    // 5. DB file exists
    let db_path_str = db_path(cli, &repo);
    let db_exists = std::path::Path::new(&db_path_str).exists();
    if db_exists {
        checks.push(CheckResult::pass("db_file", &db_path_str));
    } else {
        checks.push(CheckResult::fail(
            "db_file",
            format!("{db_path_str} not found — run `atlas init`"),
        ));
    }

    // 6. DB open + integrity + stats
    if db_exists {
        match Store::open(&db_path_str) {
            Ok(store) => {
                checks.push(CheckResult::pass("db_open", &db_path_str));
                match store.integrity_check() {
                    Ok(issues) if issues.is_empty() => {
                        checks.push(CheckResult::pass("db_integrity", "ok"));
                    }
                    Ok(issues) => {
                        checks.push(CheckResult::fail("db_integrity", issues.join("; ")));
                    }
                    Err(e) => {
                        checks.push(CheckResult::fail("db_integrity", e.to_string()));
                    }
                }
                match store.stats() {
                    Ok(stats) => {
                        checks.push(CheckResult::pass(
                            "graph_stats",
                            format!(
                                "files={} nodes={} edges={}",
                                stats.file_count, stats.node_count, stats.edge_count
                            ),
                        ));
                    }
                    Err(e) => {
                        checks.push(CheckResult::fail("graph_stats", e.to_string()));
                    }
                }
            }
            Err(e) => {
                checks.push(CheckResult::fail("db_open", e.to_string()));
            }
        }
    }

    // 7. git ls-files reachable
    match collect_files(Utf8Path::new(&repo), None) {
        Ok(files) => {
            checks.push(CheckResult::pass(
                "git_ls_files",
                format!("{} tracked files", files.len()),
            ));
        }
        Err(e) => {
            checks.push(CheckResult::fail("git_ls_files", e.to_string()));
        }
    }

    let all_ok = checks.iter().all(|c| c.ok);
    print_doctor_report(cli, &checks, all_ok)?;
    if !all_ok {
        std::process::exit(1);
    }
    Ok(())
}

fn print_doctor_report(cli: &Cli, checks: &[CheckResult], all_ok: bool) -> Result<()> {
    if cli.json {
        let items: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "check": c.name,
                    "ok": c.ok,
                    "detail": c.detail,
                })
            })
            .collect();
        print_json(
            "doctor",
            serde_json::json!({ "ok": all_ok, "checks": items }),
        )?;
    } else {
        for c in checks {
            let status = if c.ok { "PASS" } else { "FAIL" };
            println!("  [{status}] {}: {}", c.name, c.detail);
        }
        println!();
        if all_ok {
            println!("All checks passed.");
        } else {
            eprintln!("Some checks failed.");
        }
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
    atlas_engine::paths::default_db_path(repo)
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

// ---------------------------------------------------------------------------
// Install — MCP platform hooks
// ---------------------------------------------------------------------------

pub fn run_install(cli: &Cli) -> Result<()> {
    let (platform, dry_run, no_hooks, no_instructions) = match &cli.command {
        Command::Install {
            platform,
            dry_run,
            no_hooks,
            no_instructions,
        } => (platform.clone(), *dry_run, *no_hooks, *no_instructions),
        _ => unreachable!(),
    };

    let repo = resolve_repo(cli)?;
    let repo_root = std::path::Path::new(&repo);

    if dry_run {
        println!("Dry run — no files will be written.\n");
    }

    let summary =
        crate::install::run_install(repo_root, &platform, dry_run, no_hooks, no_instructions)?;

    if cli.json {
        print_json(
            "install",
            serde_json::json!({
                "dry_run": dry_run,
                "configured": summary.configured,
                "already_configured": summary.already_configured,
                "hook_paths": summary.hook_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "instruction_files": summary.instruction_files,
            }),
        )?;
    } else {
        for name in &summary.configured {
            println!("  Configured : {name}");
        }
        for name in &summary.already_configured {
            println!("  Skipped    : {name} (already configured)");
        }
        for hook in &summary.hook_paths {
            println!("  Git hook   : {}", hook.display());
        }
        for f in &summary.instruction_files {
            println!("  Instructions updated: {f}");
        }

        let total = summary.configured.len() + summary.already_configured.len();
        if total == 0 {
            println!("No platforms detected. Use --platform to target one explicitly.");
        } else if !dry_run {
            println!("\nDone. Restart your AI coding tool to pick up the new config.");
            println!("Run `atlas build` to build the knowledge graph.");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Completions — shell completion script generation
// ---------------------------------------------------------------------------

pub fn run_completions(cli: &Cli) -> Result<()> {
    use clap::CommandFactory;
    use clap_complete::generate;

    let shell = match &cli.command {
        Command::Completions { shell } => *shell,
        _ => unreachable!(),
    };

    let mut cmd = crate::cli::Cli::command();
    generate(shell, &mut cmd, "atlas", &mut std::io::stdout());
    Ok(())
}

// ---------------------------------------------------------------------------
// Analyze — autonomous code reasoning (Phase 25)
// ---------------------------------------------------------------------------

pub fn run_analyze(cli: &Cli) -> Result<()> {
    use atlas_reasoning::ReasoningEngine;

    use crate::cli::AnalyzeCommand;

    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let engine = ReasoningEngine::new(&store);

    let sub = match &cli.command {
        Command::Analyze { subcommand } => subcommand,
        _ => unreachable!(),
    };

    match sub {
        AnalyzeCommand::Remove {
            symbol,
            max_depth,
            max_nodes,
        } => {
            let result = engine
                .analyze_removal(&[symbol.as_str()], Some(*max_depth), Some(*max_nodes))
                .with_context(|| format!("removal analysis for `{symbol}` failed"))?;

            if cli.json {
                print_json("analyze_remove", serde_json::to_value(&result)?)?;
            } else {
                println!("Removal impact for: {symbol}");
                println!("  Seed nodes      : {}", result.seed.len());
                println!("  Impacted symbols: {}", result.impacted_symbols.len());
                println!("  Impacted files  : {}", result.impacted_files.len());
                println!("  Impacted tests  : {}", result.impacted_tests.len());
                for im in result.impacted_symbols.iter().take(20) {
                    println!(
                        "  [{:?}] {} {} (depth {})",
                        im.impact_class,
                        im.node.kind.as_str(),
                        im.node.qualified_name,
                        im.depth,
                    );
                }
                if !result.uncertainty_flags.is_empty() {
                    println!("\nUncertainty:");
                    for flag in &result.uncertainty_flags {
                        println!("  ! {flag}");
                    }
                }
                if !result.warnings.is_empty() {
                    println!("\nWarnings:");
                    for w in &result.warnings {
                        println!("  [{:?}] {}", w.confidence, w.message);
                    }
                }
            }
        }

        AnalyzeCommand::DeadCode { allowlist, limit } => {
            let allowlist_refs: Vec<&str> = allowlist.iter().map(String::as_str).collect();
            let candidates = engine
                .detect_dead_code(&allowlist_refs, Some(*limit))
                .context("dead-code detection failed")?;

            if cli.json {
                print_json("analyze_dead_code", serde_json::to_value(&candidates)?)?;
            } else if candidates.is_empty() {
                println!("No dead-code candidates found.");
            } else {
                println!("Dead-code candidates ({}):", candidates.len());
                for c in &candidates {
                    println!(
                        "  [{:?}] {} {} ({}:{})",
                        c.certainty,
                        c.node.kind.as_str(),
                        c.node.qualified_name,
                        c.node.file_path,
                        c.node.line_start,
                    );
                    for r in &c.reasons {
                        println!("    - {r}");
                    }
                    for b in &c.blockers {
                        println!("    ! blocker: {b}");
                    }
                }
            }
        }

        AnalyzeCommand::Safety { symbol } => {
            let result = engine
                .score_refactor_safety(symbol)
                .with_context(|| format!("safety scoring for `{symbol}` failed"))?;

            if cli.json {
                print_json("analyze_safety", serde_json::to_value(&result)?)?;
            } else {
                println!("Refactor safety for: {symbol}");
                println!("  Score  : {:.3}", result.safety.score);
                println!("  Band   : {:?}", result.safety.band);
                println!("  Fan-in : {}", result.fan_in);
                println!("  Fan-out: {}", result.fan_out);
                println!("  Tests  : {}", result.linked_test_count);
                if !result.safety.reasons.is_empty() {
                    println!("\nReasons:");
                    for r in &result.safety.reasons {
                        println!("  - {r}");
                    }
                }
                if !result.safety.suggested_validations.is_empty() {
                    println!("\nSuggested validations:");
                    for v in &result.safety.suggested_validations {
                        println!("  - {v}");
                    }
                }
            }
        }

        AnalyzeCommand::Dependency { symbol } => {
            let result = engine
                .check_dependency_removal(symbol)
                .with_context(|| format!("dependency check for `{symbol}` failed"))?;

            if cli.json {
                print_json("analyze_dependency", serde_json::to_value(&result)?)?;
            } else {
                let verdict = if result.removable {
                    "REMOVABLE"
                } else {
                    "BLOCKED"
                };
                println!("Dependency check for: {symbol}");
                println!("  Verdict   : {verdict}");
                println!("  Confidence: {:?}", result.confidence);
                println!("  Blocking  : {}", result.blocking_references.len());
                for n in &result.blocking_references {
                    println!(
                        "  - {} {} ({})",
                        n.kind.as_str(),
                        n.qualified_name,
                        n.file_path
                    );
                }
                if !result.suggested_cleanups.is_empty() {
                    println!("\nSuggested cleanups:");
                    for s in &result.suggested_cleanups {
                        println!("  - {s}");
                    }
                }
                if !result.uncertainty_flags.is_empty() {
                    println!("\nUncertainty:");
                    for flag in &result.uncertainty_flags {
                        println!("  ! {flag}");
                    }
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Refactor — deterministic transforms (Phase 25)
// ---------------------------------------------------------------------------

pub fn run_refactor(cli: &Cli) -> Result<()> {
    use atlas_refactor::RefactorEngine;

    use crate::cli::RefactorCommand;

    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_std_path();
    let db_path = db_path(cli, &repo);
    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let engine = RefactorEngine::new(&store, repo_root);

    let sub = match &cli.command {
        Command::Refactor { subcommand } => subcommand,
        _ => unreachable!(),
    };

    match sub {
        RefactorCommand::Rename {
            symbol,
            new_name,
            dry_run,
        } => {
            let plan = engine
                .plan_rename(symbol, new_name)
                .with_context(|| format!("rename plan for `{symbol}` → `{new_name}` failed"))?;
            let result = engine
                .apply_rename(&plan, *dry_run)
                .context("apply rename failed")?;

            if cli.json {
                print_json("refactor_rename", serde_json::to_value(&result)?)?;
            } else {
                print_refactor_result(&result, *dry_run);
            }
        }

        RefactorCommand::RemoveDead { symbol, dry_run } => {
            let plan = engine
                .plan_dead_code_removal(symbol)
                .with_context(|| format!("remove-dead plan for `{symbol}` failed"))?;
            let result = engine
                .apply_dead_code_removal(&plan, *dry_run)
                .context("apply dead-code removal failed")?;

            if cli.json {
                print_json("refactor_remove_dead", serde_json::to_value(&result)?)?;
            } else {
                print_refactor_result(&result, *dry_run);
            }
        }

        RefactorCommand::CleanImports { file, dry_run } => {
            let plan = engine
                .plan_import_cleanup(file)
                .with_context(|| format!("import-cleanup plan for `{file}` failed"))?;
            let result = engine
                .apply_import_cleanup(&plan, *dry_run)
                .context("apply import cleanup failed")?;

            if cli.json {
                print_json("refactor_clean_imports", serde_json::to_value(&result)?)?;
            } else {
                print_refactor_result(&result, *dry_run);
            }
        }
    }

    Ok(())
}

fn print_refactor_result(result: &atlas_core::RefactorDryRunResult, dry_run: bool) {
    let mode = if dry_run { "dry-run" } else { "applied" };
    println!("Refactor ({mode}):");
    println!("  Files changed : {}", result.files_changed);
    println!("  Edits         : {}", result.edit_count);
    println!("  Safety        : {:?}", result.plan.estimated_safety);
    if !result.plan.manual_review.is_empty() {
        println!("\nManual review required:");
        for item in &result.plan.manual_review {
            println!("  ! {item}");
        }
    }
    if !result.validation.warnings.is_empty() {
        println!("\nValidation warnings:");
        for w in &result.validation.warnings {
            println!("  ~ {w}");
        }
    }
    if !result.patches.is_empty() {
        println!("\nPatches:");
        for p in &result.patches {
            println!("--- {}", p.file_path);
            println!("{}", p.unified_diff);
        }
    }
}
