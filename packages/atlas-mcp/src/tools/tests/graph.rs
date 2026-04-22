use super::*;

#[test]
fn query_graph_regex_param_filters_results() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "compute", "output_format": "json" });
    let response = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("query_graph regex call");
    let text = unwrap_tool_text(response);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    if let Some(arr) = v.as_array() {
        for item in arr {
            let qn = item["qn"].as_str().unwrap_or("");
            let name = item["name"].as_str().unwrap_or("");
            assert!(
                qn.contains("compute") || name.contains("compute"),
                "regex filter should only return matching symbols, got qn={qn} name={name}"
            );
        }
    }
}

#[test]
fn query_graph_invalid_regex_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "[invalid", "output_format": "json" });
    let result = call("query_graph", Some(&args), "/ignored", &fixture.db_path);
    assert!(result.is_err(), "invalid regex must return an error");
    assert!(
        result.unwrap_err().to_string().contains("invalid regex"),
        "error message should mention invalid regex"
    );
}

#[test]
fn query_graph_response_carries_relationship_guidance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });

    let response =
        call("query_graph", Some(&args), "/ignored", &fixture.db_path).expect("query_graph call");

    assert_eq!(response["atlas_result_kind"], "symbol_search");
    assert_eq!(response["atlas_usage_edges_included"], false);
    assert_eq!(
        response["atlas_relationship_tools"],
        serde_json::json!(["symbol_neighbors", "traverse_graph", "get_context"])
    );
    assert_eq!(response["content"].as_array().map(Vec::len), Some(1));
}

#[test]
fn semantic_empty_result_includes_hint() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "balances tab portfolio asset balance usd notional", "semantic": true, "output_format": "json" });

    let response = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("query_graph semantic call");

    let content = response["content"].as_array().expect("content array");
    assert_eq!(content.len(), 1);
    let text = content[0]["text"].as_str().unwrap_or("");
    assert!(text.contains("[]"), "expected empty results, got: {text}");
    assert!(
        response["atlas_hint"].as_str().is_some(),
        "expected atlas_hint when semantic returns empty, got none"
    );
    let hint = response["atlas_hint"].as_str().unwrap();
    assert!(
        hint.contains("FTS found no symbol names"),
        "hint should explain FTS limitation: {hint}"
    );
}

#[test]
fn batch_query_graph_returns_per_query_results() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "queries": [
            { "text": "compute", "output_format": "json" },
            { "text": "handle_request", "output_format": "json" }
        ],
        "output_format": "json"
    });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph call");

    assert_eq!(response["atlas_result_kind"], "batch_symbol_search");
    assert_eq!(response["atlas_query_count"], 2);

    let text = response["content"][0]["text"].as_str().unwrap();
    let items: serde_json::Value = serde_json::from_str(text).expect("parse batch result");
    let arr = items.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["query_index"], 0);
    assert_eq!(arr[0]["text"], "compute");
    let first_items = arr[0]["items"].as_array().expect("items array");
    assert!(!first_items.is_empty(), "expected results for 'compute'");
    assert!(
        first_items
            .iter()
            .any(|n| n["qualified_name"] == "src/service.rs::fn::compute")
    );
    assert_eq!(arr[1]["query_index"], 1);
    assert_eq!(arr[1]["text"], "handle_request");
    let second_items = arr[1]["items"].as_array().expect("items array");
    assert!(
        !second_items.is_empty(),
        "expected results for 'handle_request'"
    );
}

#[test]
fn batch_query_graph_empty_queries_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "queries": [] });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    );
    assert!(result.is_err(), "expected error for empty queries array");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("non-empty") || msg.contains("requires"));
}

#[test]
fn batch_query_graph_text_phrase_splits_and_queries_each_token() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute handle_request", "output_format": "json" });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph with text phrase");

    assert_eq!(response["atlas_result_kind"], "batch_symbol_search");
    assert_eq!(response["atlas_query_count"], 2);

    let text = response["content"][0]["text"].as_str().unwrap();
    let arr: serde_json::Value = serde_json::from_str(text).expect("parse batch result");
    let arr = arr.as_array().expect("array");
    assert_eq!(arr.len(), 2, "one result per token");
    assert_eq!(arr[0]["text"], "compute");
    assert_eq!(arr[1]["text"], "handle_request");
    assert!(
        !arr[0]["items"].as_array().unwrap().is_empty(),
        "compute should have results"
    );
}

