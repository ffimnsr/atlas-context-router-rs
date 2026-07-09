use std::collections::HashMap;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use atlas_core::EdgeKind;
use atlas_core::error_code_docs_ref;
use atlas_core::kinds::NodeKind;
use atlas_core::model::{Edge, Node, NodeId};
use atlas_store_sqlite::Store;
use rusqlite::Connection;
use tempfile::TempDir;
use url::Url;

use crate::MCP_PROTOCOL_VERSION;

use super::ServerOptions;
use super::broker::{ReverseRequestBroker, ReverseRequestEmitter};
use super::io::run_server_io;
use super::jsonrpc::JsonRpcErrorKind;
use super::stdio::{InteractiveStdioTestSession, run_stdio_jsonrpc_session_for_tests};

// ── Helper functions ────────────────────────────────────────────────────

fn assert_error_code_doc_link(actual: &serde_json::Value, error_code: &str) {
    assert_eq!(actual, &serde_json::json!(error_code_docs_ref(error_code)));

    let catalog_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/error_codes.md");
    let catalog = std::fs::read_to_string(&catalog_path).expect("read docs/error_codes.md");
    assert!(
        catalog.contains(&format!("<a id=\"{error_code}\"></a>")),
        "docs/error_codes.md missing anchor for {error_code}"
    );
}

struct TransportFixture {
    _dir: TempDir,
    db_path: String,
}

struct TextRepoFixture {
    _dir: TempDir,
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

fn setup_text_repo(files: &[(&str, &str)]) -> TextRepoFixture {
    let dir = tempfile::tempdir().expect("text repo tempdir");
    for (path, content) in files {
        let abs = dir.path().join(path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).expect("create text repo parent");
        }
        std::fs::write(abs, content).expect("write text repo file");
    }
    TextRepoFixture { _dir: dir }
}

fn initialize_dynamic_session(
    session: &InteractiveStdioTestSession,
    roots_capability: bool,
) -> serde_json::Value {
    let capabilities = if roots_capability {
        serde_json::json!({ "roots": { "listChanged": true } })
    } else {
        serde_json::json!({})
    };
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": capabilities,
                "clientInfo": { "name": "zed", "version": "1.0.0" }
            }
        }))
        .unwrap();
    let initialize = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("initialize response");
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }))
        .unwrap();
    initialize
}

fn parse_output_lines(output: Vec<u8>) -> Vec<serde_json::Value> {
    String::from_utf8(output)
        .expect("utf8 output")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("jsonrpc response line"))
        .collect()
}

