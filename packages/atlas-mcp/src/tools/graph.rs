use anyhow::{Context, Result};
use atlas_core::SearchQuery;
use atlas_core::model::ContextTarget;
use atlas_review::{ResolvedTarget, normalize_qn_kind_tokens, resolve_target};
use atlas_search::search as fts_search;
use atlas_search::semantic as sem;
use serde::Serialize;

use crate::context::{compact_node, package_impact};

use super::shared::{
    bool_arg, error_message, error_suggestions, open_store, resolve_kind_alias, str_arg,
    string_array_arg, tool_result_value, u64_arg,
};

pub(super) fn tool_list_graph_stats(
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let store = open_store(db_path)?;
    let stats = store.stats().context("stats query failed")?;
    tool_result_value(&stats, output_format)
}

pub(super) fn tool_query_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let text = str_arg(args, "text")?
        .map(str::to_owned)
        .unwrap_or_default();
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let limit = u64_arg(args, "limit").unwrap_or(20) as usize;
    let semantic = bool_arg(args, "semantic").unwrap_or(false);
    let expand = bool_arg(args, "expand").unwrap_or(false);
    let expand_hops = u64_arg(args, "expand_hops").unwrap_or(1) as u32;
    let regex = str_arg(args, "regex")?.map(str::to_owned);
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let hybrid = bool_arg(args, "hybrid").unwrap_or(false);
    let include_files = bool_arg(args, "include_files").unwrap_or(false);

    if text.trim().is_empty() && regex.is_none() {
        anyhow::bail!("query_graph requires non-empty text or a regex pattern");
    }
    if let Some(ref pat) = regex {
        if pat.trim().is_empty() {
            anyhow::bail!("regex pattern must not be empty");
        }
        regex::Regex::new(pat).map_err(|e| anyhow::anyhow!("invalid regex pattern: {e}"))?;
    }

    let store = open_store(db_path)?;
    let query = SearchQuery {
        text,
        kind,
        language,
        include_files,
        limit,
        subpath,
        graph_expand: expand,
        graph_max_hops: expand_hops,
        regex_pattern: regex,
        fuzzy_match: fuzzy,
        hybrid,
        ..Default::default()
    };

    let results = if semantic {
        sem::expanded_search(&store, &query).context("semantic search failed")?
    } else if fuzzy || hybrid {
        fts_search(&store, &query).context("search failed")?
    } else {
        store.search(&query).context("search failed")?
    };

    let active_query_mode = match (
        query.text.trim().is_empty(),
        query.regex_pattern.is_some(),
        semantic,
        hybrid,
    ) {
        (true, true, _, _) => "regex_structural_scan",
        (false, false, false, true) => "fts5_vector_hybrid",
        (false, false, true, _) => "fts5_graph_expand",
        (false, true, false, _) => "fts5_regex_filter",
        (false, true, true, _) => "fts5_regex_filter_graph_expand",
        _ => "fts5",
    };

    #[derive(Serialize)]
    struct CompactResult<'a> {
        score: f64,
        #[serde(flatten)]
        node: crate::context::CompactNode<'a>,
    }

    let compact: Vec<CompactResult<'_>> = results
        .iter()
        .map(|r| CompactResult {
            score: (r.score * 1000.0).round() / 1000.0,
            node: compact_node(&r.node),
        })
        .collect();

    let mut response = tool_result_value(&compact, output_format)?;
    response["atlas_result_kind"] = serde_json::Value::String("symbol_search".to_owned());
    response["atlas_usage_edges_included"] = serde_json::Value::Bool(false);
    response["atlas_relationship_tools"] =
        serde_json::json!(["symbol_neighbors", "traverse_graph", "get_context"]);
    response["atlas_result_count"] = serde_json::json!(compact.len());
    response["atlas_result_files"] = serde_json::json!(
        compact
            .iter()
            .map(|result| result.node.file.to_owned())
            .collect::<Vec<_>>()
    );
    response["atlas_truncated"] = serde_json::json!(compact.len() == limit);
    response["atlas_query_mode"] = serde_json::Value::String(active_query_mode.to_owned());
    if compact.is_empty() && semantic {
        response["atlas_hint"] = serde_json::Value::String(
            "FTS found no symbol names matching the query text. \
             FTS searches indexed identifiers, not natural language phrases. \
             Try: (1) a short exact symbol name like 'BalancesTab'; \
             (2) the regex param for pattern matching (e.g. regex='Balance'); \
             (3) get_context with a file path; \
             (4) list_graph_stats to confirm the graph has been built."
                .to_owned(),
        );
    }
    Ok(response)
}

