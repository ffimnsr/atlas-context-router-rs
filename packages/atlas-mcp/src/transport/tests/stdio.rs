use super::*;

#[test]
fn rmcp_stdio_emits_official_progress_notifications_for_long_running_tool() {
    let fixture = setup_fixture();
    let session = InteractiveStdioTestSession::start(
        fixture._dir.path().to_string_lossy().as_ref(),
        &fixture.db_path,
        ServerOptions::default(),
    )
    .unwrap();

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "zed", "version": "1.0.0" }
            }
        }))
        .unwrap();
    let _ = session.recv_json(Duration::from_secs(1)).unwrap();
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }))
        .unwrap();
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "__test_sleep",
                "arguments": {
                    "sleep_ms": 120,
                    "chunk_ms": 20,
                    "report_progress": true
                },
                "_meta": {
                    crate::spec::META_PROTOCOL_VERSION: MCP_PROTOCOL_VERSION,
                    crate::spec::META_CLIENT_CAPABILITIES: {},
                    crate::spec::META_CLIENT_INFO: {"name": "zed", "version": "1.0.0"},
                    "progressToken": 77
                }
            }
        }))
        .unwrap();

    let progress = loop {
        let message = session
            .recv_json(Duration::from_secs(2))
            .unwrap()
            .expect("progress or result");
        if message["method"] == serde_json::json!("notifications/progress") {
            break message;
        }
    };
    assert_eq!(progress["params"]["progressToken"], serde_json::json!(77));
    assert!(progress["params"]["progress"].is_number());
    assert!(
        progress["params"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("slept")
    );

    let response = loop {
        let message = session
            .recv_json(Duration::from_secs(2))
            .unwrap()
            .expect("final result");
        if message["id"] == serde_json::json!(2) {
            break message;
        }
    };
    assert_eq!(
        response["result"]["structuredContent"]["slept_ms"],
        serde_json::json!(120)
    );
    let _ = session.finish().unwrap();
}

#[test]
fn rmcp_stdio_cancellation_sets_tool_cancel_flag() {
    let fixture = setup_fixture();
    let session = InteractiveStdioTestSession::start(
        fixture._dir.path().to_string_lossy().as_ref(),
        &fixture.db_path,
        ServerOptions::default(),
    )
    .unwrap();

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "zed", "version": "1.0.0" }
            }
        }))
        .unwrap();
    let _ = session.recv_json(Duration::from_secs(1)).unwrap();
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }))
        .unwrap();
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "__test_sleep",
                "arguments": {
                    "sleep_ms": 300,
                    "chunk_ms": 25,
                    "report_progress": true
                },
                "_meta": {
                    crate::spec::META_PROTOCOL_VERSION: MCP_PROTOCOL_VERSION,
                    crate::spec::META_CLIENT_CAPABILITIES: {},
                    crate::spec::META_CLIENT_INFO: {"name": "zed", "version": "1.0.0"},
                    "progressToken": 88
                }
            }
        }))
        .unwrap();

    loop {
        let message = session
            .recv_json(Duration::from_secs(2))
            .unwrap()
            .expect("progress before cancel");
        if message["method"] == serde_json::json!("notifications/progress") {
            break;
        }
    }

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {
                "requestId": 2,
                "reason": "test cancel"
            }
        }))
        .unwrap();

    let drain_deadline = std::time::Instant::now() + Duration::from_millis(750);
    while let Some(message) = session
        .recv_json(Duration::from_millis(100))
        .unwrap()
        .filter(|_| std::time::Instant::now() < drain_deadline)
    {
        assert_ne!(
            message["id"],
            serde_json::json!(2),
            "official rmcp must not emit final response for cancelled requests"
        );
        assert_eq!(
            message["method"],
            serde_json::json!("notifications/progress"),
            "only in-flight progress notifications may arrive after cancellation"
        );
        if std::time::Instant::now() >= drain_deadline {
            break;
        }
    }

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "ping",
            "params": {}
        }))
        .unwrap();
    let ping = loop {
        let message = session
            .recv_json(Duration::from_secs(2))
            .unwrap()
            .expect("ping result after cancellation");
        if message["id"] == serde_json::json!(3) {
            break message;
        }
    };
    assert_eq!(ping["result"], serde_json::Value::Null);
    let _ = session.finish().unwrap();
}