fn initialize_request_line() -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"zed\",\"version\":\"1.0.0\"}}}}}}\n",
        MCP_PROTOCOL_VERSION
    )
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

    fn emit_task_status(&self, _params: serde_json::Value) -> Result<()> {
        Ok(())
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn advertised_capabilities_have_stdio_method_handlers_and_descriptor_backing() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let capabilities = crate::spec::initialize_capabilities();

    assert!(capabilities.tools == crate::spec::EmptyCapability::default());
    assert!(capabilities.completions == crate::spec::EmptyCapability::default());
    assert!(capabilities.logging == crate::spec::EmptyCapability::default());
    assert!(capabilities.tasks.is_some());
    assert!(capabilities.experimental.is_some());

    for request in [
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"status","arguments":{"output_format":"json"}}
        }),
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"resources/list","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":5,"method":"resources/templates/list","params":{}}),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":6,
            "method":"resources/read",
            "params":{"uri":"atlas://health/status"}
        }),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":7,
            "method":"completion/complete",
            "params":{"ref":{"name":"tools/call"},"argument":{"name":"output_format","value":"j"}}
        }),
        serde_json::json!({"jsonrpc":"2.0","id":8,"method":"logging/setLevel","params":{"level":"warning"}}),
        serde_json::json!({"jsonrpc":"2.0","id":9,"method":"prompts/list","params":{}}),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":10,
            "method":"prompts/get",
            "params":{"name":"inspect_symbol","arguments":{"symbol":"compute"}}
        }),
        serde_json::json!({"jsonrpc":"2.0","id":11,"method":"tasks/list","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":12,"method":"tasks/get","params":{"taskId":"missing"}}),
        serde_json::json!({"jsonrpc":"2.0","id":13,"method":"tasks/result","params":{"taskId":"missing"}}),
        serde_json::json!({"jsonrpc":"2.0","id":14,"method":"tasks/cancel","params":{"taskId":"missing"}}),
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
            Some(JsonRpcErrorKind::MethodNotFound.code() as i64),
            "method {} must be handled",
            request["method"].as_str().expect("method")
        );
    }

    let tool_list = crate::tools::tool_list();
    let tools = tool_list["tools"].as_array().expect("tool descriptors");
    assert!(
        !tools.is_empty(),
        "tools/list descriptors must not be empty"
    );

    let prompt_list = crate::prompts::prompt_list();
    let prompts = prompt_list["prompts"]
        .as_array()
        .expect("prompt descriptors");
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
fn dynamic_roots_resolve_repo_before_first_tool_call() {
    let fixture = setup_fixture();
    let repo_root = fixture._dir.path().to_string_lossy().into_owned();
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();

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
    let initialize = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("initialize response");
    assert_eq!(initialize["id"], serde_json::json!(1));

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
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();

    let roots_request = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("roots/list request");
    assert_eq!(roots_request["method"], serde_json::json!("roots/list"));
    let roots_id = roots_request["id"].clone();
    let roots_uri = Url::from_directory_path(fixture._dir.path())
        .expect("fixture root url")
        .to_string();
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": roots_id,
            "result": {
                "roots": [
                    { "uri": roots_uri.clone(), "name": "fixture" }
                ]
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("query_graph response");
    assert_eq!(
        response["result"]["_meta"]["atlas:repoSelection"]["sessionMode"],
        serde_json::json!("dynamic")
    );
    assert_eq!(
        response["result"]["_meta"]["atlas:repoSelection"]["selectionSource"],
        serde_json::json!("single_root")
    );
    assert_eq!(
        response["result"]["_meta"]["atlas:repoRoot"],
        serde_json::json!(repo_root)
    );
    let format = response["result"]["_meta"]["atlas:outputFormat"]
        .as_str()
        .unwrap_or("toon");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("query_graph response text");
    if format == "json" {
        let query_value: serde_json::Value =
            serde_json::from_str(text).expect("query_graph payload json");
        assert_eq!(
            query_value[0]["qn"],
            serde_json::json!("src/service.rs::fn::compute")
        );
        assert_eq!(query_value[0]["file"], serde_json::json!("src/service.rs"));
    } else {
        assert!(text.contains("src/service.rs::fn::compute"));
        assert!(text.contains("src/service.rs"));
    }

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();
    let cached_response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("cached query_graph response");
    assert_eq!(cached_response["id"], serde_json::json!(3));
    assert_eq!(
        cached_response["result"]["_meta"]["atlas:repoSelection"]["selectionSource"],
        serde_json::json!("cached_active_root")
    );
    assert_eq!(
        cached_response["result"]["_meta"]["atlas:repoSelection"]["usedCachedSelection"],
        serde_json::json!(true)
    );
    assert!(
        session
            .recv_json(Duration::from_millis(150))
            .unwrap()
            .is_none(),
        "cached active repo must prevent a second roots/list reverse request"
    );

    let _ = session.finish().unwrap();
    assert!(roots_uri.contains(&repo_root));
}

#[test]
fn dynamic_roots_require_initialized_before_repo_bound_tool_call() {
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();
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
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute" }
            }
        }))
        .unwrap();
    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("pre-initialized error response");
    assert_eq!(response["id"], serde_json::json!(2));
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("wait for `initialized` before repo-bound tool calls")
    );
    assert_eq!(
        response["error"]["data"]["atlas_repo_selection"]["failure_kind"],
        serde_json::json!("request_before_initialized")
    );
    let _ = session.finish().unwrap();
}

#[test]
fn dynamic_roots_require_client_roots_capability() {
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();
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
                "arguments": { "text": "compute" }
            }
        }))
        .unwrap();
    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("missing roots capability response");
    assert_eq!(response["id"], serde_json::json!(2));
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("did not advertise roots capability")
    );
    assert_eq!(
        response["error"]["data"]["atlas_repo_selection"]["failure_kind"],
        serde_json::json!("client_lacks_roots_capability")
    );
    let _ = session.finish().unwrap();
}