pub(super) fn tool_batch_query_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    const MAX_QUERIES: usize = 20;

    let text_phrase = str_arg(args, "text")?.filter(|s| !s.trim().is_empty());
    let synthesized: Vec<serde_json::Value>;
    let queries_val: &[serde_json::Value] = if let Some(phrase) = text_phrase {
        synthesized = phrase
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|tok| !tok.is_empty())
            .map(|tok| serde_json::json!({ "text": tok }))
            .collect();
        &synthesized
    } else {
        let arr = args
            .and_then(|a| a.get("queries"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "batch_query_graph requires either a 'text' string \
                     (space-separated tokens) or a non-empty 'queries' array"
                )
            })?;
        arr.as_slice()
    };

    if queries_val.is_empty() {
        anyhow::bail!(
            "batch_query_graph requires either a 'text' string \
             (space-separated tokens) or a non-empty 'queries' array"
        );
    }
    if queries_val.len() > MAX_QUERIES {
        anyhow::bail!(
            "batch_query_graph exceeds the maximum of {MAX_QUERIES} queries per call; \
             split into smaller batches"
        );
    }

    let store = open_store(db_path)?;

    #[derive(Serialize)]
    struct BatchItem {
        query_index: usize,
        text: String,
        items: Vec<BatchResultNode>,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    #[derive(Serialize)]
    struct BatchResultNode {
        score: f64,
        name: String,
        qualified_name: String,
        kind: String,
        file_path: String,
        line_start: u32,
        language: String,
    }

    let mut batch_results: Vec<BatchItem> = Vec::with_capacity(queries_val.len());

    for (idx, q) in queries_val.iter().enumerate() {
        let q_args = Some(q);
        let text = str_arg(q_args, "text")?
            .map(str::to_owned)
            .unwrap_or_default();
        let kind = str_arg(q_args, "kind")?.map(str::to_owned);
        let language = str_arg(q_args, "language")?.map(str::to_owned);
        let limit = u64_arg(q_args, "limit").unwrap_or(20) as usize;
        let semantic = bool_arg(q_args, "semantic").unwrap_or(false);
        let expand = bool_arg(q_args, "expand").unwrap_or(false);
        let expand_hops = u64_arg(q_args, "expand_hops").unwrap_or(1) as u32;
        let regex = str_arg(q_args, "regex")?.map(str::to_owned);
        let subpath = str_arg(q_args, "subpath")?.map(str::to_owned);
        let fuzzy = bool_arg(q_args, "fuzzy").unwrap_or(false);
        let hybrid = bool_arg(q_args, "hybrid").unwrap_or(false);
        let include_files = bool_arg(q_args, "include_files").unwrap_or(false);

        if text.trim().is_empty() && regex.is_none() {
            anyhow::bail!("query at index {idx} requires non-empty 'text' or a 'regex' pattern");
        }
        if let Some(ref pat) = regex {
            if pat.trim().is_empty() {
                anyhow::bail!("query at index {idx}: regex pattern must not be empty");
            }
            regex::Regex::new(pat)
                .map_err(|e| anyhow::anyhow!("query at index {idx}: invalid regex pattern: {e}"))?;
        }

        let query = SearchQuery {
            text: text.clone(),
            kind,
            language,
            include_files,
            limit,
            subpath,
            graph_expand: expand,
            graph_max_hops: expand_hops,
            regex_pattern: regex,
            fuzzy_match: fuzzy,
            hybrid,
            ..Default::default()
        };

        let results = if semantic {
            sem::expanded_search(&store, &query).context("semantic search failed")?
        } else {
            store.search(&query).context("search failed")?
        };

        let items: Vec<BatchResultNode> = results
            .iter()
            .map(|r| BatchResultNode {
                score: (r.score * 1000.0).round() / 1000.0,
                name: r.node.name.clone(),
                qualified_name: r.node.qualified_name.clone(),
                kind: r.node.kind.as_str().to_owned(),
                file_path: r.node.file_path.clone(),
                line_start: r.node.line_start,
                language: r.node.language.clone(),
            })
            .collect();

        let atlas_hint = if items.is_empty() && semantic {
            Some(
                "FTS found no symbol names matching the query text. \
                 FTS searches indexed identifiers, not natural language phrases. \
                 Try: (1) a short exact symbol name like 'BalancesTab'; \
                 (2) the regex param for pattern matching (e.g. regex='Balance'); \
                 (3) get_context with a file path; \
                 (4) list_graph_stats to confirm the graph has been built."
                    .to_owned(),
            )
        } else {
            None
        };

        batch_results.push(BatchItem {
            query_index: idx,
            text,
            items,
            atlas_hint,
        });
    }

    let mut response = tool_result_value(&batch_results, output_format)?;
    response["atlas_result_kind"] = serde_json::Value::String("batch_symbol_search".to_owned());
    response["atlas_query_count"] =
        serde_json::Value::Number(serde_json::Number::from(batch_results.len()));
    Ok(response)
}

