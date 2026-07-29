#![cfg(feature = "http-transport")]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use atlas_mcp::spec;
use atlas_mcp::testing::{
    HttpTestHarness, InteractiveStdioTestSession, run_stdio_jsonrpc_session_for_tests,
};
use atlas_mcp::{MCP_PROTOCOL_VERSION, ServerOptions};
use serde_json::{Value, json};
use tempfile::TempDir;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/spec_2026_07_28/fixtures")
}

fn assert_fixture(name: &str, actual: Value) {
    let path = fixtures_dir().join(name);
    let expected: Value = serde_json::from_str(&fs::read_to_string(&path).expect("read fixture"))
        .expect("parse fixture json");
    assert_eq!(actual, expected, "fixture mismatch: {}", path.display());
}

fn setup_repo() -> (TempDir, String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("README.md"),
        "# fixture\n\nUsed for MCP 2026 integration tests.\n",
    )
    .expect("write readme");
    fs::create_dir_all(dir.path().join("src")).expect("create src dir");
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn greet_twice(name: &str) -> String { format!(\"hi {name} hi {name}\") }\n",
    )
    .expect("write lib.rs");
    let repo_root = dir.path().to_string_lossy().into_owned();
    let db_path = dir
        .path()
        .join(".atlas")
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();
    (dir, repo_root, db_path)
}

fn request_meta() -> Value {
    json!({
        spec::META_PROTOCOL_VERSION: MCP_PROTOCOL_VERSION,
        spec::META_CLIENT_CAPABILITIES: {
            "elicitation": { "form": {}, "url": {} }
        },
        spec::META_CLIENT_INFO: { "name": "zed", "version": "1.0.0" }
    })
}

fn as_lines(messages: &[Value]) -> String {
    let mut out = String::new();
    for value in messages {
        out.push_str(&serde_json::to_string(value).expect("serialize jsonrpc line"));
        out.push('\n');
    }
    out
}

fn stdio_response(repo_root: &str, db_path: &str, request: Value) -> Value {
    run_stdio_jsonrpc_session_for_tests(
        &as_lines(std::slice::from_ref(&request)),
        repo_root,
        db_path,
        ServerOptions::default(),
    )
    .expect("run stdio session")
    .into_iter()
    .find(|value| value["id"] == request["id"])
    .expect("response by id")
}

fn discover_stdio_snapshot(repo_root: &str, db_path: &str) -> Value {
    let response = stdio_response(
        repo_root,
        db_path,
        json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover"}),
    );
    json!({
        "protocol": response["result"]["supportedVersions"],
        "resultType": response["result"]["resultType"],
        "ttlMs": response["result"]["ttlMs"],
        "cacheScope": response["result"]["cacheScope"],
        "hasExtensionsTasks": response["result"]["capabilities"]["extensions"]["tasks"].is_object(),
        "serverName": response["result"]["serverInfo"]["name"],
    })
}

fn tools_list_first_stdio_snapshot(repo_root: &str, db_path: &str) -> Value {
    let response = stdio_response(
        repo_root,
        db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": { "_meta": request_meta() }
        }),
    );
    let tools = response["result"]["tools"].as_array().expect("tools array");
    json!({
        "resultType": response["result"]["resultType"],
        "ttlMs": response["result"]["ttlMs"],
        "cacheScope": response["result"]["cacheScope"],
        "hasGetContext": tools.iter().any(|tool| tool["name"] == json!("get_context")),
        "hasTasksMethods": tools.iter().any(|tool| {
            matches!(
                tool["name"].as_str(),
                Some("tasks/get") | Some("tasks/update") | Some("tasks/result") | Some("tasks/list") | Some("tasks/cancel")
            )
        }),
    })
}

