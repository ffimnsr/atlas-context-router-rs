use super::*;

#[test]
fn rmcp_stdio_verbose_trace_emits_request_lifecycle_diagnostics() {
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
            "id": 11,
            "method": "$/setTrace",
            "params": { "value": "verbose" }
        }))
        .unwrap();
    let trace_response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("trace response");
    assert_eq!(trace_response["id"], serde_json::json!(11));
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "__test_sleep",
                "arguments": {
                    "sleep_ms": 40,
                    "chunk_ms": 20
                },
                "_meta": request_meta_params()["_meta"].clone()
            }
        }))
        .unwrap();

    let mut notifications = Vec::new();
    let response = loop {
        let message = session
            .recv_json(Duration::from_secs(2))
            .unwrap()
            .expect("trace notification or result");
        if message["method"] == serde_json::json!("notifications/message") {
            notifications.push(message);
            continue;
        }
        if message["id"] == serde_json::json!(2) {
            break message;
        }
    };

    assert_eq!(response["id"], serde_json::json!(2));
    assert!(
        notifications.iter().any(|message| {
            message["params"]["data"]
                .as_str()
                .unwrap_or_default()
                .contains("started request_id=")
                && message["params"]["data"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("tool=__test_sleep")
        }),
        "verbose trace must emit started lifecycle diagnostic"
    );
    assert!(
        notifications.iter().any(|message| {
            message["params"]["data"]
                .as_str()
                .unwrap_or_default()
                .contains("completed request_id=")
                && message["params"]["data"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("tool=__test_sleep")
        }),
        "verbose trace must emit completed lifecycle diagnostic"
    );
    assert!(
        notifications
            .iter()
            .all(|message| message["params"]["logger"] == serde_json::json!("atlas-mcp")),
        "trace diagnostics must use official logging payloads"
    );
    let _ = session.finish().unwrap();
}

#[test]
fn rmcp_stdio_off_trace_emits_no_request_lifecycle_diagnostics() {
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
            "id": 11,
            "method": "$/setTrace",
            "params": { "value": "off" }
        }))
        .unwrap();
    let trace_response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("trace response");
    assert_eq!(trace_response["id"], serde_json::json!(11));
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute" },
                "_meta": request_meta_params()["_meta"].clone()
            }
        }))
        .unwrap();

    let response = loop {
        let message = session
            .recv_json(Duration::from_secs(2))
            .unwrap()
            .expect("result without trace diagnostics");
        assert_ne!(
            message["method"],
            serde_json::json!("notifications/message"),
            "off trace must not emit lifecycle diagnostics"
        );
        if message["id"] == serde_json::json!(2) {
            break message;
        }
    };
    assert_eq!(response["id"], serde_json::json!(2));
    assert!(
        session
            .recv_json(Duration::from_millis(150))
            .unwrap()
            .is_none(),
        "off trace must stay silent after response"
    );
    let _ = session.finish().unwrap();
}

#[test]
fn stdio_transport_emits_progress_without_mcp_log_notifications() {
    let fixture = setup_fixture();
    let input = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "query_graph",
            "progressToken": "tok-1",
            "arguments": {"text": "compute"},
            "_meta": {
                crate::spec::META_PROTOCOL_VERSION: crate::MCP_PROTOCOL_VERSION,
                crate::spec::META_CLIENT_CAPABILITIES: {},
                crate::spec::META_CLIENT_INFO: {"name": "zed", "version": "1.0.0"},
                crate::spec::META_LOG_LEVEL: "warning"
            }
        }
    })
    .to_string()
        + "\n";
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        &input,
        ServerOptions::default(),
    );
    assert!(
        responses
            .iter()
            .all(|value| value["method"] != serde_json::json!("notifications/message")),
        "removed MCP logging channel must not write notifications/message on stdout"
    );
    assert!(
        responses
            .iter()
            .all(|value| value["method"] != serde_json::json!("notifications/progress")),
        "fast tool without installed progress checkpoints must not emit progress notifications"
    );
    assert!(
        responses
            .iter()
            .any(|value| value["id"] == serde_json::json!(2)),
        "tool response must still be returned"
    );
}