pub(super) fn tool_traverse_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let from_qn = normalize_qn_kind_tokens(
        str_arg(args, "from_qn")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: from_qn"))?,
    );
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(100) as usize;

    let store = open_store(db_path)?;
    let result = store
        .traverse_from_qnames(&[from_qn.as_str()], max_depth, max_nodes)
        .context("traverse_from_qnames failed")?;

    let seeds = vec![from_qn];
    let packaged = package_impact(&result, &seeds);
    tool_result_value(&packaged, output_format)
}

pub(super) fn tool_symbol_neighbors(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let qname = normalize_qn_kind_tokens(
        str_arg(args, "qname")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: qname"))?,
    );
    let limit = u64_arg(args, "limit").unwrap_or(10) as usize;

    let store = open_store(db_path)?;
    let nbhd =
        sem::symbol_neighborhood(&store, &qname, limit).context("symbol_neighborhood failed")?;
    let caller_pairs = store
        .direct_callers(&qname, limit)
        .context("direct_callers failed")?;
    let callee_pairs = store
        .direct_callees(&qname, limit)
        .context("direct_callees failed")?;

    #[derive(Serialize)]
    struct CompactCallEdge<'a> {
        from: &'a str,
        to: &'a str,
        file: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
        confidence: f32,
        #[serde(skip_serializing_if = "Option::is_none")]
        tier: Option<&'a str>,
    }

    fn compact_call_edge(edge: &atlas_core::Edge) -> CompactCallEdge<'_> {
        CompactCallEdge {
            from: &edge.source_qn,
            to: &edge.target_qn,
            file: &edge.file_path,
            line: edge.line,
            confidence: edge.confidence,
            tier: edge.confidence_tier.as_deref(),
        }
    }

    fn compact_unique_nodes_from_pairs<'a>(
        pairs: &'a [(atlas_core::Node, atlas_core::Edge)],
    ) -> Vec<crate::context::CompactNode<'a>> {
        let mut seen = std::collections::HashSet::new();
        let mut nodes = Vec::new();
        for (node, _) in pairs {
            if seen.insert(node.qualified_name.as_str()) {
                nodes.push(compact_node(node));
            }
        }
        nodes
    }

    #[derive(Serialize)]
    struct NeighborhoodResult<'a> {
        qname: &'a str,
        callers: Vec<crate::context::CompactNode<'a>>,
        callees: Vec<crate::context::CompactNode<'a>>,
        caller_edges: Vec<CompactCallEdge<'a>>,
        callee_edges: Vec<CompactCallEdge<'a>>,
        tests: Vec<crate::context::CompactNode<'a>>,
        siblings: Vec<crate::context::CompactNode<'a>>,
        import_neighbors: Vec<crate::context::CompactNode<'a>>,
    }

    let result = NeighborhoodResult {
        qname: &qname,
        callers: compact_unique_nodes_from_pairs(&caller_pairs),
        callees: compact_unique_nodes_from_pairs(&callee_pairs),
        caller_edges: caller_pairs
            .iter()
            .map(|(_, edge)| compact_call_edge(edge))
            .collect(),
        callee_edges: callee_pairs
            .iter()
            .map(|(_, edge)| compact_call_edge(edge))
            .collect(),
        tests: nbhd.tests.iter().map(compact_node).collect(),
        siblings: nbhd.siblings.iter().map(compact_node).collect(),
        import_neighbors: nbhd.import_neighbors.iter().map(compact_node).collect(),
    };

    let all_empty = result.callers.is_empty()
        && result.callees.is_empty()
        && result.tests.is_empty()
        && result.siblings.is_empty()
        && result.import_neighbors.is_empty();

    let mut response = tool_result_value(&result, output_format)?;
    if all_empty {
        let exists = store
            .node_by_qname(&qname)
            .map(|n| n.is_some())
            .unwrap_or(false);
        if !exists {
            response["atlas_error_code"] = serde_json::Value::String("node_not_found".to_owned());
            response["atlas_message"] =
                serde_json::Value::String(error_message("node_not_found").to_owned());
            response["atlas_suggestions"] = serde_json::json!(error_suggestions("node_not_found"));
        }
    }

    Ok(response)
}

