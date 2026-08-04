use super::*;

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn missing_request_meta_is_allowed_after_rmcp_session_initialize() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();

    let response = stdio_single_response_2026(
        &repo_root,
        &fixture.db_path,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    );

    assert!(response["result"]["tools"].is_array());
}

#[test]
fn tools_list_works_as_first_stdio_request_with_valid_request_meta() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();

    let response = stdio_single_response_2026(
        &repo_root,
        &fixture.db_path,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": request_meta_params()
        }),
    );

    assert!(response["result"]["tools"].is_array());
    assert_eq!(
        response["result"]["resultType"],
        serde_json::json!("complete")
    );
}

#[test]
fn previous_request_protocol_version_is_supported() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();

    let response = stdio_single_response_2026(
        &repo_root,
        &fixture.db_path,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {
                "_meta": {
                    crate::spec::META_PROTOCOL_VERSION: crate::spec::MCP_PREVIOUS_PROTOCOL_VERSION,
                    crate::spec::META_CLIENT_CAPABILITIES: {}
                }
            }
        }),
    );

    assert!(response["result"]["tools"].is_array());
}

#[test]
fn server_discover_works_without_initialize_and_matches_initialize_capabilities() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();

    let response = stdio_single_response_2026(
        &repo_root,
        &fixture.db_path,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": request_meta_params()
        }),
    );

    let result = &response["result"];
    assert_eq!(
        result["supportedVersions"],
        crate::spec::supported_protocol_versions_value()
    );
    assert_eq!(
        result["capabilities"],
        serde_json::to_value(
            crate::rmcp_server::AtlasRmcpServer::new(
                &repo_root,
                &fixture.db_path,
                ServerOptions::default(),
            )
            .info()
            .capabilities,
        )
        .expect("capabilities")
    );
    assert_eq!(
        result["_meta"][crate::spec::META_SERVER_INFO],
        crate::spec::server_info_meta_value()
    );
    assert_eq!(
        result["instructions"],
        serde_json::json!(crate::spec::DISCOVER_INSTRUCTIONS)
    );
    assert_eq!(
        result["ttlMs"],
        serde_json::json!(crate::spec::DISCOVER_CACHE_TTL_MS)
    );
    assert_eq!(
        result["cacheScope"],
        serde_json::json!(crate::spec::DISCOVER_CACHE_SCOPE)
    );
    assert_eq!(result["resultType"], serde_json::json!("complete"));
    assert_eq!(
        result["_meta"][crate::spec::META_SERVER_INFO],
        crate::spec::server_info_meta_value()
    );
}

#[test]
fn request_success_results_include_complete_type_and_server_info_meta() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();

    for request in [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": request_meta_params()
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "resources/read",
            "params": {
                "uri": "atlas://health/status",
                "_meta": request_meta_params()["_meta"].clone()
            }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "status",
                "arguments": {},
                "_meta": request_meta_params()["_meta"].clone()
            }
        }),
    ] {
        let response = stdio_single_response_2026(&repo_root, &fixture.db_path, request.clone());
        assert!(response["result"].is_object());
    }
}

