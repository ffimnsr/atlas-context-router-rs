use super::*;

#[test]
fn symbol_neighbors_limit_is_clamped_by_central_budget_policy() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "qname": "src/service.rs::fn::compute",
        "limit": 9999,
        "output_format": "json"
    });

    let response =
        call("symbol_neighbors", Some(&args), "/repo", &fixture.db_path).expect("neighbors");

    assert_eq!(response["budget_status"], "override_clamped");
    assert_eq!(response["budget_hit"], true);
    assert_eq!(
        response["budget_name"],
        "review_context_extraction.max_nodes"
    );
    assert_eq!(response["budget_limit"], 200);
    assert_eq!(response["budget_observed"], 9999);
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
    let source_repo_id = atlas_repo::stable_repo_id(camino::Utf8Path::new(&fixture.repo_root));
    store
        .replace_file_graph_for_repo(
            &source_repo_id,
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
        value.pointer("/symbol/qname").and_then(|v| v.as_str()),
        Some("src/service.rs::fn::compute")
    );
    assert_eq!(
        value.pointer("/callers/0/qn").and_then(|v| v.as_str()),
        Some("src/api.rs::fn::handle_request")
    );
    assert_eq!(value["call_sites"].as_array().map(|v| v.len()), Some(2));
    assert_eq!(
        value.pointer("/call_sites/0/from").and_then(|v| v.as_str()),
        Some("src/api.rs::fn::handle_request")
    );
    assert_eq!(
        value.pointer("/call_sites/0/to").and_then(|v| v.as_str()),
        Some("src/service.rs::fn::compute")
    );
    assert_eq!(
        value.pointer("/call_sites/0/file").and_then(|v| v.as_str()),
        Some("src/api.rs")
    );
    assert_eq!(
        value.pointer("/call_sites/0/line").and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        value.pointer("/call_sites/1/line").and_then(|v| v.as_u64()),
        Some(2)
    );
}

#[test]
fn symbol_neighbors_normalizes_alias_qname() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "qname": "src/service.rs::function::compute",
        "output_format": "json"
    });

    let response = call(
        "symbol_neighbors",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("symbol_neighbors call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(value["symbol"]["qname"], "src/service.rs::fn::compute");
    assert_eq!(
        value.pointer("/callers/0/qn").and_then(|v| v.as_str()),
        Some("src/api.rs::fn::handle_request")
    );
}

#[test]
fn traverse_graph_normalizes_alias_qname() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "from_qn": "src/service.rs::function::compute",
        "output_format": "json"
    });

    let response = call("traverse_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("traverse_graph call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(value["nodes"][0]["qn"], "src/service.rs::fn::compute");
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
fn traverse_graph_depth_one_emits_only_root_adjacent_deduped_edges() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "from_qn": "src/service.rs::fn::compute",
        "max_depth": 1,
        "max_nodes": 10,
        "output_format": "json"
    });
    let resp = call("traverse_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("traverse_graph call");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    let root = "src/service.rs::fn::compute";
    let edges = v["edges"].as_array().expect("edges array");
    let mut seen = std::collections::HashSet::new();
    for edge in edges {
        assert_eq!(edge["direction"], json!("inbound"));
        assert!(edge["from"] == json!(root) || edge["to"] == json!(root));
        let key = (
            edge["from"].as_str().unwrap_or_default().to_owned(),
            edge["kind"].as_str().unwrap_or_default().to_owned(),
            edge["to"].as_str().unwrap_or_default().to_owned(),
        );
        assert!(seen.insert(key), "duplicate edge emitted: {edge:?}");
    }
    let node_qns: Vec<&str> = v["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .filter_map(|node| node["qn"].as_str())
        .collect();
    for edge in edges {
        let from = edge["from"].as_str().unwrap_or_default();
        let to = edge["to"].as_str().unwrap_or_default();
        assert!(
            node_qns.contains(&from) && node_qns.contains(&to),
            "edge endpoint missing from nodes[]: {edge:?}"
        );
    }
}

#[test]
fn traverse_graph_reports_budget_pressure_when_node_cap_is_reached() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "from_qn": "src/service.rs::fn::compute",
        "max_depth": 2,
        "max_nodes": 2,
        "output_format": "json"
    });
    let resp = call("traverse_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("traverse_graph call");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(v["traversal_budget"]["accepted_node_budget"], json!(2));
    assert_eq!(v["traversal_budget"]["budget_hit"], json!(true));
    assert_eq!(v["budget_hit"], json!(true));
    assert_eq!(v["truncated"], json!(true));
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
    assert_eq!(
        resp["structuredContent"]["lookup"]["error_code"].as_str(),
        Some("node_not_found")
    );
    assert_eq!(
        resp["structuredContent"]["summary"]["status"],
        json!("node_not_found")
    );
    assert_eq!(resp["structuredContent"]["callers"], json!([]));
    assert_error_code_doc_link(
        resp["structuredContent"]["lookup"]["error_code_docs"]
            .as_str()
            .expect("lookup.error_code_docs"),
        "node_not_found",
    );
    assert!(
        resp["structuredContent"]["lookup"]["message"]
            .as_str()
            .is_some()
    );
    let suggestions = resp["structuredContent"]["lookup"]["suggestions"]
        .as_array()
        .expect("lookup.suggestions");
    assert!(!suggestions.is_empty());
}

#[test]
fn cross_file_links_returns_stable_shape_for_linked_and_isolated_files() {
    let fixture = setup_mcp_fixture();
    let linked_args = serde_json::json!({ "file": "src/service.rs", "output_format": "json" });
    let linked_resp = call(
        "cross_file_links",
        Some(&linked_args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("cross_file_links linked");
    let linked: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(linked_resp)).expect("parse json");
    assert_eq!(linked["source_file"], json!("src/service.rs"));
    assert!(linked["linked_files"].as_array().is_some());
    assert!(linked["coupling_metric"].as_object().is_some());
    assert!(linked["summary"].as_object().is_some());

    let isolated_args =
        serde_json::json!({ "file": "tests/service_test.rs", "output_format": "json" });
    let isolated_resp = call(
        "cross_file_links",
        Some(&isolated_args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("cross_file_links isolated");
    let isolated: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(isolated_resp)).expect("parse json");
    assert_eq!(isolated["source_file"], json!("tests/service_test.rs"));
    assert!(isolated["linked_files"].as_array().is_some());
}

#[test]
fn cross_file_links_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "file": "src/service.rs", "output_format": "json" });
    let resp = call(
        "cross_file_links",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("cross_file_links");
    assert_provenance(&resp, &fixture.repo_root, &fixture.db_path);
}

#[test]
fn concept_clusters_returns_stable_shape_for_present_and_empty_clusters() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" });
    let resp = call(
        "concept_clusters",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("concept_clusters");
    let value: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(value["seed_files"], json!(["src/service.rs"]));
    assert!(value["clusters"].as_array().is_some());
    assert!(value["summary"].as_object().is_some());
    assert!(value["truncated"].as_bool().is_some());

    let empty_args =
        serde_json::json!({ "files": ["tests/service_test.rs"], "output_format": "json" });
    let empty_resp = call(
        "concept_clusters",
        Some(&empty_args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("concept_clusters empty");
    let empty_value: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(empty_resp)).expect("parse json");
    assert_eq!(empty_value["seed_files"], json!(["tests/service_test.rs"]));
    assert!(empty_value["clusters"].as_array().is_some());
}

#[test]
fn concept_clusters_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" });
    let resp = call(
        "concept_clusters",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("concept_clusters");
    assert_provenance(&resp, &fixture.repo_root, &fixture.db_path);
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