#[test]
fn fixed_repo_mode_ignores_conflicting_client_roots() {
    let fixture = setup_fixture();
    let session = InteractiveStdioTestSession::start(
        fixture._dir.path().to_string_lossy().as_ref(),
        &fixture.db_path,
        ServerOptions::default(),
    )
    .unwrap();

    let _ = initialize_dynamic_session(&session, true);
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("fixed-mode query response");
    assert_eq!(response["id"], serde_json::json!(2));
    assert!(
        session
            .recv_json(Duration::from_millis(150))
            .unwrap()
            .is_none(),
        "fixed mode must ignore client roots and skip roots/list reverse requests"
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
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();
    let second = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("fixed-mode post-roots-changed response");
    assert_eq!(second["id"], serde_json::json!(3));
    assert!(
        session
            .recv_json(Duration::from_millis(150))
            .unwrap()
            .is_none(),
        "fixed mode must ignore roots/list_changed invalidation"
    );
    let _ = session.finish().unwrap();
}

#[test]
fn multi_root_file_bearing_tool_selects_matching_root() {
    let repo_a = setup_text_repo(&[("src/alpha.rs", "pub fn alpha() {}\n")]);
    let repo_b = setup_text_repo(&[("src/beta.rs", "pub fn beta() {}\n")]);
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();

    let _ = initialize_dynamic_session(&session, true);
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "read_file_excerpt",
                "arguments": {
                    "file": "src/beta.rs",
                    "start_line": 1,
                    "end_line": 1
                }
            }
        }))
        .unwrap();

    let roots_request = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("roots/list request");
    let roots_id = roots_request["id"].clone();
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": roots_id,
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string(), "name": "a" },
                    { "uri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string(), "name": "b" }
                ]
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("read_file_excerpt response");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("excerpt text");
    assert!(text.contains("pub fn beta() {}"));
    let _ = session.finish().unwrap();
}

#[test]
fn multi_root_same_relative_path_collision_fails_closed() {
    let repo_a = setup_text_repo(&[("src/shared.rs", "pub fn left() {}\n")]);
    let repo_b = setup_text_repo(&[("src/shared.rs", "pub fn right() {}\n")]);
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();

    let _ = initialize_dynamic_session(&session, true);
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "read_file_excerpt",
                "arguments": {
                    "file": "src/shared.rs",
                    "start_line": 1,
                    "end_line": 1
                }
            }
        }))
        .unwrap();

    let roots_request = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("roots/list request");
    let roots_id = roots_request["id"].clone();
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": roots_id,
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string(), "name": "a" },
                    { "uri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string(), "name": "b" }
                ]
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("collision error response");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("same relative paths exist in more than one root")
    );
    assert_eq!(
        response["error"]["data"]["atlas_repo_selection"]["failure_kind"],
        serde_json::json!("multiple_roots_insufficient_evidence")
    );
    assert_eq!(
        response["error"]["data"]["atlas_repo_selection"]["tool_name"],
        serde_json::json!("read_file_excerpt")
    );
    assert!(
        response["error"]["data"]["atlas_repo_selection"]["candidate_roots"]
            .as_array()
            .map(|items| items.len() == 2)
            .unwrap_or(false)
    );
    let _ = session.finish().unwrap();
}

#[test]
fn roots_list_changed_invalidates_cached_dynamic_root_and_reresolves() {
    let repo_a = setup_text_repo(&[("src/alpha.rs", "pub fn alpha() {}\n")]);
    let repo_b = setup_text_repo(&[("src/beta.rs", "pub fn beta() {}\n")]);
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();

    let _ = initialize_dynamic_session(&session, true);
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "read_file_excerpt",
                "arguments": {
                    "file": "src/alpha.rs",
                    "start_line": 1,
                    "end_line": 1
                }
            }
        }))
        .unwrap();
    let first_roots = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("first roots/list request");
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": first_roots["id"].clone(),
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string(), "name": "a" }
                ]
            }
        }))
        .unwrap();
    let first = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("first excerpt response");
    assert!(
        first["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("pub fn alpha() {}")
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
                "name": "read_file_excerpt",
                "arguments": {
                    "file": "src/beta.rs",
                    "start_line": 1,
                    "end_line": 1
                }
            }
        }))
        .unwrap();
    let second_roots = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("second roots/list request");
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": second_roots["id"].clone(),
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string(), "name": "b" }
                ]
            }
        }))
        .unwrap();
    let second = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("second excerpt response");
    assert!(
        second["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("pub fn beta() {}")
    );
    let _ = session.finish().unwrap();
}

