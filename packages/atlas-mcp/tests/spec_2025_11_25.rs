#![cfg(feature = "http-transport")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use atlas_mcp::testing::{InteractiveStdioTestSession, run_stdio_jsonrpc_session_for_tests};
use atlas_mcp::{MCP_PROTOCOL_VERSION, ServerOptions};
use serde_json::{Value, json};
use tempfile::TempDir;

use atlas_mcp::testing::HttpTestHarness;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("spec_2025_11_25")
        .join("fixtures")
        .join(name)
}

fn read_fixture(name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(fixture_path(name)).expect("read fixture"))
        .expect("fixture json")
}

fn write_repo_file(repo: &Path, path: &str, content: &str) {
    let file_path = repo.join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(file_path, content).expect("write repo file");
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn setup_repo() -> (TempDir, String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    write_repo_file(
        dir.path(),
        "src/lib.rs",
        "pub fn greet() -> &'static str { \"hi\" }\n",
    );
    run_git(dir.path(), &["init", "--quiet"]);
    run_git(dir.path(), &["config", "user.name", "Atlas Tests"]);
    run_git(
        dir.path(),
        &["config", "user.email", "atlas-tests@example.com"],
    );
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "--quiet", "-m", "fixture baseline"]);
    let repo_root = dir.path().to_string_lossy().into_owned();
    let db_path = dir
        .path()
        .join(".atlas")
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();
    (dir, repo_root, db_path)
}

fn initialize_request(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "roots": { "listChanged": true },
                "sampling": {},
                "elicitation": { "form": {}, "url": {} }
            },
            "clientInfo": { "name": "zed", "version": "1.0.0" },
            "_meta": { "clientTag": "spec-suite" }
        }
    })
}

fn initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
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

fn stdio_messages(repo_root: &str, db_path: &str, messages: &[Value]) -> Vec<Value> {
    run_stdio_jsonrpc_session_for_tests(
        &as_lines(messages),
        repo_root,
        db_path,
        ServerOptions::default(),
    )
    .expect("run stdio jsonrpc session")
}

fn stdio_initialized_call(repo_root: &str, db_path: &str, request: Value) -> Value {
    stdio_messages(
        repo_root,
        db_path,
        &[initialize_request(1), initialized_notification(), request],
    )
    .into_iter()
    .find(|value| value["id"] == json!(3))
    .expect("response id=3")
}

fn build_fixture_graph(repo_root: &str, db_path: &str) {
    let response = stdio_initialized_call(
        repo_root,
        db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "build_or_update_graph",
                "arguments": { "mode": "build", "output_format": "json" }
            }
        }),
    );
    assert!(
        response.get("result").is_some(),
        "build response missing result: {response:#}"
    );
}

fn http_session(harness: &HttpTestHarness) -> String {
    let response = harness
        .post_jsonrpc(&[], &initialize_request(1))
        .expect("http initialize");
    assert_eq!(response.status, 200, "http initialize failed: {response:?}");
    response
        .headers
        .get("mcp-session-id")
        .cloned()
        .expect("session header")
}

fn http_headers(session_id: &str) -> [(&str, &str); 2] {
    [
        ("MCP-Protocol-Version", MCP_PROTOCOL_VERSION),
        ("Mcp-Session-Id", session_id),
    ]
}

fn normalize_dynamic(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                match key.as_str() {
                    "repo_root" => *child = json!("<repo-root>"),
                    "db_path" => *child = json!("<db-path>"),
                    "content_db_path" => *child = json!("<content-db-path>"),
                    "session_db_path" => *child = json!("<session-db-path>"),
                    "last_indexed_at" | "last_built_at" | "started_at" | "createdAt"
                    | "lastUpdatedAt" => {
                        if !child.is_null() {
                            *child = json!("<timestamp>");
                        }
                    }
                    "taskId" => *child = json!("<task-id>"),
                    _ => normalize_dynamic(child),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize_dynamic),
        _ => {}
    }
}

fn normalize_http_body(body: &Value) -> Value {
    let mut copy = body.clone();
    normalize_dynamic(&mut copy);
    copy
}

fn tool_name_snapshot(tools_list_response: &Value) -> Value {
    let names = tools_list_response["result"]["tools"]
        .as_array()
        .expect("tools/list tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name").to_owned())
        .collect::<Vec<_>>();
    json!({ "tool_names": names })
}