#[test]
fn stdio_transport_handles_initialize_list_and_tool_calls() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"prompts/list\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"prompts/get\",\"params\":{\"name\":\"inspect_symbol\",\"arguments\":{\"symbol\":\"compute\"}}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"compute\"}}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"target\":{\"kind\":\"query\",\"query\":\"compute\"}}}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    );
    assert_eq!(
        responses.len(),
        6,
        "initialized notification must not emit a response"
    );

    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .map(|response| (response["id"].clone(), response))
        .collect();

    let initialize_result = &by_id[&serde_json::json!(1)]["result"];
    assert_eq!(initialize_result["protocolVersion"], MCP_PROTOCOL_VERSION);
    assert_eq!(
        initialize_result["serverInfo"]["description"],
        serde_json::json!(env!("CARGO_PKG_DESCRIPTION"))
    );
    assert!(initialize_result["capabilities"].is_object());

    let response_tool_names: Vec<_> = by_id[&serde_json::json!(2)]["result"]["tools"]
        .as_array()
        .expect("stdio tools/list array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    let tool_registry = crate::tools::tool_list();
    let registry_tool_names: Vec<_> = tool_registry["tools"]
        .as_array()
        .expect("tool registry array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(response_tool_names, registry_tool_names);
    assert_eq!(
        by_id[&serde_json::json!(2)]["result"]["resultType"],
        serde_json::json!("complete")
    );
    let tools = by_id[&serde_json::json!(2)]["result"]["tools"]
        .as_array()
        .expect("tools/list result tools array");
    assert!(
        tools.iter().any(|tool| tool["name"] == "get_context"),
        "tools/list must expose get_context"
    );

    let prompts = by_id[&serde_json::json!(3)]["result"]["prompts"]
        .as_array()
        .expect("prompts/list result prompts array");
    assert!(
        prompts
            .iter()
            .any(|prompt| prompt["name"] == "inspect_symbol"),
        "prompts/list must expose inspect_symbol"
    );

    let prompt_text = by_id[&serde_json::json!(4)]["result"]["messages"][0]["content"]["text"]
        .as_str()
        .expect("prompt text");
    assert!(prompt_text.contains("compute"));
    assert!(prompt_text.contains("query_graph"));
    assert!(prompt_text.contains("symbol_neighbors"));

    assert!(
        by_id[&serde_json::json!(5)]["result"]["_meta"]
            .get("atlas:outputFormat")
            .is_none(),
        "query_graph transport response must not expose removed output format metadata"
    );
    let query_text = by_id[&serde_json::json!(5)]["result"]["content"][0]["text"]
        .as_str()
        .expect("query_graph text content");
    let query_value: serde_json::Value =
        serde_json::from_str(query_text).expect("query_graph payload json");
    assert_eq!(
        query_value["matches"][0]["qn"],
        "src/service.rs::fn::compute"
    );

    assert!(
        by_id[&serde_json::json!(6)]["result"]["_meta"]
            .get("atlas:outputFormat")
            .is_none(),
        "get_context transport response must not expose removed output format metadata"
    );
    let context_text = by_id[&serde_json::json!(6)]["result"]["content"][0]["text"]
        .as_str()
        .expect("get_context text content");
    let context_value: serde_json::Value =
        serde_json::from_str(context_text).expect("get_context payload json");
    assert_eq!(context_value["intent"], serde_json::json!("symbol"));
    assert!(context_value["nodes"].as_array().is_some_and(|nodes| {
        nodes
            .iter()
            .any(|node| node["qn"] == serde_json::json!("src/service.rs::fn::compute"))
    }));
}

#[test]
fn rmcp_stdio_unknown_method_returns_method_not_found() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"missing/method\",\"params\":{}}\n".to_owned(),
    ]
    .concat();

    let response = run_stdio_jsonrpc_session_for_tests(
        &input,
        &repo_root,
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run rmcp stdio unknown-method script")
    .into_iter()
    .find(|value| value["id"] == serde_json::json!(2))
    .expect("unknown-method response");

    assert_eq!(response["error"]["code"], serde_json::json!(-32601));
    assert!(
        !response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
}

#[test]
fn stdio_transport_rejects_unsupported_initialize_protocol_version() {
    let fixture = setup_fixture();
    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"zed\",\"version\":\"1.0.0\"}}}\n";
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let result = run_rmcp_script_capture(
        &repo_root,
        &fixture.db_path,
        input,
        ServerOptions::default(),
    );
    assert!(
        result
            .output
            .iter()
            .any(|value| value["id"] == serde_json::json!(1))
            || result
                .server_error
                .as_deref()
                .unwrap_or_default()
                .contains("unsupported protocol version")
    );
}

#[test]
fn stdio_transport_reports_unknown_task_with_task_not_found_error() {
    let fixture = setup_fixture();
    let repo_dir = tempfile::tempdir().expect("tempdir");
    let input = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"zed\",\"version\":\"1.0.0\"}}}}}}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tasks/get\",\"params\":{{\"taskId\":\"missing\"}}}}\n",
        MCP_PROTOCOL_VERSION
    );
    let repo_root = repo_dir.path().to_str().expect("repo dir path");
    let responses = run_rmcp_script(
        repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    );
    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .filter_map(|value| value.get("id").cloned().map(|id| (id, value)))
        .collect();
    let code = by_id[&serde_json::json!(2)]["error"]["code"].as_i64();
    assert!(code == Some(-32010) || code == Some(-32021));
}

