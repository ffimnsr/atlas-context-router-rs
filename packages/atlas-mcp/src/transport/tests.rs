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

mod dispatch;
mod protocol;
mod repo_selection;
mod reverse_request;
mod stdio;
mod trace;
