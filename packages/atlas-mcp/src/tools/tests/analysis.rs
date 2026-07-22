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
    assert!(v["test_adjacency"]["linked_test_count"].as_i64().is_some());
    assert!(
        v["test_adjacency"]["coverage_strength"].as_str().is_some(),
        "coverage_strength missing"
    );
    assert!(v["summary"].as_object().is_some());
    assert!(v["suggested_validations"].as_array().is_some());
    assert!(v["factor_evidence"].as_array().is_some());
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

    assert_eq!(v["symbol"]["qname"], "src/service.rs::fn::compute");
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

    assert!(v["symbols"].as_array().is_some());
    assert!(v["definite_impacts"].as_array().is_some());
    assert!(v["probable_impacts"].as_array().is_some());
    assert!(v["weak_impacts"].as_array().is_some());
    assert!(v["tests"].as_array().is_some());
    assert!(v["summary"]["seed_count"].as_i64().is_some());
    assert!(v["summary"]["impacted_symbol_count"].as_i64().is_some());
    assert!(v["summary"]["impacted_file_count"].as_i64().is_some());
    assert!(v["warnings"].as_array().is_some());
    assert!(v["uncertainty_flags"].as_array().is_some());
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
        v["definite_impacts"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|node| node["symbol"]["qname"] == "src/api.rs::fn::handle_request")
            || v["probable_impacts"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .any(|node| node["symbol"]["qname"] == "src/api.rs::fn::handle_request")
            || v["weak_impacts"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .any(|node| node["symbol"]["qname"] == "src/api.rs::fn::handle_request")
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

    assert!(v["scope"].as_object().is_some());
    assert!(v["candidates"].as_array().is_some());
    assert!(v["blockers"].as_array().is_some());
    assert!(v["summary"]["candidate_count"].as_i64().is_some());
}

#[test]
fn analyze_dead_code_subpath_is_echoed() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "subpath": "src", "output_format": "json" });
    let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dead_code call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(v["scope"]["subpath"].as_str(), Some("src"));
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
        v["summary"]["candidate_count"].as_i64().is_some(),
        "summary must include candidate_count"
    );
    assert_eq!(v["candidates"], serde_json::json!([]));
}