#[test]
fn stdio_transport_rejects_jsonrpc_batch_requests() {
    let fixture = setup_fixture();
    let input = "[]\n";
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let result = run_rmcp_script_capture(
        &repo_root,
        &fixture.db_path,
        input,
        ServerOptions::default(),
    );
    assert!(
        !result.output.is_empty() || result.server_error.is_some(),
        "rmcp transport must surface batch-request failure"
    );
}

#[test]
fn stdio_transport_reports_malformed_first_frame_through_rmcp_setup_error() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let result = run_rmcp_script_capture(
        &repo_root,
        &fixture.db_path,
        "not-json\n",
        ServerOptions::default(),
    );

    assert!(
        result.output.is_empty() || result.server_error.is_some(),
        "rmcp transport must reject malformed first frame"
    );
}

#[test]
fn stdio_transport_tool_argument_errors_return_is_error_tool_results() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"   \",\"regex\":\"\"}}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    );
    let response = responses
        .into_iter()
        .find(|value| value["id"] == serde_json::json!(2))
        .expect("query_graph response");
    let result = &response["result"];
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(result["content"][0]["type"], serde_json::json!("text"));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert!(
        result["structuredContent"]["retry_guidance"]
            .as_str()
            .is_some_and(|guidance| !guidance.is_empty())
    );
    assert!(
        result.get("Text").is_none(),
        "legacy Text wrapper must not appear"
    );
    assert!(result["_meta"].get("atlas:outputFormat").is_none());
}

#[test]
fn stdio_transport_query_graph_empty_request_returns_self_correcting_contract() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"   \",\"regex\":\"\",\"output_format\":\"json\"}}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    );
    let response = responses
        .into_iter()
        .find(|value| value["id"] == serde_json::json!(2))
        .expect("query_graph response");
    let result = &response["result"];
    let details = &result["structuredContent"]["details"];
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        details["offending_fields"],
        serde_json::json!(["text", "regex"])
    );
    assert_eq!(
        details["retry_example"],
        serde_json::json!({"text": "compute"})
    );
    assert_eq!(
        result["content"][0]["text"],
        serde_json::json!(
            "query_graph needs non-empty 'text', non-empty 'regex', or both Provide one accepted query shape and retry."
        )
    );
}

#[test]
fn stdio_transport_missing_file_returns_is_error_tool_result() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"read_file_around_match\",\"arguments\":{\"file\":\"src/missing.rs\",\"query\":\"needle\",\"output_format\":\"json\"}}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let response = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    )
    .into_iter()
    .find(|value| value["id"] == serde_json::json!(2))
    .expect("read_file_around_match response");
    assert!(
        response.get("error").is_none(),
        "missing file must be reported as tool execution error"
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    assert_eq!(
        response["result"]["structuredContent"]["code"],
        serde_json::json!("file_not_found")
    );
    assert_eq!(
        response["result"]["structuredContent"]["details"]["path"],
        serde_json::json!("src/missing.rs")
    );
    assert_eq!(
        response["result"]["content"][0]["text"],
        serde_json::json!(
            "file not found: src/missing.rs Use exact repo-relative file path inside current Atlas repo, then retry."
        )
    );
}

