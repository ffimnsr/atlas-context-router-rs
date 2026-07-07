use super::*;
use atlas_reasoning::{
    InsightsEngine, LargeFunctionMode, LargeFunctionRequest, RiskAssessmentTarget,
};

#[test]
fn analyze_safety_returns_score_and_band() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "symbol": "src/service.rs::fn::compute", "output_format": "json" });
    let resp = call("analyze_safety", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_safety call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(v["safety_score"].as_f64().is_some());
    assert!(v["safety_band"].as_str().is_some());
    assert!(v["fan_in"].as_i64().is_some());
    assert!(v["fan_out"].as_i64().is_some());
    assert!(v["linked_tests"].as_i64().is_some());
    assert!(
        v["coverage_strength"].as_str().is_some(),
        "coverage_strength missing"
    );
    assert!(v["reasons"].as_array().is_some());
    assert!(v["suggested_validations"].as_array().is_some());
    assert!(v["evidence"].as_array().is_some());
}

#[test]
fn analyze_safety_missing_symbol_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("analyze_safety", Some(&args), "/repo", &fixture.db_path)
        .expect("missing symbol should return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
}

#[test]
fn analyze_safety_unknown_symbol_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "symbol": "nonexistent::fn::ghost", "output_format": "json" });
    let result = call("analyze_safety", Some(&args), "/repo", &fixture.db_path)
        .expect("unknown symbol should return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
}

#[test]
fn analyze_safety_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "symbol": "src/service.rs::fn::compute" });
    let resp = call("analyze_safety", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_safety call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn analyze_safety_normalizes_alias_qname() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "symbol": "src/service.rs::function::compute",
        "output_format": "json"
    });
    let resp = call("analyze_safety", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_safety call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["symbol"], "src/service.rs::fn::compute");
}

#[test]
fn analyze_remove_returns_impact_summary() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "symbols": ["src/service.rs::fn::compute"], "output_format": "json" });
    let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_remove call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(v["seed_count"].as_i64().is_some());
    assert!(v["impacted_symbol_count"].as_i64().is_some());
    assert!(v["impacted_file_count"].as_i64().is_some());
    assert!(v["impacted_test_count"].as_i64().is_some());
    assert!(v["impacted_symbols"].as_array().is_some());
    assert!(v["impacted_files"].as_array().is_some());
    assert!(v["omitted_symbol_count"].as_i64().is_some());
    assert!(v["warnings"].as_array().is_some());
    assert!(v["uncertainty_flags"].as_array().is_some());
    assert!(v["evidence"].as_array().is_some());
}

#[test]
fn analyze_remove_empty_symbols_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "symbols": [], "output_format": "json" });
    let result = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
        .expect("empty symbols should return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
}

#[test]
fn analyze_remove_unresolved_seed_returns_warnings() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "symbols": ["nonexistent::fn::ghost"], "output_format": "json" });
    let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_remove should not hard-error for unresolved seeds");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let warnings = v["warnings"].as_array().expect("warnings array");
    let flags = v["uncertainty_flags"]
        .as_array()
        .expect("uncertainty_flags");
    assert!(!warnings.is_empty() || !flags.is_empty());
}

#[test]
fn analyze_remove_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "symbols": ["src/service.rs::fn::compute"] });
    let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_remove call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn analyze_remove_normalizes_alias_qname() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "symbols": ["src/service.rs::function::compute"],
        "output_format": "json"
    });
    let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_remove call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(
        v["impacted_symbols"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|node| node["qn"] == "src/api.rs::fn::handle_request")
    );
}

#[test]
fn analyze_dead_code_returns_candidate_list() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dead_code call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(v["candidate_count"].as_i64().is_some());
    assert!(v["omitted_count"].as_i64().is_some());
    assert!(v["candidates"].as_array().is_some());
    assert!(v["applied_limit"].as_i64().is_some());
}

#[test]
fn analyze_dead_code_subpath_is_echoed() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "subpath": "src", "output_format": "json" });
    let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dead_code call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(v["applied_subpath"].as_str(), Some("src"));
}