pub(super) fn tool_cross_file_links(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let file = str_arg(args, "file")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?
        .to_owned();
    let limit = u64_arg(args, "limit").unwrap_or(20) as usize;

    let store = open_store(db_path)?;
    let links = sem::cross_file_links(&store, &file, limit).context("cross_file_links failed")?;

    #[derive(Serialize)]
    struct LinkResult {
        from_file: String,
        to_file: String,
        via_symbols: Vec<String>,
        strength: f64,
    }

    let result: Vec<LinkResult> = links
        .into_iter()
        .map(|l| LinkResult {
            from_file: l.from_file,
            to_file: l.to_file,
            via_symbols: l.via_symbols,
            strength: (l.strength * 10.0).round() / 10.0,
        })
        .collect();

    tool_result_value(&result, output_format)
}

pub(super) fn tool_concept_clusters(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let files = string_array_arg(args, "files")?;
    if files.is_empty() {
        return Err(anyhow::anyhow!("missing required argument: files"));
    }
    let limit = u64_arg(args, "limit").unwrap_or(10) as usize;

    let store = open_store(db_path)?;
    let seed_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let clusters = sem::cluster_by_shared_symbols(&store, &seed_refs, limit)
        .context("concept_clusters failed")?;

    #[derive(Serialize)]
    struct ClusterResult {
        files: Vec<String>,
        shared_symbols: Vec<String>,
        density: f64,
    }

    let result: Vec<ClusterResult> = clusters
        .into_iter()
        .map(|c| ClusterResult {
            files: c.files,
            shared_symbols: c.shared_symbols,
            density: (c.density * 1000.0).round() / 1000.0,
        })
        .collect();

    tool_result_value(&result, output_format)
}

