#![cfg(feature = "http-transport")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
            "elicitation": { "form": {}, "url": {} },
            "extensions": {
                "io.modelcontextprotocol/tasks": {}
            }
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
    let response = run_stdio_jsonrpc_session_for_tests(
        &as_lines(&[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "zed", "version": "1.0.0" }
                }
            }),
            json!({"jsonrpc": "2.0", "id": 2, "method": "server/discover"}),
        ]),
        repo_root,
        db_path,
        ServerOptions::default(),
    )
    .expect("run stdio session")
    .into_iter()
    .find(|value| value["id"] == json!(2))
    .expect("discover response by id");
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

fn input_request_snapshot(input_requests: &Value) -> Value {
    match input_requests {
        Value::Array(items) => items.first().cloned().expect("inputRequests array"),
        Value::Object(object) => object
            .iter()
            .next()
            .map(|(id, request)| {
                let mut snapshot = json!({ "id": id });
                if let Some(request_object) = request.as_object()
                    && let Some(params) = request_object.get("params").and_then(Value::as_object)
                {
                    snapshot["type"] = params.get("mode").cloned().unwrap_or(Value::Null);
                }
                snapshot
            })
            .expect("inputRequests object"),
        other => panic!("unexpected inputRequests shape: {other}"),
    }
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
                "arguments": { "keep_days": 30 },
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
                "arguments": { "keep_days": 30 },
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
    let request = input_request_snapshot(&input_required["result"]["inputRequests"]);
    json!({
        "resultType": input_required["result"]["resultType"],
        "requestType": request["type"],
        "requestId": request["id"],
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
    let _ = harness.post_jsonrpc(
        &[],
        &json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "zed", "version": "1.0.0" }
            }
        }),
    );
    let response = harness
        .post_jsonrpc(
            &http_headers(),
            &json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover"}),
        )
        .expect("http discover");
    let body = response.json_body.as_ref();
    json!({
        "status": response.status,
        "protocolHeader": response.headers.get("mcp-protocol-version"),
        "hasSessionHeader": response.headers.contains_key("mcp-session-id"),
        "hasJsonBody": body.is_some(),
        "resultType": body.map(|body| body["result"]["resultType"].clone()).unwrap_or(Value::Null),
        "ttlMs": body.map(|body| body["result"]["ttlMs"].clone()).unwrap_or(Value::Null),
        "cacheScope": body.map(|body| body["result"]["cacheScope"].clone()).unwrap_or(Value::Null),
        "serverName": body.map(|body| body["result"]["serverInfo"]["name"].clone()).unwrap_or(Value::Null),
    })
}

fn tools_list_cacheable_http_snapshot(harness: &HttpTestHarness) -> Value {
    let _ = harness.post_jsonrpc(
        &[],
        &json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "zed", "version": "1.0.0" }
            }
        }),
    );
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
    let body = response.json_body.as_ref();
    let has_get_context = body
        .and_then(|body| body["result"]["tools"].as_array())
        .is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool["name"] == json!("get_context"))
        });
    json!({
        "status": response.status,
        "hasSessionHeader": response.headers.contains_key("mcp-session-id"),
        "hasJsonBody": body.is_some(),
        "resultType": body.map(|body| body["result"]["resultType"].clone()).unwrap_or(Value::Null),
        "ttlMs": body.map(|body| body["result"]["ttlMs"].clone()).unwrap_or(Value::Null),
        "cacheScope": body.map(|body| body["result"]["cacheScope"].clone()).unwrap_or(Value::Null),
        "hasGetContext": has_get_context,
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
    let body = response.json_body.as_ref();
    json!({
        "status": response.status,
        "hasJsonBody": body.is_some(),
        "code": body.map(|body| body["error"]["code"].clone()).unwrap_or(Value::Null),
        "atlas_error_code": body.map(|body| body["error"]["data"]["atlas_error_code"].clone()).unwrap_or(Value::Null),
        "message": body.map(|body| body["error"]["message"].clone()).unwrap_or(Value::Null),
    })
}

fn mrtr_input_required_http_snapshot(harness: &HttpTestHarness) -> Value {
    let _ = harness.post_jsonrpc(
        &[],
        &json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "zed", "version": "1.0.0" }
            }
        }),
    );
    let response = harness
        .post_jsonrpc(
            &http_headers(),
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "purge_saved_context",
                    "arguments": { "keep_days": 30 },
                    "_meta": request_meta()
                }
            }),
        )
        .expect("http input_required");
    let body = response.json_body.as_ref();
    let request = body
        .and_then(|body| body["result"].get("inputRequests"))
        .map(input_request_snapshot)
        .unwrap_or(Value::Null);
    json!({
        "status": response.status,
        "hasJsonBody": body.is_some(),
        "resultType": body.map(|body| body["result"]["resultType"].clone()).unwrap_or(Value::Null),
        "requestType": request.get("type").cloned().unwrap_or(Value::Null),
        "requestId": request.get("id").cloned().unwrap_or(Value::Null),
        "requestStatePresent": body.is_some_and(|body| body["result"]["requestState"].is_string()),
    })
}