#[test]
fn multi_root_query_only_tool_uses_request_active_root_hint() {
    let repo_a = setup_graph_repo_fixture("src/alpha.rs", "compute", "src/alpha.rs::fn::compute");
    let repo_b = setup_graph_repo_fixture("src/beta.rs", "compute", "src/beta.rs::fn::compute");
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();

    let _ = initialize_dynamic_session(&session, true);
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "_meta": {
                "atlas": {
                    "activeRootUri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string()
                }
            },
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();

    let roots_request = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("roots/list request");
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": roots_request["id"].clone(),
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string(), "name": "a" },
                    { "uri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string(), "name": "b" }
                ]
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("query_graph response");
    let query_value: serde_json::Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("query_graph json payload"),
    )
    .expect("parse query_graph payload");
    assert_eq!(query_value[0]["file"], serde_json::json!("src/beta.rs"));
    assert_eq!(
        response["result"]["_meta"]["atlas:repoSelection"]["selectionSource"],
        serde_json::json!("client_hint")
    );
    let _ = session.finish().unwrap();
}

#[test]
fn invalid_request_active_root_hint_fails_closed_even_with_cached_root() {
    let repo_a = setup_graph_repo_fixture("src/alpha.rs", "compute", "src/alpha.rs::fn::compute");
    let repo_b = setup_graph_repo_fixture("src/beta.rs", "compute", "src/beta.rs::fn::compute");
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();

    let _ = initialize_dynamic_session(&session, true);
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "_meta": {
                "atlas": {
                    "activeRootUri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string()
                }
            },
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();
    let first_roots = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("first roots/list request");
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": first_roots["id"].clone(),
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string(), "name": "a" },
                    { "uri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string(), "name": "b" }
                ]
            }
        }))
        .unwrap();
    let _ = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("cached repo seed response");

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "_meta": {
                "atlas": {
                    "activeRootUri": "https://example.com/not-a-file-root"
                }
            },
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();

    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("invalid hint response");
    assert_eq!(response["id"], serde_json::json!(3));
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("active-root hint URI must use file:// scheme")
    );
    assert_eq!(
        response["error"]["data"]["atlas_repo_selection"]["failure_kind"],
        serde_json::json!("invalid_client_hint")
    );
    assert_eq!(
        response["error"]["data"]["atlas_repo_selection"]["selection_source"],
        serde_json::json!("client_hint")
    );
    let _ = session.finish().unwrap();
}

#[test]
fn request_hint_overrides_session_hint_and_session_hint_overrides_cached_root() {
    let repo_a = setup_graph_repo_fixture("src/alpha.rs", "compute", "src/alpha.rs::fn::compute");
    let repo_b = setup_graph_repo_fixture("src/beta.rs", "compute", "src/beta.rs::fn::compute");
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "roots": { "listChanged": true } },
                "clientInfo": { "name": "zed", "version": "1.0.0" },
                "_meta": {
                    "atlas": {
                        "preferredRootUri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string()
                    }
                }
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
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();
    let roots_request = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("roots/list request");
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": roots_request["id"].clone(),
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string(), "name": "a" },
                    { "uri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string(), "name": "b" }
                ]
            }
        }))
        .unwrap();
    let first = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("session-hinted response");
    let first_value: serde_json::Value = serde_json::from_str(
        first["result"]["content"][0]["text"]
            .as_str()
            .expect("first json payload"),
    )
    .expect("parse first payload");
    assert_eq!(first_value[0]["file"], serde_json::json!("src/alpha.rs"));

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "_meta": {
                "atlas": {
                    "activeRootUri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string()
                }
            },
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();
    let second = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("request-hinted response");
    let second_value: serde_json::Value = serde_json::from_str(
        second["result"]["content"][0]["text"]
            .as_str()
            .expect("second json payload"),
    )
    .expect("parse second payload");
    assert_eq!(second_value[0]["file"], serde_json::json!("src/beta.rs"));

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "query_graph",
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();
    let third = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("session-hinted after cached-root response");
    let third_value: serde_json::Value = serde_json::from_str(
        third["result"]["content"][0]["text"]
            .as_str()
            .expect("third json payload"),
    )
    .expect("parse third payload");
    assert_eq!(third_value[0]["file"], serde_json::json!("src/alpha.rs"));
    assert_eq!(
        third["result"]["_meta"]["atlas:repoSelection"]["selectionSource"],
        serde_json::json!("client_hint")
    );
    let _ = session.finish().unwrap();
}