pub(super) fn tool_explain_query(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let text = str_arg(args, "text")?
        .map(str::to_owned)
        .unwrap_or_default();
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let limit = u64_arg(args, "limit").unwrap_or(20) as usize;
    let semantic = bool_arg(args, "semantic").unwrap_or(false);
    let regex = str_arg(args, "regex")?.map(str::to_owned);
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let hybrid = bool_arg(args, "hybrid").unwrap_or(false);
    let include_files = bool_arg(args, "include_files").unwrap_or(false);

    if text.trim().is_empty() && regex.is_none() {
        anyhow::bail!("explain_query requires non-empty text or a regex pattern");
    }

    let fts_tokens: Vec<String> = if text.trim().is_empty() {
        vec![]
    } else {
        text.split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect()
    };

    let fts_phrase = if fts_tokens.is_empty() {
        None
    } else if fts_tokens.len() == 1 {
        Some(format!("\"{}\"", fts_tokens[0]))
    } else {
        Some(
            fts_tokens
                .iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(" "),
        )
    };

    let (regex_valid, regex_error) = if let Some(ref pat) = regex {
        match regex::Regex::new(pat) {
            Ok(_) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        }
    } else {
        (true, None)
    };

    let search_path = match (text.trim().is_empty(), regex.is_some(), semantic, hybrid) {
        (true, true, _, _) => "regex_structural_scan",
        (false, false, false, true) => "fts5_vector_hybrid",
        (false, false, true, _) => "fts5_graph_expand",
        (false, true, false, _) => "fts5_regex_filter",
        (false, true, true, _) => "fts5_regex_filter_graph_expand",
        _ => "fts5",
    };

    let mut ranking_factors: Vec<&str> = vec!["fts5_bm25"];
    if fuzzy {
        ranking_factors.push("fuzzy_edit_distance_boost");
    }
    if hybrid {
        ranking_factors.push("vector_rrf_merge");
    }
    if semantic {
        ranking_factors.push("graph_neighbor_rerank");
    }

    let db_exists = std::path::Path::new(db_path).exists();

    // Open store once; reuse for both node count and actual search execution.
    let (indexed_node_count, latency_ms, result_count, matches): (
        Option<i64>,
        Option<u128>,
        Option<usize>,
        Option<Vec<serde_json::Value>>,
    ) = if db_exists {
        match atlas_store_sqlite::Store::open(db_path) {
            Ok(store) => {
                let count = store.stats().ok().map(|st| st.node_count);
                if regex_valid {
                    let query = SearchQuery {
                        text: text.clone(),
                        kind: kind.clone(),
                        language: language.clone(),
                        include_files,
                        limit,
                        subpath: subpath.clone(),
                        regex_pattern: regex.clone(),
                        fuzzy_match: fuzzy,
                        hybrid,
                        ..Default::default()
                    };
                    let t0 = std::time::Instant::now();
                    let results = if semantic {
                        sem::expanded_search(&store, &query).unwrap_or_default()
                    } else if fuzzy || hybrid {
                        fts_search(&store, &query).unwrap_or_default()
                    } else {
                        store.search(&query).unwrap_or_default()
                    };
                    let elapsed = t0.elapsed().as_millis();
                    let m: Vec<serde_json::Value> = results
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
                    let n = m.len();
                    (count, Some(elapsed), Some(n), Some(m))
                } else {
                    (count, None, None, None)
                }
            }
            Err(_) => (None, None, None, None),
        }
    } else {
        (None, None, None, None)
    };

    let warnings: Vec<&str> = {
        let mut w = vec![];
        if fts_tokens.len() > 1 {
            w.push(
                "Multi-token text is matched as implicit AND across all tokens; \
                 this often returns zero results. Prefer a single short identifier.",
            );
        }
        if text.contains(' ') && regex.is_none() {
            w.push(
                "Natural-language phrases rarely match FTS5 symbol names. \
                 Use regex for pattern matching or pass a single exact identifier.",
            );
        }
        if !regex_valid {
            w.push("regex pattern is invalid; the query would return an error.");
        }
        w
    };

    let result = serde_json::json!({
        "active_query_mode": search_path,
        "search_path": search_path,
        "input": {
            "text": text,
            "kind": kind,
            "language": language,
            "limit": limit,
            "semantic": semantic,
            "regex": regex,
            "subpath": subpath,
            "fuzzy": fuzzy,
            "hybrid": hybrid,
            "include_files": include_files,
        },
        "fts_tokens": fts_tokens,
        "fts_phrase": fts_phrase,
        "regex_valid": regex_valid,
        "regex_error": regex_error,
        "ranking_factors": ranking_factors,
        "filters_applied": {
            "kind": kind.is_some(),
            "language": language.is_some(),
            "subpath": subpath.is_some(),
            "fuzzy": fuzzy,
            "hybrid": hybrid,
            "include_files": include_files,
        },
        "indexed_node_count": indexed_node_count,
        "db_exists": db_exists,
        "warnings": warnings,
        "latency_ms": latency_ms,
        "result_count": result_count,
        "matches": matches,
    });

    tool_result_value(&result, output_format)
}

