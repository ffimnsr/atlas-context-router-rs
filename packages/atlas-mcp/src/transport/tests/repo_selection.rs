use super::*;

#[test]
fn fixed_repo_mode_never_emits_roots_list_or_invalidates_on_roots_notifications() {
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
                "capabilities": { "roots": { "listChanged": true } },
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
                "name": "query_graph",
                "arguments": { "text": "compute" },
                "_meta": request_meta_params()["_meta"].clone()
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("fixed-mode query response");
    assert_eq!(response["id"], serde_json::json!(2));
    assert_eq!(
        response["result"]["_meta"]["atlas:repoSelection"]["selectionSource"],
        serde_json::json!("cached_active_root")
    );
    assert!(
        session
            .recv_json(Duration::from_millis(150))
            .unwrap()
            .is_none(),
        "fixed mode must not emit roots/list"
    );

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/roots/list_changed",
            "params": {}
        }))
        .unwrap();
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute" },
                "_meta": request_meta_params()["_meta"].clone()
            }
        }))
        .unwrap();
    let second = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("post-roots-changed response");
    assert_eq!(second["id"], serde_json::json!(3));
    assert_eq!(
        second["result"]["_meta"]["atlas:repoSelection"]["selectionSource"],
        serde_json::json!("cached_active_root")
    );
    assert!(
        session
            .recv_json(Duration::from_millis(150))
            .unwrap()
            .is_none(),
        "roots/list_changed must not trigger reverse requests"
    );
    let _ = session.finish().unwrap();
}

#[test]
fn explicit_repo_root_selector_switches_repo_for_tool_call() {
    let repo_a = setup_graph_repo_fixture("src/alpha.rs", "compute", "src/alpha.rs::fn::compute");
    let repo_b = setup_graph_repo_fixture("src/beta.rs", "compute", "src/beta.rs::fn::compute");
    let repo_b_root = repo_b._dir.path().to_string_lossy().into_owned();
    let session = InteractiveStdioTestSession::start(
        repo_a._dir.path().to_string_lossy().as_ref(),
        &repo_a.db_path,
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
                "name": "query_graph",
                "arguments": {
                    "repo_root": repo_b_root,
                    "text": "compute",
                    "output_format": "json"
                },
                "_meta": request_meta_params()["_meta"].clone()
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("explicit repo_root query response");
    let query_value: serde_json::Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("query_graph json payload"),
    )
    .expect("parse query_graph payload");
    assert_eq!(
        query_value["matches"][0]["file"],
        serde_json::json!("src/beta.rs")
    );
    assert_eq!(
        response["result"]["_meta"]["atlas:repoSelection"]["selectionSource"],
        serde_json::json!("explicit_request")
    );
    assert_eq!(
        response["result"]["_meta"]["atlas:repoRoot"],
        serde_json::json!(repo_b._dir.path().canonicalize().unwrap().to_string_lossy())
    );

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "query_graph",
                "arguments": {
                    "text": "compute",
                    "output_format": "json"
                },
                "_meta": request_meta_params()["_meta"].clone()
            }
        }))
        .unwrap();

    let cached = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("cached explicit-root query response");
    let cached_value: serde_json::Value = serde_json::from_str(
        cached["result"]["content"][0]["text"]
            .as_str()
            .expect("query_graph cached json payload"),
    )
    .expect("parse cached query_graph payload");
    assert_eq!(
        cached_value["matches"][0]["file"],
        serde_json::json!("src/beta.rs")
    );
    assert_eq!(
        cached["result"]["_meta"]["atlas:repoSelection"]["selectionSource"],
        serde_json::json!("cached_active_root")
    );
    let _ = session.finish().unwrap();
}

#[test]
fn invalid_explicit_repo_selector_returns_actionable_error() {
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
                "name": "query_graph",
                "arguments": {
                    "repo_id": "demo",
                    "text": "compute"
                },
                "_meta": request_meta_params()["_meta"].clone()
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("invalid repo selector response");
    assert_eq!(response["id"], serde_json::json!(2));
    assert!(
        !response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    assert_eq!(
        response["error"]["data"]["atlas_repo_selection"]["failure_kind"],
        serde_json::json!("invalid_explicit_repo_selector")
    );
    assert!(
        response["error"]["data"]["atlas_repo_selection"]["recommended_fix"]
            .as_str()
            .unwrap_or_default()
            .contains("arguments.repo_root")
    );
    let _ = session.finish().unwrap();
}
