use super::*;

#[test]
fn explain_query_reports_intent_and_target_symbol() {
    let fixture = setup_mcp_fixture();
    let resp = call(
        "explain_query",
        Some(&json!({ "text": "what breaks classify_graph_store_error", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("explain_query call");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(v["query_intent"]["kind"], json!("what_breaks"));
    assert_eq!(
        v["intent_target_symbol"],
        json!("classify_graph_store_error")
    );
}

#[test]
fn explain_query_describes_fts_path() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["normalized_query"]["search_path"].as_str(), Some("fts5"));
    let tokens = v["tokenization"]["fts_tokens"]
        .as_array()
        .expect("fts_tokens array");
    assert!(tokens.iter().any(|t| t.as_str() == Some("compute")));
    assert_eq!(
        v["tokenization"]["fts_phrase"].as_str(),
        Some("\"compute\"")
    );
    assert_eq!(v["regex_plan"]["valid"].as_bool(), Some(true));
}

#[test]
fn explain_query_missing_input_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("explain_query", Some(&args), "/repo", &fixture.db_path)
        .expect("missing input must return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
}

#[test]
fn explain_query_validates_invalid_regex() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "[invalid", "output_format": "json" });
    let result = call("explain_query", Some(&args), "/repo", &fixture.db_path)
        .expect("invalid regex must return tool error result");

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert!(
        result["structuredContent"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("invalid regex"))
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

    assert_eq!(
        v["normalized_query"]["search_path"].as_str(),
        Some("regex_structural_scan")
    );
    assert_eq!(v["regex_plan"]["valid"].as_bool(), Some(true));
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
fn explain_query_reports_active_query_mode_and_ranking_factors() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "fuzzy": true, "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(
        v["normalized_query"]["active_query_mode"].as_str(),
        Some("fts5")
    );
    let factors = v["normalized_query"]["ranking_factors"]
        .as_array()
        .expect("ranking_factors array");
    assert!(
        factors
            .iter()
            .any(|f| f.as_str() == Some("fuzzy_edit_distance_boost"))
    );
}

#[test]
fn explain_query_matches_include_ranking_evidence() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp.clone());
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let first = &v["matches"].as_array().expect("matches")[0];
    assert!(first.get("ranking_evidence").is_some());
    assert!(resp.get("atlas_ranking_evidence_legend").is_some());
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
    assert_eq!(
        v["normalized_query"]["filters_applied"]["subpath"].as_bool(),
        Some(true)
    );
    assert_eq!(v["input"]["subpath"].as_str(), Some("src/auth"));
}