#[test]
fn analyze_dead_code_exclude_kind_echoed_in_response() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "exclude_kind": ["constant"], "output_format": "json" });
    let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dead_code exclude_kind call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let excluded = v["scope"]["excluded_kinds"]
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
        v["summary"]["omitted_file_count"].as_i64().is_some(),
        "must include omitted_file_count"
    );
    assert!(
        v["summary"]["omitted_edge_count"].as_i64().is_some(),
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

    assert_eq!(
        v["summary"]["omitted_file_count"].as_i64(),
        Some(1),
        "max_files=1 should omit one impacted file in fixture"
    );
    assert_eq!(
        v["summary"]["impacted_file_count"].as_i64(),
        Some(2),
        "fixture should still report full impacted_file_count"
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
    assert!(v["confidence_tier"].as_str().is_some());
    assert!(v["summary"]["blocking_reference_count"].as_i64().is_some());
    assert!(v["blocking_references"].as_array().is_some());
    assert!(v["summary"]["omitted_blocking_count"].as_i64().is_some());
    assert!(v["suggested_cleanups"].as_array().is_some());
    assert!(v["warnings"].as_array().is_some());
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
    let mut direct_value = serde_json::to_value(direct.report_result()).expect("serialize report");
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
    let mut direct_value = serde_json::to_value(direct.report_result()).expect("serialize report");
    direct_value["summary"]["generated_at"] = tool_value["summary"]["generated_at"].clone();

    assert_eq!(tool_value, direct_value);
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

fn assert_toon_structured_content_matches_direct_report(
    tool_name: &str,
    args: serde_json::Value,
    expected_report: serde_json::Value,
) {
    let fixture = setup_mcp_fixture();
    let resp = call(tool_name, Some(&args), "/repo", &fixture.db_path)
        .unwrap_or_else(|error| panic!("{tool_name} call failed: {error}"));
    assert_eq!(
        resp.pointer("/_meta/atlas:requestedOutputFormat")
            .and_then(|value| value.as_str()),
        Some("toon")
    );

    let mut expected = expected_report;
    expected["summary"]["generated_at"] =
        resp["structuredContent"]["summary"]["generated_at"].clone();
    expected["atlas_provenance"]["last_indexed_at"] =
        resp["structuredContent"]["atlas_provenance"]["last_indexed_at"].clone();
    assert_eq!(resp["structuredContent"], expected);
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn find_large_functions_changed_code_file_emits_freshness_warning() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 99 }\n",
    );
    let args = serde_json::json!({
        "threshold": 1,
        "mode": "large",
        "output_format": "json"
    });

    let resp = call(
        "find_large_functions",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("find_large_functions");

    assert_eq!(
        resp.pointer("/atlas_freshness/stale")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        resp.pointer("/atlas_freshness/stale_result_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn r4_insight_tools_keep_full_report_in_toon_structured_content() {
    let fixture = setup_mcp_fixture();
    let store = Store::open(&fixture.db_path).expect("open store");
    let engine = InsightsEngine::new(&store, atlas_engine::config::InsightsConfig::default())
        .expect("insights engine");

    let architecture = serde_json::to_value(
        &engine
            .analyze_architecture("/repo")
            .expect("architecture direct")
            .report,
    )
    .expect("serialize architecture report");
    assert_toon_structured_content_matches_direct_report(
        "analyze_architecture",
        serde_json::json!({ "output_format": "toon" }),
        architecture,
    );

    let metrics = serde_json::to_value(
        &engine
            .analyze_metrics("/repo")
            .expect("metrics direct")
            .report,
    )
    .expect("serialize metrics report");
    assert_toon_structured_content_matches_direct_report(
        "analyze_metrics",
        serde_json::json!({ "output_format": "toon" }),
        metrics,
    );

    let risk = serde_json::to_value(
        &engine
            .assess_risk(
                "/repo",
                RiskAssessmentTarget::Symbol {
                    symbol: "src/service.rs::fn::compute".to_owned(),
                },
            )
            .expect("risk direct")
            .report,
    )
    .expect("serialize risk report");
    assert_toon_structured_content_matches_direct_report(
        "assess_risk",
        serde_json::json!({ "symbol": "src/service.rs::fn::compute", "output_format": "toon" }),
        risk,
    );

    let patterns = serde_json::to_value(engine.analyze_patterns().expect("patterns direct"))
        .expect("serialize pattern report");
    assert_toon_structured_content_matches_direct_report(
        "analyze_patterns",
        serde_json::json!({ "output_format": "toon" }),
        patterns,
    );

    let large = serde_json::to_value(
        engine
            .find_large_functions(
                "/repo",
                LargeFunctionRequest {
                    threshold: Some(2),
                    mode: LargeFunctionMode::Large,
                    ..Default::default()
                },
            )
            .expect("large direct")
            .report_result(),
    )
    .expect("serialize large report");
    assert_toon_structured_content_matches_direct_report(
        "find_large_functions",
        serde_json::json!({ "threshold": 2, "mode": "large", "output_format": "toon" }),
        large,
    );

    let complex = serde_json::to_value(
        engine
            .find_large_functions(
                "/repo",
                LargeFunctionRequest {
                    complexity_threshold: Some(1),
                    mode: LargeFunctionMode::Complex,
                    ..Default::default()
                },
            )
            .expect("complex direct")
            .report_result(),
    )
    .expect("serialize complex report");
    assert_toon_structured_content_matches_direct_report(
        "find_complex_functions",
        serde_json::json!({ "complexity_threshold": 1, "output_format": "toon" }),
        complex,
    );
}