fn resource_list_snapshot(resources_list_response: &Value) -> Value {
    let resources = resources_list_response["result"]["resources"]
        .as_array()
        .expect("resources/list resources array")
        .iter()
        .map(|resource| {
            json!({
                "uri": resource["uri"],
                "name": resource["name"],
                "mimeType": resource["mimeType"]
            })
        })
        .collect::<Vec<_>>();
    json!({ "resources": resources })
}

fn list_graph_stats_snapshot(tool_call_response: &Value) -> Value {
    let structured = tool_call_response["result"]["structuredContent"].clone();
    let mut structured = structured;
    normalize_dynamic(&mut structured);
    json!({
        "atlas_output_format": tool_call_response["result"]["atlas_output_format"],
        "structured": structured,
    })
}

fn resource_read_status_snapshot(response: &Value) -> Value {
    let content = &response["result"]["contents"][0];
    let mut parsed: Value =
        serde_json::from_str(content["text"].as_str().expect("resource text json"))
            .expect("parse embedded status json");
    normalize_dynamic(&mut parsed);
    json!({
        "uri": content["uri"],
        "mimeType": content["mimeType"],
        "status": {
            "ok": parsed["ok"],
            "error_code": parsed["error_code"],
            "graph_built": parsed["graph_built"],
            "build_state": parsed["build_state"],
            "node_count": parsed["node_count"],
            "edge_count": parsed["edge_count"],
            "file_count": parsed["file_count"],
            "retrieval_available": parsed["retrieval_index"]["available"]
        }
    })
}

fn logging_set_level_snapshot(response: &Value) -> Value {
    json!({ "level": response["result"]["level"] })
}

fn protected_resource_metadata_snapshot(response: &atlas_mcp::testing::TestHttpResponse) -> Value {
    let mut body = response
        .json_body
        .clone()
        .expect("protected resource metadata json body");
    normalize_dynamic(&mut body);
    if let Some(servers) = body
        .get_mut("authorization_servers")
        .and_then(Value::as_array_mut)
    {
        servers
            .iter_mut()
            .for_each(|server| *server = json!("<auth-server>"));
    }
    json!({
        "status": response.status,
        "body": body,
    })
}

fn normalized_jsonrpc_body(response: &Value) -> Value {
    let mut body = response.clone();
    normalize_dynamic(&mut body);
    if let Some(object) = body.as_object_mut() {
        object.remove("id");
    }
    body
}

fn normalized_http_jsonrpc_body(response: &atlas_mcp::testing::TestHttpResponse) -> Value {
    let mut body = normalize_http_body(response.json_body.as_ref().expect("http json body"));
    if let Some(object) = body.as_object_mut() {
        object.remove("id");
    }
    body
}

fn initialize_success_stdio_snapshot(response: &Value) -> Value {
    let mut body = response.clone();
    normalize_dynamic(&mut body);
    json!({ "body": body })
}

fn initialize_success_http_snapshot(response: &atlas_mcp::testing::TestHttpResponse) -> Value {
    json!({
        "status": response.status,
        "protocol_header": response.headers.get("mcp-protocol-version"),
        "has_session_header": response.headers.contains_key("mcp-session-id"),
        "body": normalize_http_body(response.json_body.as_ref().expect("json body")),
    })
}

fn initialize_rejection_http_snapshot(response: &atlas_mcp::testing::TestHttpResponse) -> Value {
    json!({
        "status": response.status,
        "body": normalize_http_body(response.json_body.as_ref().expect("json body")),
    })
}

fn task_lifecycle_stdio_snapshot(repo_root: &str, db_path: &str) -> Value {
    let create = stdio_initialized_call(
        repo_root,
        db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "doctor",
                "task": { "ttl": 1000 },
                "arguments": { "output_format": "json" }
            }
        }),
    );
    let task_id = create["result"]["task"]["taskId"]
        .as_str()
        .expect("task id")
        .to_owned();

    let mut final_status = String::from("working");
    for _ in 0..50 {
        let get = stdio_initialized_call(
            repo_root,
            db_path,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tasks/get",
                "params": { "taskId": task_id }
            }),
        );
        final_status = get["result"]["status"]
            .as_str()
            .expect("task status")
            .to_owned();
        if final_status != "working" {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let mut result = stdio_initialized_call(
        repo_root,
        db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tasks/result",
            "params": { "taskId": task_id }
        }),
    )["result"]
        .clone();
    normalize_dynamic(&mut result);
    json!({
        "created_status": create["result"]["task"]["status"],
        "final_status": final_status,
        "result_ok": result["ok"],
        "check_count": result["checks"].as_array().map(Vec::len).unwrap_or_default(),
    })
}