#[test]
fn batch_query_graph_over_limit_returns_error() {
    let fixture = setup_mcp_fixture();
    let queries: Vec<serde_json::Value> = (0..21)
        .map(|i| serde_json::json!({ "text": format!("sym{i}") }))
        .collect();
    let args = serde_json::json!({ "queries": queries });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    );
    assert!(result.is_err(), "expected error for >20 queries");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("maximum"),
        "error should mention maximum: {msg}"
    );
}

#[test]
fn batch_query_graph_partial_empty_result_carries_hint() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "queries": [
            { "text": "compute" },
            { "text": "balances tab portfolio asset usd notional", "semantic": true }
        ],
        "output_format": "json"
    });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph call");

    let text = response["content"][0]["text"].as_str().unwrap();
    let items: serde_json::Value = serde_json::from_str(text).expect("parse batch result");
    let arr = items.as_array().expect("array");
    assert_eq!(arr.len(), 2);

    let first_items = arr[0]["items"].as_array().expect("items");
    assert!(!first_items.is_empty());
    assert!(
        arr[0].get("atlas_hint").is_none(),
        "no hint for successful query"
    );

    let second_items = arr[1]["items"].as_array().expect("items");
    assert!(
        second_items.is_empty(),
        "expected empty results for NL phrase"
    );
    let hint = arr[1]["atlas_hint"].as_str().expect("atlas_hint present");
    assert!(
        hint.contains("FTS found no symbol names"),
        "hint should explain FTS limit: {hint}"
    );
}

#[test]
fn symbol_neighbors_includes_call_edge_sites() {
    let fixture = setup_mcp_fixture();
    let mut store = Store::open(&fixture.db_path).expect("open store");
    let handle = make_node(
        NodeKind::Function,
        "handle_request",
        "src/api.rs::fn::handle_request",
        "src/api.rs",
    );
    let first_call = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    let mut second_call = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    second_call.line = Some(2);
    store
        .replace_file_graph(
            "src/api.rs",
            "hash:src/api.rs",
            Some("rust"),
            Some(5),
            &[handle],
            &[first_call, second_call],
        )
        .expect("replace api graph");

    let args =
        serde_json::json!({ "qname": "src/service.rs::fn::compute", "output_format": "json" });
    let response = call(
        "symbol_neighbors",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("symbol_neighbors call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        value.pointer("/callers/0/qn").and_then(|v| v.as_str()),
        Some("src/api.rs::fn::handle_request")
    );
    assert_eq!(
        value.pointer("/callers/1").and_then(|v| v.as_object()),
        None
    );
    assert_eq!(
        value
            .pointer("/caller_edges/0/from")
            .and_then(|v| v.as_str()),
        Some("src/api.rs::fn::handle_request")
    );
    assert_eq!(
        value.pointer("/caller_edges/0/to").and_then(|v| v.as_str()),
        Some("src/service.rs::fn::compute")
    );
    assert_eq!(
        value
            .pointer("/caller_edges/0/file")
            .and_then(|v| v.as_str()),
        Some("src/api.rs")
    );
    assert_eq!(
        value
            .pointer("/caller_edges/0/line")
            .and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        value
            .pointer("/caller_edges/1/line")
            .and_then(|v| v.as_u64()),
        Some(2)
    );
}

#[test]
fn list_graph_stats_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let resp = call("list_graph_stats", None, "/repo", &fixture.db_path).expect("list_graph_stats");
    assert_provenance(&resp, "/repo", &fixture.db_path);
    let prov = &resp["atlas_provenance"];
    assert_eq!(prov["indexed_file_count"].as_i64(), Some(3));
    assert!(prov["last_indexed_at"].as_str().is_some());
}

