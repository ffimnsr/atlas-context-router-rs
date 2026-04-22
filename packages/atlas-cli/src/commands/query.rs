use anyhow::{Context, Result};
use atlas_core::SearchQuery;
use atlas_search as search;
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command};

use super::{db_path, print_json, query_display_path, resolve_repo};

fn query_owner_identity(node: &atlas_core::Node) -> Option<String> {
    node.extra_json.as_object().and_then(|extra| {
        extra
            .get("owner_id")
            .or_else(|| extra.get("workspace_id"))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    })
}

pub fn run_query(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let (
        text,
        kind,
        language,
        include_files,
        subpath,
        limit,
        expand,
        expand_hops,
        fuzzy,
        hybrid,
        semantic,
        regex,
    ) = match &cli.command {
        Command::Query {
            text,
            kind,
            language,
            include_files,
            subpath,
            limit,
            expand,
            expand_hops,
            fuzzy,
            hybrid,
            semantic,
            regex,
        } => (
            text.clone(),
            kind.clone(),
            language.clone(),
            *include_files,
            subpath.clone(),
            *limit,
            *expand,
            *expand_hops,
            *fuzzy,
            *hybrid,
            *semantic,
            *regex,
        ),
        _ => unreachable!(),
    };

    if text.trim().is_empty() {
        if regex {
            anyhow::bail!("regex pattern required when --regex is set");
        } else {
            anyhow::bail!("search text required; use --regex to treat text as a regex pattern");
        }
    }

    let (effective_text, regex_pattern) = if regex {
        (String::new(), Some(text.clone()))
    } else {
        (text.clone(), None)
    };

    let query = SearchQuery {
        text: effective_text,
        kind,
        language,
        include_files,
        subpath,
        limit,
        graph_expand: expand,
        graph_max_hops: expand_hops,
        fuzzy_match: fuzzy,
        hybrid,
        regex_pattern,
        ..Default::default()
    };

    let t0 = std::time::Instant::now();
    let results = if semantic {
        search::semantic::expanded_search(&store, &query).context("semantic search failed")?
    } else {
        search::search(&store, &query).context("search failed")?
    };
    let latency_ms = t0.elapsed().as_millis();

    if cli.json {
        print_json(
            "query",
            serde_json::json!({
                "query": {
                    "text": query.text,
                    "kind": query.kind,
                    "language": query.language,
                    "include_files": query.include_files,
                    "subpath": query.subpath,
                    "limit": query.limit,
                    "graph_expand": query.graph_expand,
                    "graph_max_hops": query.graph_max_hops,
                    "fuzzy_match": query.fuzzy_match,
                    "semantic": semantic,
                    "regex_pattern": query.regex_pattern,
                },
                "latency_ms": latency_ms,
                "results": results,
            }),
        )?;
    } else if results.is_empty() {
        println!("No results. ({latency_ms}ms)");
    } else {
        for r in &results {
            let n = &r.node;
            let display_path = query_display_path(n);
            println!(
                "[{:.3}] {} {} ({}:{}){}",
                r.score,
                n.kind.as_str(),
                n.qualified_name,
                display_path,
                n.line_start,
                query_owner_identity(n)
                    .map(|owner| format!(" [owner {owner}]"))
                    .unwrap_or_default(),
            );
        }
        println!("\n{} result(s). ({latency_ms}ms)", results.len());
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

pub fn run_explain_query(cli: &Cli) -> Result<()> {
    let (text, kind, language, subpath, limit) = match &cli.command {
        Command::ExplainQuery {
            text,
            kind,
            language,
            subpath,
            limit,
        } => (
            text.clone(),
            kind.clone(),
            language.clone(),
            subpath.clone(),
            *limit,
        ),
        _ => unreachable!(),
    };

    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let query = SearchQuery {
        text: text.clone(),
        kind: kind.clone(),
        language: language.clone(),
        subpath: subpath.clone(),
        limit,
        ..Default::default()
    };

    let t0 = std::time::Instant::now();
    let results = search::search(&store, &query).context("search failed")?;
    let latency_ms = t0.elapsed().as_millis();

    let matches: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "score": r.score,
                "kind": r.node.kind.as_str(),
                "qualified_name": r.node.qualified_name,
                "file_path": r.node.file_path,
                "line_start": r.node.line_start,
                "language": r.node.language,
            })
        })
        .collect();

    if cli.json {
        print_json(
            "explain_query",
            serde_json::json!({
                "query": {
                    "text": text,
                    "kind": kind,
                    "language": language,
                    "subpath": subpath,
                    "limit": limit,
                },
                "latency_ms": latency_ms,
                "result_count": results.len(),
                "matches": matches,
            }),
        )?;
    } else {
        println!("Query explanation");
        println!("  Text     : {text}");
        if let Some(k) = &kind {
            println!("  Kind     : {k}");
        }
        if let Some(l) = &language {
            println!("  Language : {l}");
        }
        if let Some(s) = &subpath {
            println!("  Subpath  : {s}");
        }
        println!("  Limit    : {limit}");
        println!("  Latency  : {latency_ms}ms");
        println!("  Results  : {}", results.len());
        if results.is_empty() {
            println!("\nNo matches.");
        } else {
            println!("\nMatches (score / kind / qualified_name):");
            for r in &results {
                println!(
                    "  [{:.4}] {} {} @ {}:{}",
                    r.score,
                    r.node.kind.as_str(),
                    r.node.qualified_name,
                    r.node.file_path,
                    r.node.line_start,
                );
            }
        }
    }

    Ok(())
}