fn task_lifecycle_http_snapshot(harness: &HttpTestHarness) -> Value {
    let session_id = http_session(harness);
    let create = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "doctor",
                    "task": { "ttl": 1000 },
                    "arguments": { "output_format": "json" }
                }
            }),
        )
        .expect("http task create");
    let create_body = create.json_body.expect("create json body");
    let task_id = create_body["result"]["task"]["taskId"]
        .as_str()
        .expect("task id")
        .to_owned();

    let mut final_status = String::from("working");
    for _ in 0..50 {
        let get = harness
            .post_jsonrpc(
                &http_headers(&session_id),
                &json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tasks/get",
                    "params": { "taskId": task_id }
                }),
            )
            .expect("http task get");
        let body = get.json_body.expect("get json body");
        final_status = body["result"]["status"]
            .as_str()
            .expect("task status")
            .to_owned();
        if final_status != "working" {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let result = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tasks/result",
                "params": { "taskId": task_id }
            }),
        )
        .expect("http task result");
    let mut result_body = result.json_body.expect("result json body")["result"].clone();
    normalize_dynamic(&mut result_body);
    json!({
        "created_status": create_body["result"]["task"]["status"],
        "final_status": final_status,
        "result_ok": result_body["ok"],
        "check_count": result_body["checks"].as_array().map(Vec::len).unwrap_or_default(),
    })
}

fn parse_sse_json(body: &str) -> Value {
    let data_line = body
        .lines()
        .find(|line| line.starts_with("data: "))
        .expect("SSE data line");
    serde_json::from_str(data_line.trim_start_matches("data: ")).expect("SSE json payload")
}

fn elicitation_round_trip_snapshot(reverse_request: &Value, final_response: &Value) -> Value {
    json!({
        "method": reverse_request["method"],
        "mode": reverse_request["params"]["mode"],
        "message": reverse_request["params"]["message"],
        "schema": reverse_request["params"]["requestedSchema"],
        "result": {
            "deleted_source_count": final_response["result"]["structuredContent"]["deleted_source_count"],
            "deleted_bridge_file_count": final_response["result"]["structuredContent"]["deleted_bridge_file_count"],
            "keep_days": final_response["result"]["structuredContent"]["keep_days"]
        }
    })
}

fn elicitation_round_trip_stdio_snapshot(repo_root: &str, db_path: &str) -> Value {
    let session = InteractiveStdioTestSession::start(repo_root, db_path, ServerOptions::default())
        .expect("start stdio interactive test session");
    session
        .send_json(&initialize_request(1))
        .expect("send stdio initialize");
    let initialize = session
        .recv_json(Duration::from_secs(2))
        .expect("recv stdio initialize")
        .expect("stdio initialize response");
    assert_eq!(
        initialize["result"]["protocolVersion"],
        json!(MCP_PROTOCOL_VERSION)
    );
    session
        .send_json(&initialized_notification())
        .expect("send stdio initialized notification");
    session
        .send_json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "purge_saved_context",
                "arguments": {
                    "keep_days": 30,
                    "output_format": "json"
                }
            }
        }))
        .expect("send purge_saved_context call");
    let reverse_request = session
        .recv_json(Duration::from_secs(2))
        .expect("recv stdio reverse request")
        .expect("stdio reverse request");
    let reverse_request_id = reverse_request["id"].clone();
    session
        .send_json(&json!({
            "jsonrpc": "2.0",
            "id": reverse_request_id,
            "result": {
                "action": "accept",
                "content": {
                    "confirmation": "confirm"
                }
            }
        }))
        .expect("send stdio reverse response");
    let final_response = session
        .recv_json(Duration::from_secs(2))
        .expect("recv stdio final response")
        .expect("stdio final response");
    let _ = session.finish().expect("finish stdio session");
    elicitation_round_trip_snapshot(&reverse_request, &final_response)
}