#[test]
fn query_graph_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn traverse_graph_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "from_qn": "src/service.rs::fn::compute", "output_format": "json" });
    let resp =
        call("traverse_graph", Some(&args), "/repo", &fixture.db_path).expect("traverse_graph");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn symbol_neighbors_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "qname": "src/service.rs::fn::compute", "output_format": "json" });
    let resp =
        call("symbol_neighbors", Some(&args), "/repo", &fixture.db_path).expect("symbol_neighbors");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn symbol_neighbors_missing_qname_sets_error_code() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "qname": "src/nonexistent.rs::fn::ghost", "output_format": "json" });
    let resp = call("symbol_neighbors", Some(&args), "/repo", &fixture.db_path)
        .expect("symbol_neighbors should not error for missing symbol");
    assert_eq!(resp["atlas_error_code"].as_str(), Some("node_not_found"));
    assert!(resp["atlas_message"].as_str().is_some());
    let suggestions = resp["atlas_suggestions"]
        .as_array()
        .expect("atlas_suggestions");
    assert!(!suggestions.is_empty());
}

#[test]
fn cross_file_links_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "file": "src/service.rs", "output_format": "json" });
    let resp =
        call("cross_file_links", Some(&args), "/repo", &fixture.db_path).expect("cross_file_links");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn concept_clusters_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" });
    let resp =
        call("concept_clusters", Some(&args), "/repo", &fixture.db_path).expect("concept_clusters");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn batch_query_graph_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute" });
    let resp = call("batch_query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("batch_query_graph");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn provenance_indexed_file_count_is_zero_for_empty_db() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("empty.db").to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let resp =
        call("list_graph_stats", None, "/repo", &db_path).expect("list_graph_stats on empty db");
    let prov = &resp["atlas_provenance"];
    assert_eq!(prov["indexed_file_count"].as_i64(), Some(0));
    assert!(prov["last_indexed_at"].is_null());
}

#[test]
fn explain_query_describes_fts_path() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["search_path"].as_str(), Some("fts5"));
    let tokens = v["fts_tokens"].as_array().expect("fts_tokens array");
    assert!(tokens.iter().any(|t| t.as_str() == Some("compute")));
    assert_eq!(v["fts_phrase"].as_str(), Some("\"compute\""));
    assert_eq!(v["regex_valid"].as_bool(), Some(true));
}

#[test]
fn explain_query_missing_input_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("explain_query", Some(&args), "/repo", &fixture.db_path);
    assert!(result.is_err());
}

#[test]
fn explain_query_validates_invalid_regex() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "[invalid", "output_format": "json" });
    let resp = call("explain_query", Some(&args), "/repo", &fixture.db_path)
        .expect("explain_query should not error on invalid regex");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["regex_valid"].as_bool(), Some(false));
    assert!(v["regex_error"].as_str().is_some());
    let warnings = v["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().is_some_and(|s| s.contains("invalid")))
    );
}

#[test]
fn explain_query_with_regex_only_uses_structural_scan_path() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "compute.*", "output_format": "json" });
    let resp = call("explain_query", Some(&args), "/repo", &fixture.db_path)
        .expect("explain_query regex-only call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["search_path"].as_str(), Some("regex_structural_scan"));
    assert_eq!(v["regex_valid"].as_bool(), Some(true));
}

#[test]
fn explain_query_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn resolve_symbol_finds_exact_match() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["resolved"].as_bool(), Some(true));
    assert_eq!(
        v["qualified_name"].as_str(),
        Some("src/service.rs::fn::compute")
    );
    assert!(v["match_count"].as_i64().unwrap_or(0) >= 1);
    let matches = v["matches"].as_array().expect("matches array");
    assert!(!matches.is_empty());
    assert_eq!(matches[0]["kind"].as_str(), Some("function"));
    assert_eq!(matches[0]["file_path"].as_str(), Some("src/service.rs"));
}

#[test]
fn resolve_symbol_missing_name_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path);
    assert!(result.is_err());
}

#[test]
fn resolve_symbol_empty_name_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "", "output_format": "json" });
    let result = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path);
    assert!(result.is_err());
}