#[test]
fn stdio_transport_unknown_tool_still_returns_jsonrpc_error() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"unknown_tool_xyz\",\"arguments\":{}}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let response = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    )
    .into_iter()
    .find(|value| value["id"] == serde_json::json!(2))
    .expect("unknown tool response");
    assert!(
        response.get("result").is_none(),
        "unknown tool must not be normalized into result.isError"
    );
    assert_eq!(response["error"]["code"], serde_json::json!(-32601));
}

#[test]
fn stdio_transport_invalid_regex_tool_input_returns_is_error_tool_result() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"compute\",\"regex\":\"(\"}}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let response = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    )
    .into_iter()
    .find(|value| value["id"] == serde_json::json!(2))
    .expect("query_graph response");
    assert!(
        response.get("error").is_none(),
        "tool validation must not be protocol error"
    );
    assert_eq!(response["result"]["isError"], serde_json::json!(true));
    assert_eq!(
        response["result"]["content"][0]["type"],
        serde_json::json!("text")
    );
    assert_eq!(
        response["result"]["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert!(
        response["result"]["structuredContent"]["message"]
            .as_str()
            .expect("message")
            .contains("invalid regex pattern")
    );
    assert!(
        response["result"].get("Text").is_none(),
        "legacy Text wrapper must not appear"
    );
}

#[test]
fn stdio_transport_tools_call_request_shape_errors_use_invalid_params() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"arguments\":{}}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":\"bad\"}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    );
    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .filter(|value| value["id"].is_number())
        .map(|response| (response["id"].clone(), response))
        .collect();
    let code_2 = by_id[&serde_json::json!(2)]["error"]["code"].as_i64();
    let code_3 = by_id[&serde_json::json!(3)]["error"]["code"].as_i64();
    assert!(matches!(code_2, Some(-32601) | Some(-32602)));
    assert!(matches!(code_3, Some(-32601) | Some(-32602)));
}

#[test]
fn stdio_transport_redacts_internal_sql_errors_from_tool_failures() {
    let fixture = setup_fixture();
    let conn = Connection::open(&fixture.db_path).expect("open fixture db");
    conn.execute_batch("DROP TABLE nodes;")
        .expect("drop nodes table to force internal db error");
    drop(conn);

    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"compute\"}}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    );
    let response = responses
        .into_iter()
        .find(|value| value["id"] == serde_json::json!(2))
        .expect("query_graph response");

    let result = &response["result"];
    assert_eq!(
        result["isError"].as_bool(),
        Some(true),
        "corrupt db must produce isError=true tool result; response={response}"
    );
    let reason = result["structuredContent"]["message"]
        .as_str()
        .or_else(|| result["atlas_readiness"]["reason"].as_str())
        .unwrap_or_default();
    assert!(
        !reason.to_ascii_lowercase().contains("sqlite"),
        "reason must not leak sqlite internals: {reason}"
    );
    assert!(
        !reason.to_ascii_lowercase().contains("sql"),
        "reason must not leak sql internals: {reason}"
    );
    assert!(
        !reason.contains("no such table"),
        "reason must not leak raw schema failure: {reason}"
    );
}

#[test]
fn stdio_transport_exposes_resources_completion_methods_and_rejects_removed_logging_method() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"resources/list\",\"params\":{\"limit\":1}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"resources/templates/list\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"completion/complete\",\"params\":{\"ref\":{\"name\":\"tools/call\"},\"argument\":{\"name\":\"output_format\",\"value\":\"j\"}}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"logging/setLevel\",\"params\":{\"level\":\"warning\"}}\n".to_owned(),
    ]
    .concat();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    );
    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .filter(|value| value.get("id").is_some())
        .map(|response| (response["id"].clone(), response))
        .collect();
    assert_eq!(
        by_id[&serde_json::json!(2)]["result"]["resources"][0]["uri"],
        serde_json::json!("atlas://docs/index")
    );
    assert!(
        by_id[&serde_json::json!(3)]["result"]["resourceTemplates"]
            .as_array()
            .expect("templates")
            .len()
            >= 4
    );
    assert!(
        by_id[&serde_json::json!(4)].get("result").is_some()
            || by_id[&serde_json::json!(4)].get("error").is_some()
    );
    assert!(
        by_id[&serde_json::json!(5)].get("result").is_some()
            || by_id[&serde_json::json!(5)].get("error").is_some()
    );
}
