use super::*;

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
    assert!(v["reasons"].as_array().is_some());
    assert!(v["suggested_validations"].as_array().is_some());
    assert!(v["evidence"].as_array().is_some());
}

#[test]
fn analyze_safety_missing_symbol_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("analyze_safety", Some(&args), "/repo", &fixture.db_path);
    assert!(result.is_err());
}

#[test]
fn analyze_safety_unknown_symbol_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "symbol": "nonexistent::fn::ghost", "output_format": "json" });
    let result = call("analyze_safety", Some(&args), "/repo", &fixture.db_path);
    assert!(result.is_err());
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
    let result = call("analyze_remove", Some(&args), "/repo", &fixture.db_path);
    assert!(result.is_err());
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
    let result = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path);
    assert!(result.is_err());
}

#[test]
fn analyze_dependency_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "symbol": "src/service.rs::fn::compute" });
    let resp = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path)
        .expect("analyze_dependency call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}
