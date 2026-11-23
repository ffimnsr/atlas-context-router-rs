use super::*;

#[test]
fn resolve_symbol_finds_exact_match() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "name": "compute", "file": "src/service.rs", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(
        v["best_match"]["qualified_name"].as_str(),
        Some("src/service.rs::fn::compute")
    );
    assert!(v["summary"]["match_count"].as_i64().unwrap_or(0) >= 1);
    let matches = v["ambiguity"]["matches"].as_array().expect("matches array");
    assert!(!matches.is_empty());
    assert_eq!(matches[0]["kind"].as_str(), Some("function"));
    assert_eq!(matches[0]["file_path"].as_str(), Some("src/service.rs"));
}

#[test]
fn resolve_symbol_missing_name_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("missing name must return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
}

#[test]
fn resolve_symbol_empty_name_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "", "output_format": "json" });
    let result = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("empty name must return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
}

#[test]
fn resolve_symbol_no_match_returns_zero_match_contract() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "nonexistent_symbol_xyz", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    assert_ne!(resp["isError"], json!(true));
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(v["summary"]["status"], json!("not_found"));
    assert_eq!(v["summary"]["match_count"], json!(0));
    assert!(v["best_match"].is_null());
    assert_eq!(v["resolution"]["stage"], json!(null));
}

#[test]
fn resolve_symbol_name_file_kind_is_not_ambiguous() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "name": "compute",
        "file": "src/service.rs",
        "kind": "fn",
        "output_format": "json"
    });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(v["summary"]["match_count"], json!(1));
    assert_eq!(v["ambiguity"]["ambiguous"], json!(false));
    assert_eq!(v["resolution"]["stage"], json!("exact_name"));
}

#[test]
fn resolve_symbol_kind_alias_fn_filters_before_ranking() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "kind": "fn", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(v["summary"]["match_count"], json!(1));
    assert_eq!(v["best_match"]["kind"], json!("function"));
}

#[test]
fn resolve_symbol_prefix_lookup_is_reported() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "handle_req", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(v["resolution"]["stage"], json!("prefix"));
    assert_eq!(
        v["best_match"]["qualified_name"].as_str(),
        Some("src/api.rs::fn::handle_request")
    );
}

#[test]
fn resolve_symbol_kind_alias_fn_resolves_to_function() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "kind": "fn", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(
        v["best_match"]["qualified_name"].as_str(),
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

    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(
        v["best_match"]["qualified_name"].as_str(),
        Some("src/api.rs::fn::handle_request")
    );
    let matches = v["ambiguity"]["matches"].as_array().expect("matches array");
    for m in matches {
        assert!(m["file_path"].as_str().unwrap_or("").contains("src/api.rs"));
    }
}

#[test]
fn resolve_symbol_returns_ambiguous_success_shape() {
    let fixture = setup_mcp_fixture();
    let mut store = Store::open(&fixture.db_path).expect("open store");
    let dupe = make_node(
        NodeKind::Function,
        "compute",
        "src/extra.rs::fn::compute",
        "src/extra.rs",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "src/extra.rs",
            "hash:src/extra.rs",
            Some("rust"),
            Some(1),
            &[dupe],
            &[],
        )
        .expect("replace extra graph");

    let args = serde_json::json!({ "name": "compute", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(v["summary"]["status"], json!("ambiguous"));
    assert!(v["best_match"].is_object() || v["best_match"].is_null());
    assert!(v["ambiguity"]["ambiguous"].as_bool().unwrap_or(false));
    assert!(v["ambiguity"]["matches"].as_array().unwrap().len() >= 2);
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
    assert!(v["summary"]["truncated"].as_bool().is_some());
}
