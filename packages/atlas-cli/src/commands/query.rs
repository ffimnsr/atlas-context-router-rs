use anyhow::{Context, Result};
use atlas_core::{BudgetManager, RankingEvidence, SearchQuery};
use atlas_search as search;
use atlas_search::QueryExplanation;
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command};

use super::{db_path, load_budget_policy, print_json, query_display_path, resolve_repo};

fn query_owner_identity(node: &atlas_core::Node) -> Option<String> {
    node.extra_json.as_object().and_then(|extra| {
        extra
            .get("owner_id")
            .or_else(|| extra.get("workspace_id"))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    })
}

fn ranking_evidence_labels(evidence: &RankingEvidence) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if evidence.exact_name_match {
        labels.push("exact_name");
    }
    if evidence.exact_qualified_name_match {
        labels.push("exact_qname");
    }
    if evidence.prefix_match {
        labels.push("prefix");
    }
    if evidence.fuzzy.is_some() {
        labels.push("fuzzy");
    }
    if evidence.kind_boost.is_some() {
        labels.push("kind_boost");
    }
    if evidence.public_exported_boost.is_some() {
        labels.push("public_api");
    }
    if evidence.same_directory_boost.is_some() {
        labels.push("same_directory");
    }
    if evidence.same_language_boost.is_some() {
        labels.push("same_language");
    }
    if evidence.recent_file_boost.is_some() {
        labels.push("recent_file");
    }
    if evidence.changed_file_boost.is_some() {
        labels.push("changed_file");
    }
    if evidence.graph_expansion.is_some() {
        labels.push("graph_expand");
    }
    if evidence.hybrid_rrf.is_some() {
        labels.push("hybrid_rrf");
    }
    labels
}

pub fn run_query(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let policy = load_budget_policy(&repo)?;

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

    let mut budgets = BudgetManager::new();
    let effective_limit = budgets.resolve_limit(
        policy.query_candidates_and_seeds.candidates,
        "query_candidates_and_seeds.max_candidates",
        Some(limit),
    );

    let query = SearchQuery {
        text: effective_text,
        kind,
        language,
        include_files,
        subpath,
        limit: effective_limit,
        graph_expand: expand,
        graph_max_hops: expand_hops,
        fuzzy_match: fuzzy,
        hybrid,
        regex_pattern,
        ..Default::default()
    };

    let t0 = std::time::Instant::now();
    let results = search::execute_query(&store, &query, semantic).context("search failed")?;
    let latency_ms = t0.elapsed().as_millis();
    budgets.record_usage(
        policy.query_candidates_and_seeds.wall_time_ms,
        "query_candidates_and_seeds.max_query_wall_time_ms",
        policy.query_candidates_and_seeds.wall_time_ms.default_limit,
        latency_ms as usize,
        latency_ms as usize > policy.query_candidates_and_seeds.wall_time_ms.default_limit,
    );
    let budget = budgets.summary(
        "query_candidates_and_seeds.max_candidates",
        effective_limit,
        results.len(),
    );

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
                "results": results,
                "ranking_evidence_legend": atlas_core::ranking_evidence_legend(),
                "budget": budget,
            }),
        )?;
    } else if results.is_empty() {
        println!("No results. ({latency_ms}ms)");
    } else {
        for r in &results {
            let n = &r.node;
            let display_path = query_display_path(n);
            let labels = r
                .ranking_evidence
                .as_ref()
                .map(ranking_evidence_labels)
                .unwrap_or_default();
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
            if cli.verbose && !labels.is_empty() {
                println!("        evidence: {}", labels.join(", "));
            }
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

    let explanation = search::explain_query(Some(&store), true, &query, false);

    if cli.json {
        print_json("explain_query", serde_json::to_value(&explanation)?)?;
    } else {
        let QueryExplanation {
            input,
            latency_ms,
            result_count,
            matches,
            ..
        } = explanation;
        println!("Query explanation");
        println!("  Text     : {}", input.text);
        if let Some(k) = &input.kind {
            println!("  Kind     : {k}");
        }
        if let Some(l) = &input.language {
            println!("  Language : {l}");
        }
        if let Some(s) = &input.subpath {
            println!("  Subpath  : {s}");
        }
        println!("  Limit    : {}", input.limit);
        println!("  Latency  : {}ms", latency_ms.unwrap_or_default());
        println!("  Results  : {}", result_count.unwrap_or_default());
        if matches.as_ref().is_none_or(|items| items.is_empty()) {
            println!("\nNo matches.");
        } else {
            println!("\nMatches (score / kind / qualified_name):");
            for item in matches.as_ref().into_iter().flatten() {
                println!(
                    "  [{:.4}] {} {} @ {}:{}",
                    item.score, item.kind, item.qualified_name, item.file_path, item.line_start,
                );
            }
        }
    }

    Ok(())
}