fn elicitation_round_trip_http_snapshot(harness: &HttpTestHarness) -> Value {
    let session_id = http_session(harness);
    harness
        .post_jsonrpc(&http_headers(&session_id), &initialized_notification())
        .expect("http initialized notification");

    let session_id_for_worker = session_id.clone();
    let harness_for_worker = harness.clone();
    let worker = thread::spawn(move || {
        harness_for_worker.post_jsonrpc(
            &http_headers(&session_id_for_worker),
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "purge_saved_context",
                    "arguments": {
                        "keep_days": 30,
                        "output_format": "json"
                    }
                }
            }),
        )
    });

    let reverse_request = loop {
        let poll = harness
            .get_mcp(&http_headers(&session_id))
            .expect("http reverse-request poll");
        if poll.status == 200 && poll.body_text.contains("\"method\":\"elicitation/create\"") {
            break parse_sse_json(&poll.body_text);
        }
        thread::sleep(Duration::from_millis(20));
    };

    harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({
                "jsonrpc": "2.0",
                "id": reverse_request["id"],
                "result": {
                    "action": "accept",
                    "content": {
                        "confirmation": "confirm"
                    }
                }
            }),
        )
        .expect("http reverse response");

    let final_response = worker
        .join()
        .expect("join http tool call thread")
        .expect("http final response");
    elicitation_round_trip_snapshot(
        &reverse_request,
        final_response
            .json_body
            .as_ref()
            .expect("http final json body"),
    )
}

#[test]
fn spec_fixture_stdio_initialize_success_matches_golden() {
    let (_dir, repo_root, db_path) = setup_repo();
    let response = stdio_messages(&repo_root, &db_path, &[initialize_request(1)])
        .into_iter()
        .find(|value| value["id"] == json!(1))
        .expect("initialize response");
    assert_eq!(
        initialize_success_stdio_snapshot(&response),
        read_fixture("initialize_success.stdio.json")
    );
}

#[test]
fn spec_fixture_stdio_initialize_rejection_matches_golden() {
    let (_dir, repo_root, db_path) = setup_repo();
    let response = stdio_messages(
        &repo_root,
        &db_path,
        &[json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {}
            }
        })],
    )
    .into_iter()
    .find(|value| value["id"] == json!(1))
    .expect("initialize error response");
    let mut snapshot = response.clone();
    normalize_dynamic(&mut snapshot);
    assert_eq!(
        json!({ "body": snapshot }),
        read_fixture("initialize_rejection.stdio.json")
    );
}

#[test]
fn spec_fixture_http_initialize_success_and_rejection_match_golden() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    let success = harness
        .post_jsonrpc(&[], &initialize_request(1))
        .expect("http initialize success");
    assert_eq!(
        initialize_success_http_snapshot(&success),
        read_fixture("initialize_success.http.json")
    );

    let rejection = harness
        .post_jsonrpc(
            &[],
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {}
                }
            }),
        )
        .expect("http initialize rejection");
    assert_eq!(
        initialize_rejection_http_snapshot(&rejection),
        read_fixture("initialize_rejection.http.json")
    );
}

#[test]
fn spec_fixture_elicitation_round_trip_matches_golden() {
    let (_dir, repo_root, db_path) = setup_repo();
    assert_eq!(
        elicitation_round_trip_stdio_snapshot(&repo_root, &db_path),
        read_fixture("elicitation_round_trip.snapshot.json")
    );
    let harness = HttpTestHarness::new(&repo_root, &db_path);
    assert_eq!(
        elicitation_round_trip_http_snapshot(&harness),
        read_fixture("elicitation_round_trip.snapshot.json")
    );
}

