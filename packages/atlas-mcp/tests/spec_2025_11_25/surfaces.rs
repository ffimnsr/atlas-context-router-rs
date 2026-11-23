use super::*;

#[test]
fn spec_fixture_stdio_and_http_runtime_surfaces_match_golden() {
    let (_dir, repo_root, db_path) = setup_repo();
    build_fixture_graph(&repo_root, &db_path);
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    let session_id = http_session(&harness);

    let stdio_tools = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}),
    );
    assert_eq!(
        tool_name_snapshot(&stdio_tools),
        read_fixture("tools_list.snapshot.json")
    );
    let http_tools = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        )
        .expect("http tools/list");
    assert_eq!(
        tool_name_snapshot(http_tools.json_body.as_ref().expect("tools json body")),
        read_fixture("tools_list.snapshot.json")
    );

    let stdio_structured = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{
                "name":"list_graph_stats",
                "arguments":{}
            }
        }),
    );
    assert_eq!(
        list_graph_stats_snapshot(&stdio_structured),
        read_fixture("tools_call_structured_output.snapshot.json")
    );
    let http_structured = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"list_graph_stats",
                    "arguments":{}
                }
            }),
        )
        .expect("http tools/call list_graph_stats");
    assert_eq!(
        list_graph_stats_snapshot(
            http_structured
                .json_body
                .as_ref()
                .expect("structured json body")
        ),
        read_fixture("tools_call_structured_output.snapshot.json")
    );

    let stdio_resources = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({"jsonrpc":"2.0","id":3,"method":"resources/list","params":{}}),
    );
    assert_eq!(
        resource_list_snapshot(&stdio_resources),
        read_fixture("resources_list.snapshot.json")
    );
    let http_resources = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({"jsonrpc":"2.0","id":4,"method":"resources/list","params":{}}),
        )
        .expect("http resources/list");
    assert_eq!(
        resource_list_snapshot(
            http_resources
                .json_body
                .as_ref()
                .expect("resources json body")
        ),
        read_fixture("resources_list.snapshot.json")
    );

    let stdio_resource_read = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"resources/read",
            "params":{"uri":"atlas://health/status"}
        }),
    );
    assert_eq!(
        resource_read_status_snapshot(&stdio_resource_read),
        read_fixture("resources_read.snapshot.json")
    );
    let http_resource_read = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"resources/read",
                "params":{"uri":"atlas://health/status"}
            }),
        )
        .expect("http resources/read");
    assert_eq!(
        resource_read_status_snapshot(
            http_resource_read
                .json_body
                .as_ref()
                .expect("resource read json body")
        ),
        read_fixture("resources_read.snapshot.json")
    );

    let stdio_logging = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"logging/setLevel",
            "params":{"level":"warning"}
        }),
    );
    assert!(
        stdio_logging.get("error").is_none(),
        "stdio logging/setLevel should succeed through rmcp"
    );
    assert!(
        stdio_logging.get("result").is_some(),
        "stdio logging/setLevel should return json-rpc result"
    );
    let http_logging = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"logging/setLevel",
                "params":{"level":"warning"}
            }),
        )
        .expect("http logging/setLevel");
    assert!(
        http_logging
            .json_body
            .as_ref()
            .expect("logging json body")
            .get("error")
            .is_none(),
        "http logging/setLevel should succeed through rmcp"
    );

    assert_eq!(
        task_lifecycle_stdio_snapshot(&repo_root, &db_path),
        read_fixture("task_lifecycle.snapshot.json")
    );
}

#[test]
fn structured_tool_results_preserve_provenance_and_freshness_metadata() {
    let (_dir, repo_root, db_path) = setup_repo();
    build_fixture_graph(&repo_root, &db_path);
    write_repo_file(
        Path::new(&repo_root),
        "src/lib.rs",
        "pub fn greet() -> &'static str { \"hi updated\" }\n",
    );

    let response = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({
            "jsonrpc":"2.0",
            "id":7,
            "method":"tools/call",
            "params":{
                "name":"query_graph",
                "arguments":{"text":"greet"}
            }
        }),
    );

    let structured = &response["result"]["structuredContent"];
    assert!(
        structured["atlas_provenance"].is_object(),
        "query_graph structured content must expose atlas_provenance"
    );
    assert_eq!(
        structured["atlas_freshness"]["stale"],
        json!(true),
        "query_graph structured content must expose stale freshness warning"
    );
}