fn mrtr_retry_stdio_snapshot(repo_root: &str, db_path: &str) -> Value {
    let session = InteractiveStdioTestSession::start(repo_root, db_path, ServerOptions::default())
        .expect("start interactive stdio session");
    session
        .send_json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "elicitation": { "form": {}, "url": {} }
                },
                "clientInfo": { "name": "zed", "version": "1.0.0" },
                "_meta": { "clientTag": "spec-2026" }
            }
        }))
        .expect("send stdio initialize");
    let _ = session
        .recv_json(Duration::from_secs(2))
        .expect("recv stdio initialize")
        .expect("stdio initialize response");
    session
        .send_json(&json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }))
        .expect("send stdio initialized notification");
    session
        .send_json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "purge_saved_context",
                "arguments": { "keep_days": 30, "output_format": "json" },
                "_meta": request_meta()
            }
        }))
        .expect("send first purge request");
    let input_required = session
        .recv_json(Duration::from_secs(2))
        .expect("recv input_required")
        .expect("input_required response");
    let request_state = input_required["result"]["requestState"]
        .as_str()
        .expect("requestState")
        .to_owned();
    session
        .send_json(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "purge_saved_context",
                "arguments": { "keep_days": 30, "output_format": "json" },
                "requestState": request_state,
                "inputResponses": {
                    "confirmation": {
                        "action": "accept",
                        "content": { "confirmation": "confirm" }
                    }
                },
                "_meta": request_meta()
            }
        }))
        .expect("send retry purge request");
    let final_response = loop {
        let message = session
            .recv_json(Duration::from_secs(2))
            .expect("recv final response")
            .expect("final response message");
        if message["id"] == json!(3) {
            break message;
        }
    };
    assert!(
        session
            .recv_json(Duration::from_millis(100))
            .expect("recv optional trailing output")
            .is_none(),
        "stdio MRTR path must not emit server-initiated JSON-RPC requests"
    );
    let _ = session.finish().expect("finish interactive stdio session");
    json!({
        "resultType": input_required["result"]["resultType"],
        "requestType": input_required["result"]["inputRequests"][0]["type"],
        "requestId": input_required["result"]["inputRequests"][0]["id"],
        "requestStatePresent": input_required["result"]["requestState"].is_string(),
        "finalResultType": final_response["result"]["resultType"],
        "finalErrorCode": final_response["error"]["code"],
    })
}

fn http_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Accept", "application/json"),
        ("MCP-Protocol-Version", MCP_PROTOCOL_VERSION),
    ]
}

fn discover_http_snapshot(harness: &HttpTestHarness) -> Value {
    let response = harness
        .post_jsonrpc(
            &http_headers(),
            &json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover"}),
        )
        .expect("http discover");
    json!({
        "status": response.status,
        "protocolHeader": response.headers.get("mcp-protocol-version"),
        "hasSessionHeader": response.headers.contains_key("mcp-session-id"),
        "resultType": response.json_body.as_ref().expect("json body")["result"]["resultType"],
        "ttlMs": response.json_body.as_ref().expect("json body")["result"]["ttlMs"],
        "cacheScope": response.json_body.as_ref().expect("json body")["result"]["cacheScope"],
        "serverName": response.json_body.as_ref().expect("json body")["result"]["serverInfo"]["name"],
    })
}

fn tools_list_cacheable_http_snapshot(harness: &HttpTestHarness) -> Value {
    let response = harness
        .post_jsonrpc(
            &http_headers(),
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": { "_meta": request_meta() }
            }),
        )
        .expect("http tools/list");
    let body = response.json_body.as_ref().expect("json body");
    let tools = body["result"]["tools"].as_array().expect("tools array");
    json!({
        "status": response.status,
        "hasSessionHeader": response.headers.contains_key("mcp-session-id"),
        "resultType": body["result"]["resultType"],
        "ttlMs": body["result"]["ttlMs"],
        "cacheScope": body["result"]["cacheScope"],
        "hasGetContext": tools.iter().any(|tool| tool["name"] == json!("get_context")),
    })
}

fn header_validation_http_snapshot(harness: &HttpTestHarness) -> Value {
    let response = harness
        .post_jsonrpc(
            &[
                ("Accept", "application/json"),
                ("MCP-Protocol-Version", MCP_PROTOCOL_VERSION),
                ("Mcp-Method", "tools/list"),
            ],
            &json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list", "params": {}}),
        )
        .expect("http header mismatch");
    let body = response.json_body.as_ref().expect("json body");
    json!({
        "status": response.status,
        "code": body["error"]["code"],
        "atlas_error_code": body["error"]["data"]["atlas_error_code"],
        "message": body["error"]["message"],
    })
}

fn mrtr_input_required_http_snapshot(harness: &HttpTestHarness) -> Value {
    let response = harness
        .post_jsonrpc(
            &http_headers(),
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "purge_saved_context",
                    "arguments": { "keep_days": 30, "output_format": "json" },
                    "_meta": request_meta()
                }
            }),
        )
        .expect("http input_required");
    let body = response.json_body.as_ref().expect("json body");
    json!({
        "status": response.status,
        "resultType": body["result"]["resultType"],
        "requestType": body["result"]["inputRequests"][0]["type"],
        "requestId": body["result"]["inputRequests"][0]["id"],
        "requestStatePresent": body["result"]["requestState"].is_string(),
    })
}

