use super::*;

pub(crate) fn tool_traverse_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
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
    let policy = load_budget_policy(repo_root)?;
    let result = store
        .traverse_from_qnames(
            &[from_qn.as_str()],
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
        .context("traverse_from_qnames failed")?;

    let seeds = vec![from_qn.clone()];
    let packaged = package_impact(&result, &seeds);
    let mut payload = Map::new();
    let mut combined_nodes = serde_json::to_value(&packaged.changed_nodes)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    combined_nodes.extend(
        serde_json::to_value(&packaged.impacted_nodes)?
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    let mut node_qns: HashSet<String> = combined_nodes
        .iter()
        .filter_map(|node| node.get("qn").and_then(Value::as_str).map(str::to_owned))
        .collect();
    node_qns.insert(from_qn.clone());
    let emitted_edges = super::traversal_wire::traversal_edges(
        &result.relevant_edges,
        &from_qn,
        max_depth,
        &node_qns,
    );
    let emitted_edge_count = emitted_edges.len();
    let budget_hit = packaged
        .traversal_budget
        .is_some_and(|budget| budget.budget_hit);
    let truncated = packaged.truncated || budget_hit;
    payload.insert("root_symbol".to_owned(), json!(from_qn));
    payload.insert("direction".to_owned(), json!("bidirectional"));
    payload.insert("depth".to_owned(), json!(max_depth));
    payload.insert("nodes".to_owned(), Value::Array(combined_nodes));
    payload.insert("edges".to_owned(), Value::Array(emitted_edges));
    payload.insert(
        "summary".to_owned(),
        json!({
            "changed_symbol_count": packaged.changed_node_count,
            "impacted_symbol_count": packaged.impacted_node_count,
            "impacted_file_count": packaged.impacted_file_count,
            "relevant_edge_count": emitted_edge_count,
            "collected_edge_count": packaged.relevant_edge_count,
        }),
    );
    payload.insert("truncated".to_owned(), json!(truncated));
    payload.insert("budget_hit".to_owned(), json!(budget_hit));
    payload.insert("impacted_files".to_owned(), json!(packaged.impacted_files));
    payload.insert("seed_budgets".to_owned(), json!(packaged.seed_budgets));
    payload.insert(
        "traversal_budget".to_owned(),
        json!(packaged.traversal_budget),
    );

    let envelope = ToolSuccessEnvelope::new("traverse_graph", Value::Object(payload))
        .with_truncation(
            truncated,
            truncated.then_some("traversal capped by node or edge limits"),
        );
    let mut response = normalized_tool_result_value(&envelope, output_format)?;
    inject_budget_metadata(&mut response, &result.budget);
    Ok(response)
}

pub(crate) fn tool_symbol_neighbors(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let qname = normalize_qn_kind_tokens(
        str_arg(args, "qname")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: qname"))?,
    );
    let requested_limit = u64_arg(args, "limit").unwrap_or(10) as usize;
    let limit = budgets.resolve_limit(
        policy.review_context_extraction.nodes,
        "review_context_extraction.max_nodes",
        Some(requested_limit),
    );

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

    let callers = compact_unique_nodes_from_pairs(&caller_pairs);
    let callees = compact_unique_nodes_from_pairs(&callee_pairs);
    let caller_edges: Vec<_> = caller_pairs
        .iter()
        .map(|(_, edge)| compact_call_edge(edge))
        .collect();
    let callee_edges: Vec<_> = callee_pairs
        .iter()
        .map(|(_, edge)| compact_call_edge(edge))
        .collect();
    let tests: Vec<_> = nbhd.tests.iter().map(compact_node).collect();
    let siblings: Vec<_> = nbhd.siblings.iter().map(compact_node).collect();
    let imports: Vec<_> = nbhd.import_neighbors.iter().map(compact_node).collect();
    let exists = store
        .node_by_qname(&qname)
        .map(|n| n.is_some())
        .unwrap_or(false);
    let status = if exists { "ok" } else { "node_not_found" };
    let warnings = if exists {
        Vec::new()
    } else {
        vec![error_message("node_not_found").to_owned()]
    };
    let call_sites = caller_edges
        .iter()
        .map(|edge| {
            json!({
                "direction": "incoming",
                "from": edge.from,
                "to": edge.to,
                "file": edge.file,
                "line": edge.line,
                "confidence": edge.confidence,
                "tier": edge.tier,
            })
        })
        .chain(callee_edges.iter().map(|edge| {
            json!({
                "direction": "outgoing",
                "from": edge.from,
                "to": edge.to,
                "file": edge.file,
                "line": edge.line,
                "confidence": edge.confidence,
                "tier": edge.tier,
            })
        }))
        .collect::<Vec<_>>();
    let payload = json!({
        "tool": "symbol_neighbors",
        "symbol": {
            "qname": qname,
            "found": exists,
        },
        "lookup": {
            "status": status,
            "error_code": if exists { Value::Null } else { json!("node_not_found") },
            "error_code_docs": if exists { Value::Null } else { json!(error_code_docs("node_not_found")) },
            "message": if exists { Value::Null } else { json!(error_message("node_not_found")) },
            "suggestions": if exists { json!([]) } else { json!(error_suggestions("node_not_found")) },
        },
        "callers": callers,
        "callees": callees,
        "call_sites": call_sites,
        "tests": tests,
        "siblings": siblings,
        "imports": imports,
        "summary": {
            "status": status,
            "caller_count": caller_pairs.len(),
            "callee_count": callee_pairs.len(),
            "call_site_count": caller_edges.len() + callee_edges.len(),
            "test_count": nbhd.tests.len(),
            "sibling_count": nbhd.siblings.len(),
            "import_count": nbhd.import_neighbors.len(),
        },
        "warnings": warnings,
    });

    let mut response = tool_result_value(&payload, output_format)?;

    let observed = caller_pairs.len()
        + callee_pairs.len()
        + nbhd.tests.len()
        + nbhd.siblings.len()
        + nbhd.import_neighbors.len()
        + caller_edges.len()
        + callee_edges.len();
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "review_context_extraction.max_nodes",
            limit,
            requested_limit.max(observed),
        ),
    );

    Ok(response)
}
