use std::collections::HashMap;

use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use atlas_core::EdgeKind;

use atlas_core::kinds::NodeKind;
use atlas_core::model::{Edge, Node, NodeId};
use atlas_store_sqlite::Store;
use rusqlite::Connection;
use tempfile::TempDir;

use crate::MCP_PROTOCOL_VERSION;

use super::ServerOptions;
use super::broker::{ReverseRequestBroker, ReverseRequestEmitter};
use super::stdio::{
    InteractiveStdioTestSession, StdioTestScriptResult,
    run_stdio_jsonrpc_session_capture_for_tests, run_stdio_jsonrpc_session_for_tests,
};

// ── Helper functions ────────────────────────────────────────────────────

struct TransportFixture {
    _dir: TempDir,
    db_path: String,
}

fn make_node(kind: NodeKind, name: &str, qualified_name: &str, file_path: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        file_path: file_path.to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: kind == NodeKind::Test,
        file_hash: format!("hash:{file_path}"),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    }
}

fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file_path: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    }
}

fn setup_graph_repo_fixture(
    primary_file: &str,
    primary_name: &str,
    primary_qn: &str,
) -> TransportFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join(".atlas").join("worldtree.db");
    std::fs::create_dir_all(db_path.parent().expect("atlas dir")).expect("create atlas dir");
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");

    let primary = make_node(NodeKind::Function, primary_name, primary_qn, primary_file);
    store
        .replace_file_graph(
            primary_file,
            &format!("hash:{primary_file}"),
            Some("rust"),
            Some(5),
            std::slice::from_ref(&primary),
            &[],
        )
        .expect("replace primary graph");

    TransportFixture { _dir: dir, db_path }
}

fn setup_fixture() -> TransportFixture {
    let fixture =
        setup_graph_repo_fixture("src/service.rs", "compute", "src/service.rs::fn::compute");
    let mut store = Store::open(&fixture.db_path).expect("reopen store");

    let handle = make_node(
        NodeKind::Function,
        "handle_request",
        "src/api.rs::fn::handle_request",
        "src/api.rs",
    );
    let handle_calls_compute = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    store
        .replace_file_graph(
            "src/api.rs",
            "hash:src/api.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&handle),
            &[handle_calls_compute],
        )
        .expect("replace api graph");

    fixture
}

fn run_rmcp_script(
    repo_root: &str,
    db_path: &str,
    input: &str,
    options: ServerOptions,
) -> Vec<serde_json::Value> {
    run_stdio_jsonrpc_session_for_tests(input, repo_root, db_path, options)
        .expect("run rmcp stdio script")
}

fn run_rmcp_script_capture(
    repo_root: &str,
    db_path: &str,
    input: &str,
    options: ServerOptions,
) -> StdioTestScriptResult {
    run_stdio_jsonrpc_session_capture_for_tests(input, repo_root, db_path, options)
        .expect("run rmcp stdio capture script")
}

const METHOD_NOT_FOUND_CODE: i64 = -32601;
const UNSUPPORTED_PROTOCOL_VERSION_CODE: i64 = -32022;

fn initialize_request_line() -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"zed\",\"version\":\"1.0.0\"}}}}}}\n",
        MCP_PROTOCOL_VERSION
    )
}

fn request_meta_params() -> serde_json::Value {
    serde_json::json!({
        "_meta": {
            crate::spec::META_PROTOCOL_VERSION: MCP_PROTOCOL_VERSION,
            crate::spec::META_CLIENT_CAPABILITIES: {},
            crate::spec::META_CLIENT_INFO: {
                "name": "zed",
                "version": "1.0.0"
            }
        }
    })
}

fn stdio_single_response(
    repo_root: &str,
    db_path: &str,
    request: serde_json::Value,
) -> serde_json::Value {
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        serde_json::to_string(&request).expect("serialize request") + "\n",
    ]
    .concat();
    run_stdio_jsonrpc_session_for_tests(&input, repo_root, db_path, ServerOptions::default())
        .expect("run stdio request")
        .into_iter()
        .find(|value| value["id"] == request["id"])
        .expect("stdio response by id")
}

fn stdio_single_response_2026(
    repo_root: &str,
    db_path: &str,
    request: serde_json::Value,
) -> serde_json::Value {
    let input = serde_json::to_string(&request).expect("serialize request") + "\n";
    run_stdio_jsonrpc_session_for_tests(&input, repo_root, db_path, ServerOptions::default())
        .expect("run stdio request")
        .into_iter()
        .find(|value| value["id"] == request["id"])
        .expect("stdio response by id")
}