pub(super) fn tool_resolve_symbol(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    const DEFAULT_LIMIT: usize = 10;

    let name = str_arg(args, "name")?
        .ok_or_else(|| anyhow::anyhow!("resolve_symbol requires 'name'"))?
        .to_owned();
    let kind_input = str_arg(args, "kind")?.map(str::to_owned);
    let file_filter = str_arg(args, "file")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let limit = u64_arg(args, "limit").unwrap_or(DEFAULT_LIMIT as u64) as usize;

    if name.trim().is_empty() {
        anyhow::bail!("resolve_symbol requires non-empty 'name'");
    }

    let store = open_store(db_path)?;

    if name.contains("::") {
        let target = ContextTarget::QualifiedName {
            qname: name.clone(),
        };
        match resolve_target(&store, &target).context("resolve_symbol qname lookup failed")? {
            ResolvedTarget::Node(node) => {
                #[derive(Serialize)]
                struct ResolvedMatch<'a> {
                    qualified_name: &'a str,
                    name: &'a str,
                    kind: &'a str,
                    file_path: &'a str,
                    language: &'a str,
                    line_start: u32,
                }
                let m = ResolvedMatch {
                    qualified_name: &node.qualified_name,
                    name: &node.name,
                    kind: node.kind.as_str(),
                    file_path: &node.file_path,
                    language: &node.language,
                    line_start: node.line_start,
                };
                let normalised = normalize_qn_kind_tokens(&name);
                let alias_note = if normalised != name {
                    Some(format!(
                        "Input '{name}' normalised to canonical QN '{normalised}'"
                    ))
                } else {
                    None
                };
                let result = serde_json::json!({
                    "qualified_name": m.qualified_name,
                    "resolved": true,
                    "ambiguous": false,
                    "match_count": 1,
                    "atlas_truncated": false,
                    "matches": [m],
                    "alias_note": alias_note,
                    "suggestions": [{
                        "hint": "Exact match resolved. Pass qualified_name to symbol_neighbors \
                                 or traverse_graph for callers, callees, and relationships.",
                        "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
                    }],
                });
                return tool_result_value(&result, output_format);
            }
            ResolvedTarget::Ambiguous(meta) => {
                let result = serde_json::json!({
                    "qualified_name": meta.candidates.first(),
                    "resolved": false,
                    "ambiguous": true,
                    "match_count": meta.candidates.len(),
                    "atlas_truncated": false,
                    "matches": serde_json::Value::Array(
                        meta.candidates.iter().map(|qn| serde_json::json!({"qualified_name": qn})).collect()
                    ),
                    "suggestions": [{
                        "hint": "Multiple symbols match. Narrow with 'file', 'kind', or 'language'. \
                                 Then pass the exact qualified_name to symbol_neighbors or traverse_graph.",
                        "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
                    }],
                });
                return tool_result_value(&result, output_format);
            }
            ResolvedTarget::NotFound { suggestions } => {
                let result = serde_json::json!({
                    "qualified_name": null,
                    "resolved": false,
                    "ambiguous": false,
                    "match_count": 0,
                    "atlas_truncated": false,
                    "matches": [],
                    "suggestions": [{
                        "hint": format!(
                            "No symbol matched '{}'. Verify canonical QN tokens (e.g. '::fn::' not '::function::'). \
                             Candidates: {:?}. Try query_graph or resolve_symbol with a shorter name.",
                            name, suggestions
                        ),
                        "candidates": suggestions,
                        "next_tools": ["query_graph", "explain_query"]
                    }],
                });
                return tool_result_value(&result, output_format);
            }
            ResolvedTarget::File(_) => {}
        }
    }

    let resolved_kind = kind_input.as_deref().map(resolve_kind_alias);
    let fetch_limit = (limit * 4).max(40);
    let query = SearchQuery {
        text: name.clone(),
        kind: resolved_kind.clone(),
        language: language.clone(),
        limit: fetch_limit,
        ..Default::default()
    };
    let results = store
        .search(&query)
        .context("resolve_symbol search failed")?;

    let filtered: Vec<_> = if let Some(ref file_pat) = file_filter {
        results
            .into_iter()
            .filter(|r| r.node.file_path.contains(file_pat.as_str()))
            .collect()
    } else {
        results
    };

    let name_lower = name.to_ascii_lowercase();
    let mut ranked: Vec<_> = filtered.into_iter().enumerate().collect();
    ranked.sort_by(|(ai, a), (bi, b)| {
        let a_exact = a.node.name.to_ascii_lowercase() == name_lower;
        let b_exact = b.node.name.to_ascii_lowercase() == name_lower;
        b_exact.cmp(&a_exact).then_with(|| ai.cmp(bi))
    });

    let total_before_limit = ranked.len();
    let ranked: Vec<_> = ranked.into_iter().map(|(_, r)| r).take(limit).collect();
    let truncated = total_before_limit > ranked.len();

    let best_qn = ranked.first().map(|r| r.node.qualified_name.as_str());
    let ambiguous = ranked.len() > 1;

    #[derive(Serialize)]
    struct ResolvedMatch<'a> {
        qualified_name: &'a str,
        name: &'a str,
        kind: &'a str,
        file_path: &'a str,
        language: &'a str,
        line_start: u32,
    }

    let matches: Vec<ResolvedMatch<'_>> = ranked
        .iter()
        .map(|r| ResolvedMatch {
            qualified_name: &r.node.qualified_name,
            name: &r.node.name,
            kind: r.node.kind.as_str(),
            file_path: &r.node.file_path,
            language: &r.node.language,
            line_start: r.node.line_start,
        })
        .collect();

    let suggestions: Vec<serde_json::Value> = if best_qn.is_some() {
        if ambiguous {
            vec![serde_json::json!({
                "hint": "Multiple symbols match. Narrow with 'file', 'kind', or 'language'. \
                         Then pass the exact qualified_name to symbol_neighbors or traverse_graph.",
                "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
            })]
        } else {
            vec![serde_json::json!({
                "hint": "Exact match resolved. Pass qualified_name to symbol_neighbors \
                         or traverse_graph for callers, callees, and relationships.",
                "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
            })]
        }
    } else {
        vec![serde_json::json!({
            "hint": "No symbol matched. Try query_graph with a regex pattern, \
                     or use explain_query to validate the search input.",
            "next_tools": ["query_graph", "explain_query"]
        })]
    };

    let result = serde_json::json!({
        "qualified_name": best_qn,
        "resolved": best_qn.is_some(),
        "ambiguous": ambiguous,
        "match_count": matches.len(),
        "atlas_truncated": truncated,
        "matches": matches,
        "suggestions": suggestions,
    });

    tool_result_value(&result, output_format)
}