#[test]
fn session_preferred_root_hint_revalidates_after_roots_list_changed() {
    let repo_a = setup_graph_repo_fixture("src/alpha.rs", "compute", "src/alpha.rs::fn::compute");
    let repo_b = setup_graph_repo_fixture("src/beta.rs", "compute", "src/beta.rs::fn::compute");
    let session = InteractiveStdioTestSession::start_dynamic(ServerOptions::default()).unwrap();

    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "roots": { "listChanged": true } },
                "clientInfo": { "name": "zed", "version": "1.0.0" },
                "_meta": {
                    "atlas": {
                        "preferredRootUri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string()
                    }
                }
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
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();
    let first_roots = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("first roots/list request");
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": first_roots["id"].clone(),
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_a._dir.path()).unwrap().to_string(), "name": "a" },
                    { "uri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string(), "name": "b" }
                ]
            }
        }))
        .unwrap();
    let _ = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("first preferred response");

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
                "arguments": { "text": "compute", "output_format": "json" }
            }
        }))
        .unwrap();
    let second_roots = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("second roots/list request");
    session
        .send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": second_roots["id"].clone(),
            "result": {
                "roots": [
                    { "uri": Url::from_directory_path(repo_b._dir.path()).unwrap().to_string(), "name": "b" }
                ]
            }
        }))
        .unwrap();
    let response = session
        .recv_json(Duration::from_secs(1))
        .unwrap()
        .expect("revalidated preferred hint error");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("does not match any advertised workspace root")
    );
    assert_eq!(
        response["error"]["data"]["atlas_repo_selection"]["failure_kind"],
        serde_json::json!("invalid_client_hint")
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
        "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"query\":\"compute\"}}}\n".to_owned(),
    ]
    .concat();
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
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
        initialize_result,
        &crate::spec::negotiate_initialize(Some(&serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "zed", "version": "1.0.0" }
        })))
        .expect("shared initialize result")
    );
    assert_eq!(
        initialize_result["serverInfo"]["description"],
        serde_json::json!(env!("CARGO_PKG_DESCRIPTION"))
    );

    assert_eq!(
        by_id[&serde_json::json!(2)]["result"],
        crate::tools::tool_list(),
        "stdio tools/list must serialize shared typed descriptor registry"
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

    assert_eq!(
        by_id[&serde_json::json!(5)]["result"]["_meta"]["atlas:outputFormat"],
        "json",
        "query_graph transport response must preserve JSON default"
    );
    let query_text = by_id[&serde_json::json!(5)]["result"]["content"][0]["text"]
        .as_str()
        .expect("query_graph text content");
    let query_value: serde_json::Value =
        serde_json::from_str(query_text).expect("query_graph payload json");
    assert_eq!(query_value[0]["qn"], "src/service.rs::fn::compute");

    assert_eq!(
        by_id[&serde_json::json!(6)]["result"]["_meta"]["atlas:outputFormat"],
        "toon",
        "get_context transport response must preserve TOON default"
    );
    let context_text = by_id[&serde_json::json!(6)]["result"]["content"][0]["text"]
        .as_str()
        .expect("get_context text content");
    assert!(context_text.contains("intent: symbol"));
    assert!(context_text.contains("src/service.rs::fn::compute"));
}

#[test]
fn stdio_transport_rejects_initialize_without_client_info() {
    let fixture = setup_fixture();
    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{}}}\n";
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["error"]["code"], serde_json::json!(-32602));
    assert_eq!(
        responses[0]["error"]["message"],
        serde_json::json!("initialize requires object params.clientInfo")
    );
    assert_eq!(
        responses[0]["error"]["data"]["atlas_error_code"],
        serde_json::json!("invalid_params")
    );
}

