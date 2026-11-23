use super::*;

#[test]
fn batch_query_graph_items_returns_per_query_results() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "items": [
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

    let body = &response["structuredContent"];
    assert_eq!(body["summary"]["query_count"], 2);

    let items = body["items"].as_array().expect("items array");
    let results = body["results"].as_array().expect("results array");
    assert_eq!(items.len(), 2);
    assert_eq!(results.len(), 2);
    assert_eq!(items[0]["query_index"], 0);
    assert_eq!(items[0]["normalized_text"], "compute");
    let first_items = results[0]["matches"].as_array().expect("matches array");
    assert!(!first_items.is_empty(), "expected results for 'compute'");
    assert!(
        first_items
            .iter()
            .any(|n| n["qualified_name"] == "src/service.rs::fn::compute")
    );
    assert_eq!(items[1]["query_index"], 1);
    assert_eq!(items[1]["normalized_text"], "handle_request");
    let second_items = results[1]["matches"].as_array().expect("matches array");
    assert!(
        !second_items.is_empty(),
        "expected results for 'handle_request'"
    );
}

#[test]
fn batch_query_graph_accepts_canonical_items_only() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "items": [
            { "text": "compute" },
            { "text": "handle_request" }
        ],
        "output_format": "json"
    });
    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected batch_query_graph success for canonical items");

    assert!(response["_meta"].get("deprecated_input_fields").is_none());
}

#[test]
fn batch_query_graph_empty_items_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "items": [] });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected tool error for empty items array");
    assert_eq!(result["isError"], serde_json::json!(true));
    let msg = result["structuredContent"]["message"]
        .as_str()
        .unwrap_or("");
    assert!(msg.contains("non-empty"));
}

#[test]
fn batch_query_graph_rejects_legacy_queries_field() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "queries": [{ "text": "compute" }, { "text": "handle_request" }],
        "output_format": "json"
    });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph legacy queries must return tool error result");

    assert_eq!(response["isError"], serde_json::json!(true));
    assert_eq!(
        response["structuredContent"]["message"],
        serde_json::json!("legacy batch_query_graph fields are no longer supported")
    );
}

#[test]
fn batch_query_graph_rejects_legacy_text_field() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "compute,handle_request",
        "output_format": "json"
    });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph legacy text must return tool error result");
    assert_eq!(response["isError"], serde_json::json!(true));
    assert_eq!(
        response["structuredContent"]["message"],
        serde_json::json!("legacy batch_query_graph fields are no longer supported")
    );
}

#[test]
fn batch_query_graph_rejects_legacy_text_plus_queries() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "compute",
        "queries": [{ "text": "handle_request" }]
    });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected tool error for text+queries conflict");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["message"],
        serde_json::json!("legacy batch_query_graph fields are no longer supported")
    );
}

#[test]
fn batch_query_graph_rejects_text_plus_items() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "compute",
        "items": [{ "text": "handle_request" }]
    });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected tool error for text+items conflict");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["message"],
        serde_json::json!("legacy batch_query_graph fields are no longer supported")
    );
}

#[test]
fn batch_query_graph_over_limit_returns_error() {
    let fixture = setup_mcp_fixture();
    let items: Vec<serde_json::Value> = (0..21)
        .map(|i| serde_json::json!({ "text": format!("sym{i}") }))
        .collect();
    let args = serde_json::json!({ "items": items });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected tool error for >20 queries");
    assert_eq!(result["isError"], serde_json::json!(true));
    let msg = result["structuredContent"]["message"]
        .as_str()
        .unwrap_or("");
    assert!(
        msg.contains("maximum"),
        "error should mention maximum: {msg}"
    );
}

#[test]
fn batch_query_graph_partial_empty_result_carries_hint() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "items": [
            { "text": "compute" },
            { "text": "who calls missing_symbol", "semantic": true }
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

    let results = response["structuredContent"]["results"]
        .as_array()
        .expect("results array");
    assert_eq!(results.len(), 2);

    let first_items = results[0]["matches"].as_array().expect("matches");
    assert!(!first_items.is_empty());
    assert_eq!(
        results[0]["warnings"].as_array().map(Vec::len),
        Some(0),
        "no warning for successful query"
    );

    let second_items = results[1]["matches"].as_array().expect("matches");
    assert!(
        second_items.is_empty(),
        "expected empty results for NL phrase"
    );
    let hint = results[1]["warnings"][0].as_str().expect("warning present");
    assert!(
        hint.contains("FTS found no symbol names"),
        "hint should explain FTS limit: {hint}"
    );
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
fn batch_query_graph_json_includes_ranking_evidence() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "items": [{ "text": "compute" }],
        "output_format": "json"
    });
    let resp = call("batch_query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("batch_query_graph call");
    let first = &resp["structuredContent"]["results"]
        .as_array()
        .expect("batch results")[0]["matches"][0];
    assert!(first.get("ranking_evidence").is_some());
    assert!(resp.get("atlas_ranking_evidence_legend").is_some());
}