fn subscriptions_listen_http_snapshot(harness: &HttpTestHarness) -> Value {
    let response = harness
        .post_jsonrpc(
            &[
                ("Accept", "application/json, text/event-stream"),
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
    let _ = harness.post_jsonrpc(
        &[],
        &json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "zed", "version": "1.0.0" }
            }
        }),
    );
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
    let body = response.json_body.as_ref();
    let has_get_context = body
        .and_then(|body| body["result"]["tools"].as_array())
        .is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool["name"] == json!("get_context"))
        });
    json!({
        "status": response.status,
        "hasSessionHeader": response.headers.contains_key("mcp-session-id"),
        "hasJsonBody": body.is_some(),
        "resultType": body.map(|body| body["result"]["resultType"].clone()).unwrap_or(Value::Null),
        "hasGetContext": has_get_context,
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
        let full_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        let Ok(contents) = fs::read_to_string(&full_path) else {
            continue;
        };
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
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wiki/mcp-reference.md"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"),
    ] {
        let contents = fs::read_to_string(&path).expect("read doc file");
        assert!(
            !contents.contains("roots/list"),
            "docs must not guide clients toward roots/list: {}",
            path.display()
        );
        assert!(
            !contents.contains("output_format"),
            "docs must not advertise output_format: {}",
            path.display()
        );
        assert!(
            !contents.to_ascii_lowercase().contains("toon"),
            "docs must not advertise TOON: {}",
            path.display()
        );
        assert!(
            !contents.contains("ATLAS_MCP_OUTPUT_FORMAT"),
            "docs must not advertise legacy output env: {}",
            path.display()
        );
    }
}

fn cargo_tree(args: &[&str]) -> String {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("cargo")
        .args(args)
        .current_dir(&repo_root)
        .output()
        .expect("run cargo tree");
    assert!(
        output.status.success(),
        "cargo {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("cargo tree stdout utf8")
}

fn package_versions(tree: &str, package: &str) -> BTreeSet<String> {
    tree.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let version = fields.next()?;
            (name == package && version.starts_with('v'))
                .then(|| version.trim_start_matches('v').to_owned())
        })
        .collect()
}

#[test]
fn dependency_tree_omits_toon_format() {
    let tree = cargo_tree(&["tree", "-p", "atlas-mcp", "--prefix", "none"]);
    assert!(
        !tree.contains("toon-format"),
        "atlas-mcp dependency tree must not contain toon-format"
    );
}

#[test]
fn http_dependency_stack_keeps_single_versions_for_key_crates() {
    let tree = cargo_tree(&[
        "tree",
        "-p",
        "atlas-mcp",
        "--features",
        "http-transport",
        "--prefix",
        "none",
    ]);
    let mut versions = BTreeMap::new();
    for package in [
        "axum",
        "hyper",
        "hyper-util",
        "reqwest",
        "tower",
        "tower-http",
        "http",
        "http-body",
        "http-body-util",
    ] {
        let found = package_versions(&tree, package);
        assert!(
            !found.is_empty(),
            "expected {package} in http dependency tree"
        );
        assert!(
            found.len() <= 1,
            "http dependency tree should keep one {package} version, found {:?}",
            found
        );
        versions.insert(package, found);
    }

    assert_eq!(versions["tower-http"].len(), 1);
    assert_eq!(versions["reqwest"].len(), 1);
}