#[test]
fn analyze_dead_code_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({});
    let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dead_code call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn analyze_dead_code_summary_mode_returns_count_only() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "summary": true, "output_format": "json" });
    let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dead_code summary call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(
        v["candidate_count"].as_i64().is_some(),
        "summary must include candidate_count"
    );
    assert!(
        v.get("candidates").is_none(),
        "summary must NOT include candidates list"
    );
}

#[test]
fn analyze_dead_code_exclude_kind_echoed_in_response() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "exclude_kind": ["constant"], "output_format": "json" });
    let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dead_code exclude_kind call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let excluded = v["excluded_kinds"]
        .as_array()
        .expect("excluded_kinds must be array");
    assert!(
        excluded.iter().any(|k| k.as_str() == Some("constant")),
        "excluded_kinds must echo back 'constant'"
    );
}

#[test]
fn analyze_remove_response_includes_compact_file_and_edge_omit_counts() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "symbols": ["src/service.rs::fn::compute"], "output_format": "json" });
    let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_remove call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(
        v["omitted_file_count"].as_i64().is_some(),
        "must include omitted_file_count"
    );
    assert!(
        v["omitted_edge_count"].as_i64().is_some(),
        "must include omitted_edge_count"
    );
}

#[test]
fn analyze_remove_max_files_caps_impacted_files_list() {
    let fixture = setup_mcp_fixture();
    // max_files=1 with multiple impacted files should cap the list.
    let args = serde_json::json!({
        "symbols": ["src/service.rs::fn::compute"],
        "max_files": 1,
        "output_format": "json"
    });
    let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_remove max_files call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let files = v["impacted_files"]
        .as_array()
        .expect("impacted_files array");
    assert!(
        files.len() <= 1,
        "max_files=1 must cap impacted_files list to at most 1"
    );
}

#[test]
fn analyze_remove_max_nodes_is_clamped_by_central_budget_policy() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "symbols": ["src/service.rs::fn::compute"],
        "max_nodes": 9999,
        "output_format": "json"
    });
    let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_remove call");

    assert_eq!(resp["budget_status"], "override_clamped");
    assert_eq!(resp["budget_hit"], true);
    assert_eq!(resp["budget_limit"], 1000);
    assert_eq!(resp["budget_observed"], 9999);
}

#[test]
fn analyze_dependency_returns_removable_verdict() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "symbol": "src/service.rs::fn::compute", "output_format": "json" });
    let resp = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dependency call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(v["removable"].as_bool().is_some());
    assert!(v["confidence"].as_str().is_some());
    assert!(v["blocking_reference_count"].as_i64().is_some());
    assert!(v["blocking_references"].as_array().is_some());
    assert!(v["omitted_blocking_count"].as_i64().is_some());
    assert!(v["suggested_cleanups"].as_array().is_some());
    assert!(v["uncertainty_flags"].as_array().is_some());
    assert!(v["evidence"].as_array().is_some());
}

#[test]
fn analyze_dependency_missing_symbol_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path)
        .expect("missing symbol should return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
}

#[test]
fn analyze_dependency_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "symbol": "src/service.rs::fn::compute" });
    let resp = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dependency call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn analyze_dependency_normalizes_alias_qname() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "symbol": "src/service.rs::function::compute",
        "output_format": "json"
    });
    let resp = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dependency call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["symbol"], "src/service.rs::fn::compute");
}