#[test]
fn spec_transports_return_equivalent_bodies_after_normalization() {
    let (_dir, repo_root, db_path) = setup_repo();
    build_fixture_graph(&repo_root, &db_path);
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    let session_id = http_session(&harness);

    for (stdio_request, http_request) in [
        (
            json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        ),
        (
            json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{"name":"list_graph_stats","arguments":{}}
            }),
            json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{"name":"list_graph_stats","arguments":{}}
            }),
        ),
        (
            json!({"jsonrpc":"2.0","id":3,"method":"resources/list","params":{}}),
            json!({"jsonrpc":"2.0","id":4,"method":"resources/list","params":{}}),
        ),
        (
            json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"resources/read",
                "params":{"uri":"atlas://health/status"}
            }),
            json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"resources/read",
                "params":{"uri":"atlas://health/status"}
            }),
        ),
        (
            json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"logging/setLevel",
                "params":{"level":"warning"}
            }),
            json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"logging/setLevel",
                "params":{"level":"warning"}
            }),
        ),
    ] {
        let stdio = stdio_initialized_call(&repo_root, &db_path, stdio_request);
        let http = harness
            .post_jsonrpc(&http_headers(&session_id), &http_request)
            .expect("http parity request");
        assert_eq!(
            normalized_jsonrpc_body(&stdio),
            normalized_http_jsonrpc_body(&http)
        );
    }
}

#[test]
fn spec_auth_protected_http_routes_share_challenge_and_protocol_behavior() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness = HttpTestHarness::new_with_test_auth(&repo_root, &db_path, &["https://good.test"])
        .expect("http auth harness");

    let metadata = harness
        .get_metadata(&[("Origin", "https://good.test")])
        .expect("metadata route");
    assert_eq!(metadata.status, 200);
    assert_eq!(
        metadata.headers.get("access-control-allow-origin"),
        Some(&"https://good.test".to_owned())
    );

    let post_missing = harness
        .post_jsonrpc(&[], &initialize_request(1))
        .expect("post missing bearer");
    assert_eq!(post_missing.status, 401);
    assert!(
        post_missing
            .headers
            .get("www-authenticate")
            .expect("www-authenticate")
            .contains("error=\"invalid_token\"")
    );

    let invalid_auth = [("Authorization", "Bearer not-a-jwt")];
    let post_invalid = harness
        .post_jsonrpc(&invalid_auth, &initialize_request(1))
        .expect("post invalid bearer");
    assert_eq!(post_invalid.status, 401);
    assert!(
        post_invalid
            .headers
            .get("www-authenticate")
            .expect("www-authenticate")
            .contains("invalid bearer token")
    );

    let bearer = format!(
        "Bearer {}",
        harness
            .make_test_bearer_token(&["atlas:mcp", "atlas:read"])
            .expect("test bearer")
    );
    let initialized = harness
        .post_jsonrpc(
            &[("Authorization", bearer.as_str())],
            &initialize_request(1),
        )
        .expect("authorized initialize");
    assert_eq!(initialized.status, 200);
    assert!(!initialized.headers.contains_key("mcp-session-id"));

    let forbidden_origin = [
        ("Authorization", bearer.as_str()),
        ("Origin", "https://evil.test"),
    ];
    for response in [
        harness
            .get_metadata(&[("Origin", "https://evil.test")])
            .expect("metadata forbidden origin"),
        harness
            .post_jsonrpc(&forbidden_origin, &initialize_request(1))
            .expect("post forbidden origin"),
    ] {
        assert_eq!(response.status, 403);
        assert!(!response.headers.contains_key("www-authenticate"));
    }
}