fn subscriptions_listen_http_snapshot(harness: &HttpTestHarness) -> Value {
    let response = harness
        .post_jsonrpc(
            &[
                ("Accept", "text/event-stream"),
                ("MCP-Protocol-Version", MCP_PROTOCOL_VERSION),
                ("Mcp-Method", "subscriptions/listen"),
            ],
            &json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "subscriptions/listen",
                "params": { "notificationTypes": ["tools", "resource_subscriptions"] }
            }),
        )
        .expect("http subscriptions/listen");
    json!({
        "status": response.status,
        "contentType": response.headers.get("content-type"),
        "hasSessionHeader": response.headers.contains_key("mcp-session-id"),
        "hasMessageEvent": response.body_text.contains("event: message"),
        "hasSubscriptionId": response.body_text.contains("\"subscriptionId\":\"sub-7\""),
        "hasToolsListChanged": response.body_text.contains("notifications/tools/list_changed"),
        "hasResourceUpdated": response.body_text.contains("notifications/resources/updated"),
        "hasPromptsListChanged": response.body_text.contains("notifications/prompts/list_changed"),
        "hasProgress": response.body_text.contains("$/progress"),
    })
}

fn no_session_http_snapshot(harness: &HttpTestHarness) -> Value {
    let response = harness
        .post_jsonrpc(
            &[
                ("Accept", "application/json"),
                ("MCP-Protocol-Version", MCP_PROTOCOL_VERSION),
                ("Mcp-Method", "tools/list"),
                ("Mcp-Session-Id", "stale-session"),
                ("Last-Event-ID", "stale-event"),
            ],
            &json!({
                "jsonrpc": "2.0",
                "id": 8,
                "method": "tools/list",
                "params": { "_meta": request_meta() }
            }),
        )
        .expect("http no-session behavior");
    let body = response.json_body.as_ref().expect("json body");
    json!({
        "status": response.status,
        "hasSessionHeader": response.headers.contains_key("mcp-session-id"),
        "resultType": body["result"]["resultType"],
        "hasGetContext": body["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["name"] == json!("get_context")),
    })
}

#[test]
fn spec_2026_stdio_discover_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    assert_fixture(
        "discover.stdio.json",
        discover_stdio_snapshot(&repo_root, &db_path),
    );
}

#[test]
fn spec_2026_stdio_tools_list_first_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    assert_fixture(
        "tools_list_first.stdio.json",
        tools_list_first_stdio_snapshot(&repo_root, &db_path),
    );
}

#[test]
fn spec_2026_stdio_mrtr_retry_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    assert_fixture(
        "mrtr_retry.stdio.json",
        mrtr_retry_stdio_snapshot(&repo_root, &db_path),
    );
}

#[test]
fn spec_2026_http_discover_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    assert_fixture("discover.http.json", discover_http_snapshot(&harness));
}

#[test]
fn spec_2026_http_tools_list_cacheable_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    assert_fixture(
        "tools_list_cacheable.http.json",
        tools_list_cacheable_http_snapshot(&harness),
    );
}

#[test]
fn spec_2026_http_header_validation_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    assert_fixture(
        "header_validation.http.json",
        header_validation_http_snapshot(&harness),
    );
}

#[test]
fn spec_2026_http_input_required_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    assert_fixture(
        "mrtr_input_required.http.json",
        mrtr_input_required_http_snapshot(&harness),
    );
}

#[test]
fn spec_2026_http_subscriptions_listen_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    assert_fixture(
        "subscriptions_listen.http.json",
        subscriptions_listen_http_snapshot(&harness),
    );
}

#[test]
fn spec_2026_http_no_session_behavior_matches_fixture() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    assert_fixture("no_session.http.json", no_session_http_snapshot(&harness));
}

#[test]
fn drift_no_active_roots_list_dispatch_path_remains() {
    for path in [
        "src/transport/dispatch.rs",
        "src/transport/input.rs",
        "src/transport_http.rs",
    ] {
        let contents = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
            .expect("read source file");
        assert!(
            !contents.contains("\"roots/list\""),
            "active MCP roots/list handler must stay removed: {path}"
        );
    }
}

#[test]
fn drift_docs_stop_describing_roots_or_http_session_protocol() {
    for path in [
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../MCP_TOOLS.md"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"),
    ] {
        let contents = fs::read_to_string(&path).expect("read doc file");
        assert!(
            !contents.contains("roots/list"),
            "docs must not guide clients toward roots/list: {}",
            path.display()
        );
    }
}
