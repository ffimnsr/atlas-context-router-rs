use super::*;

#[test]
fn stdio_transport_cancels_queued_request_without_response() {
    let fixture = setup_fixture();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"__test_sleep\",\"arguments\":{\"sleep_ms\":200}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"__test_sleep\",\"progressToken\":\"cancel-me\",\"arguments\":{\"sleep_ms\":200}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"$/cancelRequest\",\"params\":{\"id\":2}}\n"
    );
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        input,
        ServerOptions {
            // Set worker thread count and timeout aggressively so
            // cancellation can be observed without waiting for real I/O.
            worker_threads: 1,
            tool_timeout_ms: 10_000,
            tool_timeout_ms_by_tool: HashMap::new(),
            #[cfg(feature = "http-transport")]
            http_auth: None,
        },
    );
    // With a single worker thread, the second tool/call (`id:2`) starts
    // after the first completes. The cancel targets `id:2` before it
    // starts, so we should see:
    //   - id=1 (complete)
    //   - id=2 (cancel notification – complete with isError or no response)
    //   - cancel notification for progressToken "cancel-me" (progress end)
    // Since id=2 was cancelled before start, we expect exactly 2 JSON-RPC
    // responses (for ids 1 and 2) plus the progress notification.
    let response_ids: Vec<_> = responses
        .iter()
        .filter_map(|value| value.get("id").and_then(|id| id.as_i64()))
        .collect();
    assert!(response_ids.contains(&1), "first request should complete");
    assert!(
        response_ids.contains(&2)
            || responses.iter().any(|v| {
                v.get("method") == Some(&serde_json::json!("$/progress"))
                    && v.pointer("/params/value/kind") == Some(&serde_json::json!("end"))
            }),
        "second request should be cancelled (response or progress end)"
    );
}

#[test]
fn removed_handrolled_transport_modules_are_absent_from_source_tree() {
    let transport_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/transport");
    for removed in [
        "legacy_2025.rs",
        "socket_dispatch.rs",
        "socket_input.rs",
        "socket_io.rs",
        "socket_jsonrpc.rs",
        "socket_notify.rs",
    ] {
        assert!(
            !transport_dir.join(removed).exists(),
            "removed transport source must stay deleted: {removed}"
        );
    }
}

#[test]
fn removed_jsonrpc_helpers_and_dispatch_module_are_not_exported() {
    let transport_mod = include_str!("../mod.rs");
    assert!(!transport_mod.contains("socket_dispatch"));
    assert!(!transport_mod.contains("socket_input"));
    assert!(!transport_mod.contains("socket_io"));
    assert!(!transport_mod.contains("socket_jsonrpc"));
    assert!(!transport_mod.contains("socket_notify"));

    let socket_source = include_str!("../socket.rs");
    assert!(!socket_source.contains("fn jsonrpc_ok("));
    assert!(!socket_source.contains("fn jsonrpc_error("));
}

#[test]
fn dispatch_panic_returns_internal_error_and_server_survives() {
    let fixture = setup_fixture();
    // Dispatch of an unknown tool returns MethodNotFound (not a panic).
    // The server survives and continues processing subsequent requests.
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"__nonexistent_test_tool\",\"arguments\":{}}}\n".to_owned(),
        // second request to verify server still processes
        "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/list\",\"params\":{}}\n".to_owned(),
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
        .filter_map(|v| v.get("id").cloned().map(|id| (id, v)))
        .collect();

    // Unknown tool must produce MethodNotFound JSON-RPC error
    let response = by_id
        .get(&serde_json::json!(10))
        .expect("unknown tool response");
    assert!(
        response.get("result").is_none(),
        "unknown tool must not produce result"
    );
    assert_eq!(
        response["error"]["code"],
        serde_json::json!(-32601),
        "unknown tool must produce MethodNotFound"
    );
    assert_eq!(
        response["error"]["data"]["atlas_error_code"],
        serde_json::json!("method_not_found")
    );
    // tools/list must still work after the error
    let list_response = by_id
        .get(&serde_json::json!(11))
        .expect("tools/list response");
    assert!(
        list_response.get("result").is_some(),
        "server must survive after dispatch error"
    );
}