#[test]
fn stdio_transport_rejects_unsupported_initialize_protocol_version() {
    let fixture = setup_fixture();
    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"zed\",\"version\":\"1.0.0\"}}}\n";
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["error"]["code"], serde_json::json!(-32602));
    assert_eq!(
        responses[0]["error"]["message"],
        serde_json::json!(
            "unsupported protocol version '2024-11-05'; supported version: 2025-11-25"
        )
    );
}

#[test]
fn stdio_transport_reports_unknown_task_with_task_not_found_error() {
    let fixture = setup_fixture();
    let repo_dir = tempfile::tempdir().expect("tempdir");
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"zed\",\"version\":\"1.0.0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tasks/get\",\"params\":{\"taskId\":\"missing\"}}\n"
    );
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        repo_dir.path().to_str().expect("repo dir path"),
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .filter_map(|value| value.get("id").cloned().map(|id| (id, value)))
        .collect();
    assert_eq!(
        by_id[&serde_json::json!(2)]["error"]["code"],
        serde_json::json!(-32010)
    );
    assert_eq!(
        by_id[&serde_json::json!(2)]["error"]["data"]["atlas_error_code"],
        serde_json::json!("task_not_found")
    );
}

#[test]
fn stdio_transport_rejects_jsonrpc_batch_requests() {
    let fixture = setup_fixture();
    let input = "[]\n";
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["error"]["code"], serde_json::json!(-32600));
    assert_eq!(
        responses[0]["error"]["message"],
        serde_json::json!("JSON-RPC batch requests are not supported")
    );
}