#[test]
fn spec_fixture_http_protected_resource_metadata_matches_golden() {
    let (_dir, repo_root, db_path) = setup_repo();
    let harness =
        HttpTestHarness::new_with_test_auth(&repo_root, &db_path, &["https://app.atlas.test"])
            .expect("http auth harness");
    let response = harness
        .get_metadata(&[("Origin", "https://app.atlas.test")])
        .expect("http protected-resource metadata");
    assert_eq!(
        protected_resource_metadata_snapshot(&response),
        read_fixture("protected_resource_metadata.snapshot.json")
    );
    assert_eq!(
        response.headers.get("access-control-allow-origin"),
        Some(&"https://app.atlas.test".to_owned())
    );
}

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
                "arguments":{"output_format":"json"}
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
                    "arguments":{"output_format":"json"}
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
    assert_eq!(
        logging_set_level_snapshot(&stdio_logging),
        read_fixture("logging_set_level.snapshot.json")
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
    assert_eq!(
        logging_set_level_snapshot(http_logging.json_body.as_ref().expect("logging json body")),
        read_fixture("logging_set_level.snapshot.json")
    );

    assert_eq!(
        task_lifecycle_stdio_snapshot(&repo_root, &db_path),
        read_fixture("task_lifecycle.snapshot.json")
    );
    assert_eq!(
        task_lifecycle_http_snapshot(&harness),
        read_fixture("task_lifecycle.snapshot.json")
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
                "params":{"name":"list_graph_stats","arguments":{"output_format":"json"}}
            }),
            json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{"name":"list_graph_stats","arguments":{"output_format":"json"}}
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
    let get_missing = harness.get_mcp(&[]).expect("get missing bearer");
    let delete_missing = harness.delete_mcp(&[]).expect("delete missing bearer");
    for response in [&post_missing, &get_missing, &delete_missing] {
        assert_eq!(response.status, 401);
        assert!(
            response
                .headers
                .get("www-authenticate")
                .expect("www-authenticate")
                .contains("error=\"invalid_token\"")
        );
    }

    let invalid_auth = [("Authorization", "Bearer not-a-jwt")];
    let post_invalid = harness
        .post_jsonrpc(&invalid_auth, &initialize_request(1))
        .expect("post invalid bearer");
    let get_invalid = harness.get_mcp(&invalid_auth).expect("get invalid bearer");
    let delete_invalid = harness
        .delete_mcp(&invalid_auth)
        .expect("delete invalid bearer");
    for response in [&post_invalid, &get_invalid, &delete_invalid] {
        assert_eq!(response.status, 401);
        assert!(
            response
                .headers
                .get("www-authenticate")
                .expect("www-authenticate")
                .contains("invalid bearer token")
        );
    }

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
    let session_id = initialized
        .headers
        .get("mcp-session-id")
        .cloned()
        .expect("session header");
    let post_missing_version = harness
        .post_jsonrpc(
            &[
                ("Authorization", bearer.as_str()),
                ("Mcp-Session-Id", session_id.as_str()),
            ],
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        )
        .expect("post missing protocol header");
    let get_missing_version = harness
        .get_mcp(&[("Authorization", bearer.as_str())])
        .expect("get missing protocol header");
    let delete_missing_version = harness
        .delete_mcp(&[("Authorization", bearer.as_str())])
        .expect("delete missing protocol header");
    for response in [
        &post_missing_version,
        &get_missing_version,
        &delete_missing_version,
    ] {
        assert_eq!(response.status, 400);
        assert_eq!(
            response.headers.get("mcp-protocol-version"),
            Some(&MCP_PROTOCOL_VERSION.to_owned())
        );
    }

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
        harness
            .get_mcp(&forbidden_origin)
            .expect("get forbidden origin"),
        harness
            .delete_mcp(&forbidden_origin)
            .expect("delete forbidden origin"),
    ] {
        assert_eq!(response.status, 403);
        assert!(!response.headers.contains_key("www-authenticate"));
    }
}

#[test]
fn spec_capabilities_and_descriptors_omit_unimplemented_methods() {
    let (_dir, repo_root, db_path) = setup_repo();
    let initialize = stdio_messages(&repo_root, &db_path, &[initialize_request(1)])
        .into_iter()
        .find(|value| value["id"] == json!(1))
        .expect("initialize response");
    let capabilities = initialize["result"]["capabilities"]
        .as_object()
        .expect("capabilities object");
    let mut capability_keys = capabilities.keys().cloned().collect::<Vec<_>>();
    capability_keys.sort();
    assert_eq!(
        capability_keys,
        vec![
            "completions".to_owned(),
            "experimental".to_owned(),
            "logging".to_owned(),
            "prompts".to_owned(),
            "resources".to_owned(),
            "tasks".to_owned(),
            "tools".to_owned(),
        ]
    );
    for absent in ["roots", "sampling", "elicitation"] {
        assert!(
            !capabilities.contains_key(absent),
            "capability {absent} must stay absent until implemented"
        );
    }

    let tools = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}),
    );
    let tool_names = tools["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    for absent in [
        "elicitation/create",
        "roots/list",
        "sampling/createMessage",
        "tasks/get",
        "tasks/result",
        "tasks/list",
        "tasks/cancel",
    ] {
        assert!(
            !tool_names.contains(&absent),
            "non-tool or unimplemented method {absent} must not appear in tool descriptors"
        );
    }
}