struct TestReverseEmitter {
    sent: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl ReverseRequestEmitter for TestReverseEmitter {
    fn emit_request(&self, request: serde_json::Value) -> Result<()> {
        self.sent
            .lock()
            .expect("test reverse emitter lock poisoned")
            .push(request);
        Ok(())
    }
}

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
fn unsupported_request_protocol_version_returns_mcp_error() {
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
                    crate::spec::META_PROTOCOL_VERSION: "2025-11-25",
                    crate::spec::META_CLIENT_CAPABILITIES: {}
                }
            }
        }),
    );

    assert_eq!(
        response["error"]["code"],
        serde_json::json!(UNSUPPORTED_PROTOCOL_VERSION_CODE)
    );
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
        serde_json::json!([MCP_PROTOCOL_VERSION])
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

#[test]
fn reverse_request_broker_times_out_and_cleans_up() {
    let broker = ReverseRequestBroker::new();
    let emitter: Arc<dyn ReverseRequestEmitter> = Arc::new(TestReverseEmitter {
        sent: Arc::new(Mutex::new(Vec::new())),
    });
    let error = broker
        .issue_request(
            "stdio:1",
            &emitter,
            "elicitation/create",
            serde_json::json!({"mode":"form"}),
            Duration::from_millis(5),
        )
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(broker.is_pending_empty());
}

#[test]
fn reverse_request_broker_enforces_scope_correlation() {
    let broker = ReverseRequestBroker::new();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let emitter: Arc<dyn ReverseRequestEmitter> = Arc::new(TestReverseEmitter {
        sent: Arc::clone(&sent),
    });
    let broker_for_thread = broker.clone();
    let emitter_for_thread = Arc::clone(&emitter);
    let handle = thread::spawn(move || {
        broker_for_thread.issue_request(
            "http:session-a:2",
            &emitter_for_thread,
            "elicitation/create",
            serde_json::json!({"mode":"form"}),
            Duration::from_secs(1),
        )
    });
    let request_id = (0..50)
        .find_map(|_| {
            let maybe = sent
                .lock()
                .expect("sent lock poisoned")
                .first()
                .and_then(|value| value.get("id"))
                .cloned();
            if maybe.is_none() {
                thread::sleep(Duration::from_millis(5));
            }
            maybe
        })
        .expect("reverse request id");
    assert!(!broker.try_resolve_response_for_scope(
        Some("http:session-b:"),
        &serde_json::json!({"jsonrpc":"2.0","id": request_id.clone(),"result":{"ok":true}}),
    ));
    assert!(broker.try_resolve_response_for_scope(
        Some("http:session-a:"),
        &serde_json::json!({"jsonrpc":"2.0","id": request_id,"result":{"ok":true}}),
    ));
    assert_eq!(
        handle.join().expect("join reverse request thread").unwrap(),
        serde_json::json!({"ok":true})
    );
}

#[test]
fn purge_saved_context_stdio_returns_input_required_without_reverse_request() {
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
                "capabilities": { "elicitation": { "form": {} } },
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
                "name": "purge_saved_context",
                "arguments": { "keep_days": 30 },
                "_meta": {
                    crate::spec::META_PROTOCOL_VERSION: MCP_PROTOCOL_VERSION,
                    crate::spec::META_CLIENT_CAPABILITIES: {
                        "elicitation": { "form": {} }
                    },
                    crate::spec::META_CLIENT_INFO: {
                        "name": "zed",
                        "version": "1.0.0"
                    }
                }
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("tools/call response");
    assert_eq!(
        response["result"]["resultType"],
        serde_json::json!("input_required")
    );
    assert!(
        response["result"]["inputRequests"]
            .get("confirmation")
            .is_some()
    );
    assert!(
        session
            .recv_json(Duration::from_millis(100))
            .unwrap()
            .is_none(),
        "server must not emit outbound elicitation/create reverse request"
    );

    let _ = session.finish().unwrap();
}

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
    let input = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"progressToken\":\"tok-1\",\"arguments\":{\"text\":\"compute\"},\"_meta\":{\"io.modelcontextprotocol/protocolVersion\":\"2026-07-28\",\"io.modelcontextprotocol/clientCapabilities\":{},\"io.modelcontextprotocol/clientInfo\":{\"name\":\"zed\",\"version\":\"1.0.0\"},\"io.modelcontextprotocol/logLevel\":\"warning\"}}}\n";
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let responses = run_rmcp_script(
        &repo_root,
        &fixture.db_path,
        input,
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
    let transport_mod = include_str!("mod.rs");
    assert!(!transport_mod.contains("socket_dispatch"));
    assert!(!transport_mod.contains("socket_input"));
    assert!(!transport_mod.contains("socket_io"));
    assert!(!transport_mod.contains("socket_jsonrpc"));
    assert!(!transport_mod.contains("socket_notify"));

    let socket_source = include_str!("socket.rs");
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
