use super::*;

#[test]
fn get_context_response_includes_detail_controls_metadata() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let controls = resp
        .get("structuredContent")
        .and_then(|v| v.get("detail_controls"))
        .expect("atlas_detail_controls");
    assert!(
        controls.get("code_spans").is_some(),
        "code_spans key present"
    );
    assert!(controls.get("tests").is_some(), "tests key present");
    assert!(controls.get("imports").is_some(), "imports key present");
    assert!(controls.get("neighbors").is_some(), "neighbors key present");
    assert!(controls.get("semantic").is_some(), "semantic key present");
    assert!(
        controls.get("omitted_sections").is_some(),
        "omitted_sections key present"
    );
}

#[test]
fn get_context_default_omits_tests_code_spans_neighbors() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let controls = &resp["structuredContent"]["detail_controls"];
    let omitted = controls["omitted_sections"].as_array().expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(omitted_strs.contains(&"tests"), "tests omitted by default");
    assert!(
        omitted_strs.contains(&"code_spans"),
        "code_spans omitted by default"
    );
    assert!(
        omitted_strs.contains(&"neighbors"),
        "neighbors omitted by default"
    );
}

#[test]
fn get_context_tests_toggle_enables_test_nodes() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "tests": true,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let controls = &resp["structuredContent"]["detail_controls"];
    assert_eq!(controls["tests"].as_bool(), Some(true));
    // omitted_sections must NOT contain "tests" when enabled
    let omitted = controls["omitted_sections"].as_array().expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(
        !omitted_strs.contains(&"tests"),
        "tests not omitted when enabled"
    );
    // test node must appear in result
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let nodes = v["nodes"].as_array().expect("nodes array");
    let has_test = nodes.iter().any(|n| {
        n.get("qn")
            .and_then(|q| q.as_str())
            .map(|q| q.contains("compute_test"))
            .unwrap_or(false)
    });
    assert!(has_test, "test node must appear when tests=true");
}

#[test]
fn get_context_tests_false_excludes_test_nodes() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "tests": false,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let nodes = v["nodes"].as_array().expect("nodes array");
    let has_test = nodes.iter().any(|n| {
        n.get("qn")
            .and_then(|q| q.as_str())
            .map(|q| q.contains("compute_test"))
            .unwrap_or(false)
    });
    assert!(!has_test, "test node must not appear when tests=false");
}

#[test]
fn get_context_code_spans_toggle_controls_line_ranges() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "render".to_owned(),
        qualified_name: "src/ui.rs::fn::render".to_owned(),
        file_path: "src/ui.rs".to_owned(),
        line_start: 10,
        line_end: 20,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h1".to_owned(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph("src/ui.rs", "h1", Some("rust"), Some(20), &[node], &[])
        .expect("replace_file_graph");

    let with_spans = serde_json::json!({
        "target": { "kind": "file", "file": "src/ui.rs" },
        "code_spans": true,
        "output_format": "json"
    });
    let resp_with = call("get_context", Some(&with_spans), "/ignored", &db_path).expect("call");
    assert_eq!(
        resp_with["structuredContent"]["detail_controls"]["code_spans"].as_bool(),
        Some(true)
    );

    let without_spans = serde_json::json!({
        "target": { "kind": "file", "file": "src/ui.rs" },
        "code_spans": false,
        "output_format": "json"
    });
    let resp_without =
        call("get_context", Some(&without_spans), "/ignored", &db_path).expect("call");
    assert_eq!(
        resp_without["structuredContent"]["detail_controls"]["code_spans"].as_bool(),
        Some(false)
    );
    let omitted = resp_without["structuredContent"]["detail_controls"]["omitted_sections"]
        .as_array()
        .expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(
        omitted_strs.contains(&"code_spans"),
        "code_spans in omitted_sections when disabled"
    );
}

#[test]
fn get_context_imports_false_reflected_in_controls() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "imports": false, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["structuredContent"]["detail_controls"]["imports"].as_bool(),
        Some(false)
    );
}

#[test]
fn get_context_neighbors_toggle_reflected_in_controls() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "neighbors": true, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["structuredContent"]["detail_controls"]["neighbors"].as_bool(),
        Some(true)
    );
    let omitted = resp["structuredContent"]["detail_controls"]["omitted_sections"]
        .as_array()
        .expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(
        !omitted_strs.contains(&"neighbors"),
        "neighbors not omitted when enabled"
    );
}