#[test]
fn stdio_transport_returns_jsonrpc_errors_for_parse_and_method_failures() {
    let fixture = setup_fixture();
    let input = concat!(
        "not-json\n",
        "{\"id\":6,\"method\":\"initialize\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"missing/method\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"unknown_tool_xyz\",\"arguments\":{}}}\n"
    );
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
    assert_eq!(responses.len(), 4);
    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .map(|response| (response["id"].clone(), response))
        .collect();

    assert_eq!(by_id[&serde_json::Value::Null]["error"]["code"], -32700);
    assert_eq!(
        by_id[&serde_json::Value::Null]["error"]["data"]["atlas_error_code"],
        serde_json::json!("parse_error")
    );
    assert_error_code_doc_link(
        &by_id[&serde_json::Value::Null]["error"]["data"]["atlas_error_code_docs"],
        "parse_error",
    );
    assert_eq!(by_id[&serde_json::json!(6)]["id"], 6);
    assert_eq!(by_id[&serde_json::json!(6)]["error"]["code"], -32600);
    assert_eq!(
        by_id[&serde_json::json!(6)]["error"]["data"]["atlas_error_code"],
        serde_json::json!("invalid_request")
    );
    assert_error_code_doc_link(
        &by_id[&serde_json::json!(6)]["error"]["data"]["atlas_error_code_docs"],
        "invalid_request",
    );
    assert_eq!(by_id[&serde_json::json!(8)]["id"], 8);
    assert_eq!(by_id[&serde_json::json!(8)]["error"]["code"], -32601);
    assert_eq!(
        by_id[&serde_json::json!(8)]["error"]["data"]["atlas_error_code"],
        serde_json::json!("method_not_found")
    );
    assert_error_code_doc_link(
        &by_id[&serde_json::json!(8)]["error"]["data"]["atlas_error_code_docs"],
        "method_not_found",
    );
    assert_eq!(by_id[&serde_json::json!(7)]["id"], 7);
    assert_eq!(by_id[&serde_json::json!(7)]["error"]["code"], -32601);
    assert_eq!(
        by_id[&serde_json::json!(7)]["error"]["data"]["atlas_error_code"],
        serde_json::json!("method_not_found")
    );
    assert_error_code_doc_link(
        &by_id[&serde_json::json!(7)]["error"]["data"]["atlas_error_code_docs"],
        "method_not_found",
    );
    assert!(
        by_id[&serde_json::json!(7)]["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("method not found")
    );
}

#[test]
fn stdio_transport_tool_argument_errors_return_is_error_tool_results() {
    let fixture = setup_fixture();
    let input = [
        initialize_request_line(),
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n".to_owned(),
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"output_format\":\"bogus\"}}}\n".to_owned(),
    ]
    .concat();
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
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
    assert_eq!(
        result["structuredContent"]["retry_guidance"],
        serde_json::json!("Use supported output_format value 'toon' or 'json', then retry.")
    );
    assert!(
        result.get("Text").is_none(),
        "legacy Text wrapper must not appear"
    );
    assert_eq!(
        result["_meta"]["atlas:outputFormat"],
        serde_json::json!("toon")
    );
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let response = parse_output_lines(writer)
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let response = parse_output_lines(writer)
        .into_iter()
        .find(|value| value["id"] == serde_json::json!(2))
        .expect("unknown tool response");
    assert!(
        response.get("result").is_none(),
        "unknown tool must not be normalized into result.isError"
    );
    assert_eq!(response["error"]["code"], serde_json::json!(-32601));
    assert_eq!(
        response["error"]["data"]["atlas_error_code"],
        serde_json::json!("method_not_found")
    );
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let response = parse_output_lines(writer)
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .filter(|value| value["id"].is_number())
        .map(|response| (response["id"].clone(), response))
        .collect();
    assert_eq!(
        by_id[&serde_json::json!(2)]["error"]["code"],
        serde_json::json!(-32602)
    );
    assert_eq!(
        by_id[&serde_json::json!(3)]["error"]["code"],
        serde_json::json!(-32602)
    );
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
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
    assert_eq!(
        result["atlas_readiness"]["execution_state"].as_str(),
        Some("corrupt"),
        "execution_state must be corrupt for a dropped-table db"
    );
    let reason = result["atlas_readiness"]["reason"]
        .as_str()
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
fn stdio_transport_exposes_resources_completion_and_logging_methods() {
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .filter(|value| value.get("id").is_some())
        .map(|response| (response["id"].clone(), response))
        .collect();
    assert_eq!(
        by_id[&serde_json::json!(2)]["result"]["resources"][0]["uri"],
        serde_json::json!("atlas://graph/provenance")
    );
    assert!(
        by_id[&serde_json::json!(3)]["result"]["resourceTemplates"]
            .as_array()
            .expect("templates")
            .len()
            >= 2
    );
    assert_eq!(
        by_id[&serde_json::json!(4)]["result"]["completion"]["values"][0]["value"],
        serde_json::json!("json")
    );
    assert_eq!(
        by_id[&serde_json::json!(5)]["result"]["level"],
        serde_json::json!("warning")
    );
}

#[test]
fn stdio_transport_emits_progress_and_log_notifications() {
    let fixture = setup_fixture();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$/setTrace\",\"params\":{\"value\":\"messages\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"progressToken\":\"tok-1\",\"arguments\":{\"text\":\"compute\"}}}\n"
    );
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
    assert!(
        responses
            .iter()
            .any(|value| value["method"] == serde_json::json!("$/logMessage")),
        "setTrace=messages should emit log notifications"
    );
    assert!(
        responses.iter().any(|value| {
            value["method"] == serde_json::json!("$/progress")
                && value["params"]["token"] == serde_json::json!("tok-1")
                && value["params"]["value"]["kind"] == serde_json::json!("begin")
        }),
        "tool call should emit progress begin"
    );
    assert!(
        responses.iter().any(|value| {
            value["method"] == serde_json::json!("$/progress")
                && value["params"]["token"] == serde_json::json!("tok-1")
                && value["params"]["value"]["kind"] == serde_json::json!("end")
        }),
        "tool call should emit progress end"
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions {
            // Set worker thread count and timeout aggressively so
            // cancellation can be observed without waiting for real I/O.
            worker_threads: 1,
            tool_timeout_ms: 10_000,
            tool_timeout_ms_by_tool: HashMap::new(),
            #[cfg(feature = "http-transport")]
            http_auth: None,
        },
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
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
    let reader = BufReader::new(Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();

    run_server_io(
        reader,
        &mut writer,
        "/ignored",
        &fixture.db_path,
        ServerOptions::default(),
    )
    .expect("run server io");

    let responses = parse_output_lines(writer);
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
    assert_error_code_doc_link(
        &response["error"]["data"]["atlas_error_code_docs"],
        "method_not_found",
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
