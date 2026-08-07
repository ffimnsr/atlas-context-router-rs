use super::*;

pub(crate) fn tool_get_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    use atlas_contentstore::ContentStore;

    let target = match parse_get_context_target(args) {
        Ok(target) => target,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let intent_override = str_arg(args, "intent")?.map(str::to_owned);
    let max_nodes = u64_arg(args, "max_nodes").map(|n| n as usize);
    let max_edges = u64_arg(args, "max_edges").map(|n| n as usize);
    let max_files = u64_arg(args, "max_files").map(|n| n as usize);
    let max_depth = u64_arg(args, "max_depth").map(|n| n as u32);
    let code_spans = bool_arg(args, "code_spans");
    let tests = bool_arg(args, "tests");
    let imports = bool_arg(args, "imports");
    let neighbors = bool_arg(args, "neighbors");
    let semantic = bool_arg(args, "semantic").unwrap_or(false);
    let include_saved_context = bool_arg(args, "include_saved_context").unwrap_or(false);
    let allow_cross_repo_edges = bool_arg(args, "allow_cross_repo_edges").unwrap_or(false);
    let session_id = str_arg(args, "session_id")?.map(str::to_owned);
    let agent_id = str_arg(args, "agent_id")?.map(str::to_owned);
    let merge_agent_partitions = bool_arg(args, "merge_agent_partitions").unwrap_or(false);
    let token_budget = u64_arg(args, "token_budget").map(|n| n as usize);

    let mut request = match target.kind {
        GetContextTargetKind::Files => {
            let intent = intent_override
                .as_deref()
                .map(parse_mcp_intent)
                .unwrap_or(ContextIntent::Review);
            ContextRequest {
                intent,
                target: target.target.clone(),
                ..ContextRequest::default()
            }
        }
        GetContextTargetKind::File => {
            let intent = intent_override
                .as_deref()
                .map(parse_mcp_intent)
                .unwrap_or(ContextIntent::File);
            ContextRequest {
                intent,
                target: target.target.clone(),
                ..ContextRequest::default()
            }
        }
        GetContextTargetKind::Query => {
            let mut parsed = target
                .parsed_request
                .clone()
                .expect("query target parsed request");
            if let Some(ref ov) = intent_override {
                parsed.intent = parse_mcp_intent(ov);
            }
            parsed
        }
    };

    if max_nodes.is_some() {
        request.max_nodes = max_nodes;
    }
    if max_edges.is_some() {
        request.max_edges = max_edges;
    }
    if max_files.is_some() {
        request.max_files = max_files;
    }
    if max_depth.is_some() {
        request.depth = max_depth;
    }
    if let Some(v) = code_spans {
        request.include_code_spans = v;
    }
    if let Some(v) = tests {
        request.include_tests = v;
    }
    if let Some(v) = imports {
        request.include_imports = v;
    }
    if let Some(v) = neighbors {
        request.include_neighbors = v;
    }
    request.include_saved_context = include_saved_context;
    request.allow_cross_repo_edges = allow_cross_repo_edges;
    request.session_id = session_id;
    request.agent_id = agent_id.clone();
    request.merge_agent_partitions = merge_agent_partitions;
    if token_budget.is_some() {
        request.token_budget = token_budget;
    }

    let store = open_store(db_path)?;
    let policy = load_budget_policy(repo_root)?;
    let token_counter = load_token_counter(repo_root)?;

    // --semantic: when target is a SymbolName, run graph-aware semantic search
    // first to resolve the best-matching qualified name, then build context
    // around the resolved node instead of doing a fuzzier name lookup.
    if semantic && let ContextTarget::SymbolName { ref name } = request.target {
        let sq = SearchQuery {
            text: name.clone(),
            limit: 5,
            graph_expand: true,
            graph_max_hops: 1,
            ..Default::default()
        };
        let hits = sem::context_boosted_search(&store, &sq, &[], &[]).unwrap_or_default();
        if let Some(top) = hits.into_iter().next() {
            request.target = ContextTarget::QualifiedName {
                qname: top.node.qualified_name,
            };
        }
    }

    let engine = ContextEngine::new(&store)
        .with_budget_policy(policy)
        .with_token_counter(token_counter.counter)
        .with_token_fallback(token_counter.fallback_used, token_counter.fallback_reason);

    let result = if include_saved_context {
        let content_db = derive_content_db_path(db_path);
        match ContentStore::open(&content_db) {
            Ok(mut cs) => {
                let _ = cs.migrate();
                let engine = engine.with_content_store(&cs);
                engine.build(&request).context("context engine failed")?
            }
            Err(_) => engine.build(&request).context("context engine failed")?,
        }
    } else {
        engine.build(&request).context("context engine failed")?
    };

    let include_context_ranking_evidence = output_format == crate::output::OutputFormat::Json;
    let packaged = package_context_result(&result, include_context_ranking_evidence);
    let mut packaged_value = serde_json::to_value(&packaged)?;
    let linked_decisions = context_decision_lookup_query(&request)
        .map(|query| {
            let hits = search_decisions_best_effort(
                repo_root,
                db_path,
                request.session_id.as_deref(),
                &query,
                3,
            );
            (query, hits)
        })
        .filter(|(_, hits)| !hits.is_empty());
    if let Some((query, hits)) = linked_decisions.as_ref() {
        packaged_value["linked_decisions"] = decision_hits_json(hits);
        packaged_value["decision_lookup_query"] = serde_json::Value::String(query.clone());
    }
    let context_files: Vec<String> = match &request.target {
        ContextTarget::ChangedFiles { paths } => paths.clone(),
        ContextTarget::FilePath { path } => vec![path.clone()],
        _ => Vec::new(),
    }
    .into_iter()
    .chain(result.files.iter().map(|file| file.path.clone()))
    .chain(result.nodes.iter().map(|node| node.node.file_path.clone()))
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect();

    let mut omitted: Vec<&str> = Vec::new();
    if !result.request.include_tests {
        omitted.push("tests");
    }
    if !result.request.include_code_spans {
        omitted.push("code_spans");
    }
    if !result.request.include_neighbors {
        omitted.push("neighbors");
    }

    let response_budget_limit = policy
        .mcp_cli_payload_serialization
        .mcp_response_bytes
        .default_limit;

    let response_budget_limit = response_budget_limit.saturating_sub(500);
    let stage_budget = if let Some(response_budget) =
        enforce_mcp_response_budget(&mut packaged_value, output_format, response_budget_limit)?
    {
        result.budget.clone().merge(response_budget)
    } else {
        result.budget.clone()
    };
    let mode = match &request.target {
        ContextTarget::ChangedFiles { .. } => "change_context",
        ContextTarget::FilePath { .. } => "file_context",
        ContextTarget::QualifiedName { .. } | ContextTarget::SymbolName { .. } => "symbol_context",
        ContextTarget::ChangedSymbols { .. } => "change_context",
        ContextTarget::EdgeQuerySeed { .. } => "symbol_context",
    };
    let assets = packaged_value
        .get("saved_context_sources")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|asset| {
            let mut object = as_object_map(asset);
            object.insert("artifact_kind".to_owned(), json!("saved_context"));
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    let ranked_symbols = packaged_value
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|node| {
            json!({
                "qn": node.get("qn").cloned().unwrap_or(Value::Null),
                "reason": node.get("reason").cloned().unwrap_or(Value::Null),
                "distance": node.get("distance").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    let ranked_edges = packaged_value
        .get("edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|edge| {
            json!({
                "from": edge.get("from").cloned().unwrap_or(Value::Null),
                "to": edge.get("to").cloned().unwrap_or(Value::Null),
                "kind": edge.get("kind").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    let ranked_files = packaged_value
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|file| {
            json!({
                "path": file.get("path").cloned().unwrap_or(Value::Null),
                "reason": file.get("reason").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    let ambiguity_candidates = packaged_value
        .get("ambiguity_candidates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let repo_aliases = repo_aliases_by_id(repo_root);
    let ambiguity_candidates_detailed = ambiguity_candidates
        .iter()
        .filter_map(Value::as_str)
        .map(|qname| {
            let node = store.node_by_qname(qname).ok().flatten();
            let repo_id = node.as_ref().and_then(node_repo_id);
            json!({
                "qualified_name": qname,
                "file_path": node.as_ref().map(|node| node.file_path.clone()),
                "kind": node.as_ref().map(|node| node.kind.as_str().to_owned()),
                "repo": {
                    "repo_id": repo_id,
                    "display_alias": repo_id.and_then(|id| repo_aliases.get(id)).cloned(),
                }
            })
        })
        .collect::<Vec<_>>();
    let ambiguity = json!({
        "query": packaged_value.get("ambiguity_query").cloned().unwrap_or(Value::Null),
        "candidates": serde_json::Value::Array(ambiguity_candidates),
        "candidates_detailed": ambiguity_candidates_detailed,
    });
    let mut normalized_payload = as_object_map(packaged_value.clone());
    normalized_payload.remove("saved_context_sources");
    normalized_payload.insert("mode".to_owned(), json!(mode));
    normalized_payload.insert(
        "target".to_owned(),
        json!({
            "kind": target.kind.as_str(),
            "query": target.query,
            "file": target.file,
            "files": target.files,
        }),
    );
    normalized_payload.insert(
        "query".to_owned(),
        match &request.target {
            ContextTarget::QualifiedName { qname } => json!(qname),
            ContextTarget::SymbolName { name } => json!(name),
            ContextTarget::EdgeQuerySeed { source_qname, .. } => json!(source_qname),
            _ => Value::Null,
        },
    );
    normalized_payload.insert(
        "file".to_owned(),
        match &request.target {
            ContextTarget::FilePath { path } => json!(path),
            _ => Value::Null,
        },
    );
    normalized_payload.insert(
        "files".to_owned(),
        match &request.target {
            ContextTarget::ChangedFiles { paths } => json!(paths),
            _ => json!([]),
        },
    );
    normalized_payload.insert("ranked_symbols".to_owned(), Value::Array(ranked_symbols));
    normalized_payload.insert("ranked_edges".to_owned(), Value::Array(ranked_edges));
    normalized_payload.insert("ranked_files".to_owned(), Value::Array(ranked_files));
    normalized_payload.insert("assets".to_owned(), Value::Array(assets));
    normalized_payload.insert("ambiguity".to_owned(), ambiguity);
    if include_context_ranking_evidence {
        normalized_payload.insert(
            "ranking_evidence_legend".to_owned(),
            context_ranking_evidence_legend_json(),
        );
    }
    normalized_payload.insert("context_files".to_owned(), json!(context_files));
    normalized_payload.insert(
        "cross_repo_context_hops".to_owned(),
        cross_repo_context_hops_json(
            packaged_value
                .get("edges")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &store,
        ),
    );
    normalized_payload.insert(
        "detail_controls".to_owned(),
        serde_json::json!({
            "max_files": result.request.max_files,
            "max_nodes": result.request.max_nodes,
            "max_edges": result.request.max_edges,
            "code_spans": result.request.include_code_spans,
            "tests": result.request.include_tests,
            "imports": result.request.include_imports,
            "neighbors": result.request.include_neighbors,
            "semantic": semantic,
            "allow_cross_repo_edges": result.request.allow_cross_repo_edges,
            "agent_id": result.request.agent_id,
            "merge_agent_partitions": result.request.merge_agent_partitions,
            "omitted_sections": omitted,
        }),
    );
    normalized_payload.insert(
        "agent_scope".to_owned(),
        serde_json::json!({
            "agent_id": result.request.agent_id,
            "merge_agent_partitions": result.request.merge_agent_partitions,
        }),
    );
    let missing_lookup_hint = "No graph nodes matched this request. Possible causes: \
         (1) the graph has not been built yet — run build_or_update_graph first; \
         (2) 'query' contained a natural-language phrase instead of a symbol name or \
         qualified name — try a short exact identifier (e.g. 'BalancesTab') or \
         use query_graph with regex for pattern matching; \
         (3) the file path is wrong or the file has no indexed symbols.";
    let lookup = if result.nodes.is_empty() {
        serde_json::json!({
            "status": "node_not_found",
            "error_code": "node_not_found",
            "error_code_docs": error_code_docs("node_not_found"),
            "message": error_message("node_not_found"),
            "suggestions": error_suggestions("node_not_found"),
            "hint": missing_lookup_hint,
        })
    } else {
        serde_json::json!({
            "status": "ok",
            "error_code": Value::Null,
            "error_code_docs": Value::Null,
            "message": Value::Null,
            "suggestions": [],
            "hint": Value::Null,
        })
    };
    normalized_payload.insert("lookup".to_owned(), lookup);
    let warnings = if result.nodes.is_empty() {
        vec![error_message("node_not_found").to_owned()]
    } else {
        Vec::new()
    };

    let mut response = build_normalized_success_response(
        "get_context",
        Value::Object(normalized_payload),
        output_format,
        warnings,
        packaged.truncated,
        packaged
            .truncated
            .then_some("context capped by node, edge, file, or payload budget"),
    )?;
    inject_budget_metadata(&mut response, &stage_budget);
    inject_deprecated_input_fields(&mut response, &target.deprecated_input_fields);
    if let Some((query, hits)) = linked_decisions {
        let source_ids = hits
            .iter()
            .flat_map(|hit| hit.decision.source_ids.iter().cloned())
            .take(5)
            .collect::<Vec<_>>();
        record_mcp_decision_best_effort(
            repo_root,
            db_path,
            &format!("reuse prior decision for context: {query}"),
            Some("stored decision memory matched current context request"),
            serde_json::json!({
                "query": query,
                "conclusion": "prior decision reused for context request",
                "source_ids": source_ids,
                "evidence": hits.iter().take(3).map(|hit| serde_json::json!({
                    "decision_id": hit.decision.decision_id,
                    "summary": hit.decision.summary,
                    "relevance_score": hit.relevance_score,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    Ok(response)
}