#[test]
fn get_context_max_files_reflected_in_controls() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "max_files": 3, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["structuredContent"]["detail_controls"]["max_files"].as_u64(),
        Some(3),
        "max_files echoed in controls"
    );
}

#[test]
fn get_context_combined_toggles_applied_correctly() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "tests": true,
        "code_spans": true,
        "neighbors": true,
        "imports": false,
        "max_files": 5,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let controls = &resp["structuredContent"]["detail_controls"];
    assert_eq!(controls["tests"].as_bool(), Some(true));
    assert_eq!(controls["code_spans"].as_bool(), Some(true));
    assert_eq!(controls["neighbors"].as_bool(), Some(true));
    assert_eq!(controls["imports"].as_bool(), Some(false));
    assert_eq!(controls["max_files"].as_u64(), Some(5));
    // none of the enabled sections should appear in omitted_sections
    let omitted = controls["omitted_sections"].as_array().expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(!omitted_strs.contains(&"tests"));
    assert!(!omitted_strs.contains(&"code_spans"));
    assert!(!omitted_strs.contains(&"neighbors"));
}

#[test]
fn get_context_semantic_flag_reflected_in_controls() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "semantic": true, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["structuredContent"]["detail_controls"]["semantic"].as_bool(),
        Some(true),
        "semantic flag echoed in controls"
    );
}

#[test]
fn get_context_files_with_max_files_cap_respected() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "target": { "kind": "files", "files": ["src/service.rs", "src/api.rs"] },
        "max_files": 1,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["structuredContent"]["detail_controls"]["max_files"].as_u64(),
        Some(1),
        "max_files echoed in controls for files-mode request"
    );
}

#[test]
fn get_context_json_requests_and_default_output_both_include_controls() {
    let fixture = setup_mcp_fixture();

    let json_args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "tests": true
    });
    let json_resp = call(
        "get_context",
        Some(&json_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("call json");
    assert!(
        json_resp
            .get("structuredContent")
            .and_then(|v| v.get("detail_controls"))
            .is_some(),
        "json: controls present"
    );

    let default_args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "tests": true
    });
    let toon_resp = call(
        "get_context",
        Some(&default_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("call default");
    assert!(
        toon_resp
            .get("structuredContent")
            .and_then(|v| v.get("detail_controls"))
            .is_some(),
        "default: controls present"
    );
}

#[test]
fn get_context_applies_mcp_response_byte_cap() {
    let fixture = setup_mcp_fixture();
    let atlas_dir = fixture._dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).expect("create atlas dir");
    std::fs::write(
        atlas_dir.join("config.toml"),
        "[mcp]\nworker_threads = 2\ntool_timeout_ms = 300000\nmax_mcp_response_bytes = 1200\n",
    )
    .expect("write config");

    let repo_root = fixture._dir.path().to_string_lossy().to_string();
    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "tests": true,
        "neighbors": true,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), &repo_root, &fixture.db_path).expect("call");

    let value = resp["structuredContent"].clone();
    assert_eq!(value["truncated"].as_bool(), Some(true));
    assert!(
        value["node_count"].as_u64().is_some(),
        "response must keep summary fields"
    );
    assert!(
        value["nodes"].as_array().is_some(),
        "response must keep nodes array shape"
    );
    assert!(
        value["payload_truncation"]["omitted_byte_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "response cap must report omitted bytes"
    );
    assert_eq!(resp["budget_status"], "partial_result");
    assert_eq!(
        resp["budget_name"],
        "mcp_cli_payload_serialization.max_mcp_response_bytes"
    );
    assert_eq!(resp["safe_to_answer"], true);
}

#[test]
fn get_context_fails_closed_when_mcp_response_budget_is_impossible() {
    let fixture = setup_mcp_fixture();
    let atlas_dir = fixture._dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).expect("create atlas dir");
    std::fs::write(
        atlas_dir.join("config.toml"),
        "[mcp]\nworker_threads = 2\ntool_timeout_ms = 300000\nmax_mcp_response_bytes = 64\n",
    )
    .expect("write config");

    let repo_root = fixture._dir.path().to_string_lossy().to_string();
    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "output_format": "json"
    });
    let err = call("get_context", Some(&args), &repo_root, &fixture.db_path)
        .expect("response budget under impossible cap must return tool error result");
    assert_eq!(err["isError"], serde_json::json!(true));
    assert!(
        err["structuredContent"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("max_mcp_response_bytes")),
        "unexpected error payload: {err:#}"
    );
}