#[test]
fn advertised_capabilities_have_stdio_method_handlers_and_descriptor_backing() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let capabilities = crate::spec::initialize_capabilities();

    assert!(capabilities.tools == crate::spec::EmptyCapability::default());
    assert!(capabilities.completions == crate::spec::EmptyCapability::default());
    assert!(capabilities.extensions.is_some());
    assert!(capabilities.experimental.is_some());

    for request in [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"server/discover"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"status","arguments":{}}
        }),
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"resources/list","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":5,"method":"resources/templates/list","params":{}}),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":6,
            "method":"resources/read",
            "params":{"uri":"atlas://health/status"}
        }),
        serde_json::json!({"jsonrpc":"2.0","id":9,"method":"prompts/list","params":{}}),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":10,
            "method":"prompts/get",
            "params":{"name":"inspect_symbol","arguments":{"symbol":"compute"}}
        }),
        serde_json::json!({"jsonrpc":"2.0","id":12,"method":"tasks/get","params":{"taskId":"missing"}}),
    ] {
        let response = stdio_single_response(&repo_root, &fixture.db_path, request.clone());
        assert!(
            response.get("result").is_some() || response.get("error").is_some(),
            "method {} must produce result or typed error",
            request["method"].as_str().expect("method")
        );
        assert_ne!(
            response
                .get("error")
                .and_then(|value| value.get("code"))
                .and_then(serde_json::Value::as_i64),
            Some(METHOD_NOT_FOUND_CODE),
            "method {} must be handled",
            request["method"].as_str().expect("method")
        );
    }

    for removed_method in ["tasks/list", "tasks/result", "tasks/cancel"] {
        let removed = stdio_single_response_2026(
            &repo_root,
            &fixture.db_path,
            serde_json::json!({
                "jsonrpc":"2.0",
                "id":15,
                "method": removed_method,
                "params": {
                    "taskId":"missing",
                    "_meta": {
                        crate::spec::META_PROTOCOL_VERSION: MCP_PROTOCOL_VERSION,
                        crate::spec::META_CLIENT_CAPABILITIES: {},
                        crate::spec::META_CLIENT_INFO: {"name": "zed", "version": "1.0.0"}
                    }
                }
            }),
        );
        let code = removed["error"]["code"].as_i64();
        assert!(
            code == Some(METHOD_NOT_FOUND_CODE) || code == Some(-32021),
            "removed method {removed_method} must return method-not-found or missing-required-client-capability, got {code:?}"
        );
    }

    let removed_logging = stdio_single_response_2026(
        &repo_root,
        &fixture.db_path,
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":8,
            "method":"logging/setLevel",
            "params":{
                "level":"warning",
                "_meta": {
                    crate::spec::META_PROTOCOL_VERSION: MCP_PROTOCOL_VERSION,
                    crate::spec::META_CLIENT_CAPABILITIES: {},
                    crate::spec::META_CLIENT_INFO: {"name": "zed", "version": "1.0.0"}
                }
            }
        }),
    );
    assert!(removed_logging.get("result").is_some() || removed_logging.get("error").is_some());

    let tool_list = crate::tools::tool_list();
    let tools = tool_list["tools"].as_array().expect("tool descriptors");
    assert!(
        !tools.is_empty(),
        "tools/list descriptors must not be empty"
    );

    let prompts = crate::prompts::prompt_descriptors();
    assert!(
        !prompts.is_empty(),
        "prompts/list descriptors must not be empty"
    );
    for (name, args) in [
        ("review_change", serde_json::json!({"files":"src/lib.rs"})),
        ("inspect_symbol", serde_json::json!({"symbol":"compute"})),
        ("plan_refactor", serde_json::json!({"target":"compute"})),
        ("resume_prior_session", serde_json::json!({})),
    ] {
        crate::prompts::prompt_get(name, Some(&args))
            .unwrap_or_else(|error| panic!("prompt {name} must resolve from descriptor: {error}"));
    }

    let resources = crate::resources::resources_list(None).expect("resources/list")["resources"]
        .as_array()
        .expect("resources array")
        .clone();
    assert!(
        !resources.is_empty(),
        "resources/list descriptors must not be empty"
    );
    for resource in resources {
        let uri = resource["uri"].as_str().expect("resource uri");
        crate::resources::resources_read(
            Some(&serde_json::json!({"uri": uri})),
            &repo_root,
            &fixture.db_path,
        )
        .unwrap_or_else(|error| panic!("resource {uri} must read from descriptor: {error}"));
    }

    let template_list =
        crate::resources::resources_templates_list(None).expect("resources/templates/list");
    let templates = template_list["resourceTemplates"]
        .as_array()
        .expect("resource templates array");
    assert!(
        !templates.is_empty(),
        "resources/templates/list descriptors must not be empty"
    );
}