#[test]
fn find_large_functions_matches_direct_report_json() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "threshold": 2,
        "mode": "large",
        "output_format": "json"
    });
    let resp = call(
        "find_large_functions",
        Some(&args),
        "/repo",
        &fixture.db_path,
    )
    .expect("find_large_functions call");
    let text = unwrap_tool_text(resp.clone());
    let tool_value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let store = Store::open(&fixture.db_path).expect("open store");
    let engine = InsightsEngine::new(&store, atlas_engine::config::InsightsConfig::default())
        .expect("insights engine");
    let direct = engine
        .find_large_functions(
            "/repo",
            LargeFunctionRequest {
                threshold: Some(2),
                mode: LargeFunctionMode::Large,
                ..Default::default()
            },
        )
        .expect("direct analysis");
    let mut direct_value = serde_json::to_value(&direct.report).expect("serialize report");
    direct_value["summary"]["generated_at"] = tool_value["summary"]["generated_at"].clone();

    assert_eq!(tool_value, direct_value);
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn analyze_architecture_matches_direct_report_json() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let resp = call(
        "analyze_architecture",
        Some(&args),
        "/repo",
        &fixture.db_path,
    )
    .expect("analyze_architecture call");
    let text = unwrap_tool_text(resp.clone());
    let tool_value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let store = Store::open(&fixture.db_path).expect("open store");
    let engine = InsightsEngine::new(&store, atlas_engine::config::InsightsConfig::default())
        .expect("insights engine");
    let direct = engine
        .analyze_architecture("/repo")
        .expect("direct analysis");
    let mut direct_value = serde_json::to_value(&direct.report).expect("serialize report");
    direct_value["summary"]["generated_at"] = tool_value["summary"]["generated_at"].clone();

    assert_eq!(tool_value, direct_value);
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn analyze_metrics_matches_direct_report_json() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("analyze_metrics", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_metrics call");
    let text = unwrap_tool_text(resp.clone());
    let tool_value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let store = Store::open(&fixture.db_path).expect("open store");
    let engine = InsightsEngine::new(&store, atlas_engine::config::InsightsConfig::default())
        .expect("insights engine");
    let direct = engine.analyze_metrics("/repo").expect("direct analysis");
    let mut direct_value = serde_json::to_value(&direct.report).expect("serialize report");
    direct_value["summary"]["generated_at"] = tool_value["summary"]["generated_at"].clone();

    assert_eq!(tool_value, direct_value);
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn assess_risk_matches_direct_report_json() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "symbol": "src/service.rs::fn::compute",
        "output_format": "json"
    });
    let resp =
        call("assess_risk", Some(&args), "/repo", &fixture.db_path).expect("assess_risk call");
    let text = unwrap_tool_text(resp.clone());
    let tool_value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let store = Store::open(&fixture.db_path).expect("open store");
    let engine = InsightsEngine::new(&store, atlas_engine::config::InsightsConfig::default())
        .expect("insights engine");
    let direct = engine
        .assess_risk(
            "/repo",
            RiskAssessmentTarget::Symbol {
                symbol: "src/service.rs::fn::compute".to_owned(),
            },
        )
        .expect("direct analysis");
    let mut direct_value = serde_json::to_value(&direct.report).expect("serialize report");
    direct_value["summary"]["generated_at"] = tool_value["summary"]["generated_at"].clone();

    assert_eq!(tool_value, direct_value);
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn analyze_patterns_matches_direct_report_json() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("analyze_patterns", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_patterns call");
    let text = unwrap_tool_text(resp.clone());
    let tool_value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let store = Store::open(&fixture.db_path).expect("open store");
    let engine = InsightsEngine::new(&store, atlas_engine::config::InsightsConfig::default())
        .expect("insights engine");
    let direct = engine.analyze_patterns().expect("direct analysis");
    let mut direct_value = serde_json::to_value(&direct).expect("serialize report");
    direct_value["summary"]["generated_at"] = tool_value["summary"]["generated_at"].clone();

    assert_eq!(tool_value, direct_value);
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn find_complex_functions_matches_direct_report_json() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "complexity_threshold": 1,
        "output_format": "json"
    });
    let resp = call(
        "find_complex_functions",
        Some(&args),
        "/repo",
        &fixture.db_path,
    )
    .expect("find_complex_functions call");
    let text = unwrap_tool_text(resp.clone());
    let tool_value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let store = Store::open(&fixture.db_path).expect("open store");
    let engine = InsightsEngine::new(&store, atlas_engine::config::InsightsConfig::default())
        .expect("insights engine");
    let direct = engine
        .find_large_functions(
            "/repo",
            LargeFunctionRequest {
                complexity_threshold: Some(1),
                mode: LargeFunctionMode::Complex,
                ..Default::default()
            },
        )
        .expect("direct analysis");
    let mut direct_value = serde_json::to_value(&direct.report).expect("serialize report");
    direct_value["summary"]["generated_at"] = tool_value["summary"]["generated_at"].clone();

    assert_eq!(tool_value, direct_value);
    assert_provenance(&resp, "/repo", &fixture.db_path);
}