#[test]
fn resolve_symbol_no_match_returns_unresolved() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "nonexistent_symbol_xyz", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["resolved"].as_bool(), Some(false));
    assert!(v["qualified_name"].is_null());
    assert_eq!(v["match_count"].as_i64(), Some(0));
    let suggestions = v["suggestions"].as_array().expect("suggestions array");
    assert!(!suggestions.is_empty());
    let hint = suggestions[0]["hint"].as_str().expect("hint string");
    assert!(hint.contains("query_graph") || hint.contains("explain_query"));
}

#[test]
fn resolve_symbol_kind_alias_fn_resolves_to_function() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "kind": "fn", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["resolved"].as_bool(), Some(true));
    assert_eq!(
        v["qualified_name"].as_str(),
        Some("src/service.rs::fn::compute")
    );
}

#[test]
fn resolve_symbol_file_filter_narrows_results() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "handle_request", "file": "src/api.rs", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["resolved"].as_bool(), Some(true));
    assert_eq!(
        v["qualified_name"].as_str(),
        Some("src/api.rs::fn::handle_request")
    );
    let matches = v["matches"].as_array().expect("matches array");
    for m in matches {
        assert!(m["file_path"].as_str().unwrap_or("").contains("src/api.rs"));
    }
}

#[test]
fn resolve_symbol_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn resolve_symbol_includes_suggestions() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let suggestions = v["suggestions"].as_array().expect("suggestions array");
    assert!(!suggestions.is_empty());
    let next_tools = suggestions[0]["next_tools"].as_array().expect("next_tools");
    assert!(
        next_tools
            .iter()
            .any(|t| t.as_str() == Some("symbol_neighbors"))
    );
}

#[test]
fn resolve_symbol_truncation_metadata_present() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp.clone());
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert!(v.get("atlas_truncated").is_some());
}

#[test]
fn query_graph_truncation_metadata_present() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute" });
    let resp =
        call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph call");
    assert!(resp.get("atlas_truncated").is_some());
    assert!(resp.get("atlas_result_count").is_some());
}

#[test]
fn query_graph_subpath_filters_results() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "text": "compute", "subpath": "tests", "output_format": "json" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("query_graph subpath call");
    let text = unwrap_tool_text(resp.clone());
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    if let Some(arr) = v.as_array() {
        for item in arr {
            let fp = item["file"].as_str().unwrap_or("");
            assert!(
                fp.starts_with("tests"),
                "subpath='tests' must restrict results to tests/, got file={fp}"
            );
        }
    }
}

#[test]
fn query_graph_fuzzy_returns_near_miss() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "comput", "fuzzy": true, "output_format": "json" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("query_graph fuzzy call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let arr = v.as_array().expect("expected array result");
    assert!(
        arr.iter()
            .any(|item| item["qn"].as_str().unwrap_or("").contains("compute"))
    );
}

#[test]
fn query_graph_hybrid_falls_back_to_fts() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "hybrid": true, "output_format": "json" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("query_graph hybrid call");
    let text = unwrap_tool_text(resp.clone());
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let arr = v.as_array().expect("expected array result");
    assert!(
        arr.iter()
            .any(|item| item["qn"].as_str().unwrap_or("").contains("compute"))
    );
    assert_eq!(
        resp["atlas_query_mode"].as_str(),
        Some("fts5_vector_hybrid")
    );
}

#[test]
fn query_graph_response_includes_query_mode() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph call");
    assert!(resp.get("atlas_query_mode").is_some());
    assert_eq!(resp["atlas_query_mode"].as_str(), Some("fts5"));
}

#[test]
fn explain_query_reports_active_query_mode_and_ranking_factors() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "fuzzy": true, "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(v["active_query_mode"].as_str(), Some("fts5"));
    let factors = v["ranking_factors"]
        .as_array()
        .expect("ranking_factors array");
    assert!(
        factors
            .iter()
            .any(|f| f.as_str() == Some("fuzzy_edit_distance_boost"))
    );
}

#[test]
fn explain_query_reports_subpath_filter() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "text": "compute", "subpath": "src/auth", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(v["filters_applied"]["subpath"].as_bool(), Some(true));
    assert_eq!(v["input"]["subpath"].as_str(), Some("src/auth"));
}
