//! Shared fixtures and helper assertions for the rmcp-server unit tests.

use super::protocol::{
    BootstrapNotificationPlan, logging_message_notification_param, tool_call_log_level,
};
use super::{AtlasRmcpCallContext, AtlasRmcpServer};
use crate::completion;
use crate::output::OutputFormat;
use crate::resources;
use crate::session_tools;
use crate::transport::ServerOptions;
use crate::transport::repo_selection::strip_repo_selector_fields;
use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};
use atlas_session::{DurableTaskStatus, DurableTaskUpdate, NewDurableTask, SessionStore};
use atlas_store_sqlite::Store;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CompleteRequestParams, ReadResourceResponse,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

mod call_tool;
mod protocol;
mod registry;
mod repo_selection;

fn server() -> AtlasRmcpServer {
    AtlasRmcpServer::new(
        "/tmp/repo",
        "/tmp/repo/.atlas/index.db",
        ServerOptions::default(),
    )
}

fn json_array_strings(array: &Value, key: &str) -> Vec<String> {
    array
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item[key].as_str().expect("string").to_owned())
        .collect()
}

fn call_tool_request(name: &str, args: Option<Value>) -> CallToolRequestParams {
    let mut request = CallToolRequestParams::new(name.to_owned());
    if let Some(args) = args {
        request = request.with_arguments(
            serde_json::from_value(args).expect("arguments object for CallToolRequestParams"),
        );
    }
    request
}

fn expect_complete(response: CallToolResponse) -> rmcp::model::CallToolResult {
    match response {
        CallToolResponse::Complete(result) => result,
        other => panic!("expected complete response, got {other:?}"),
    }
}

fn expect_read_resource_complete(
    response: ReadResourceResponse,
) -> rmcp::model::ReadResourceResult {
    match response {
        ReadResourceResponse::Complete(result) => result,
        other => panic!("expected complete resource response, got {other:?}"),
    }
}

fn assert_call_tool_structured_content_matches_handrolled(
    fixture: &ToolFixture,
    tool_name: &str,
    args: Value,
) {
    let handrolled =
        handrolled_tools_call(fixture, tool_name, &args).expect("handrolled tools/call response");
    let rmcp = fixture
        .server
        .call_tool_for_tests(call_tool_request(tool_name, Some(args)))
        .expect("rmcp tools/call response");
    let rmcp_complete = expect_complete(rmcp);

    assert_eq!(
        rmcp_complete.structured_content,
        handrolled.get("structuredContent").cloned(),
        "structured content mismatch for {tool_name}"
    );
}

fn assert_read_resource_matches_handrolled(fixture: &ToolFixture, uri: &str) {
    let handrolled = resources::resources_read(
        Some(&json!({ "uri": uri })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("handrolled resource read");
    let rmcp = fixture
        .server
        .read_resource_result(uri)
        .expect("rmcp resource read");
    let rmcp_complete = expect_read_resource_complete(ReadResourceResponse::Complete(rmcp.clone()));

    assert_eq!(
        serde_json::to_value(&rmcp_complete).expect("serialize rmcp resource"),
        handrolled
    );
}

fn assert_completion_matches_handrolled(fixture: &ToolFixture, request: CompleteRequestParams) {
    let legacy_request =
        super::legacy_completion_request_value(&request).expect("legacy completion request");
    let handrolled =
        completion::complete(Some(&legacy_request), &fixture.repo_root, &fixture.db_path)
            .expect("handrolled completion");
    let rmcp = fixture
        .server
        .complete_result(request)
        .expect("rmcp completion");
    let handrolled_values = handrolled["completion"]["values"]
        .as_array()
        .expect("handrolled values")
        .iter()
        .map(|item| item["value"].as_str().expect("value").to_owned())
        .collect::<Vec<_>>();

    assert_eq!(rmcp.completion.values, handrolled_values);
    assert_eq!(
        rmcp.completion.total,
        handrolled["completion"]["total"]
            .as_u64()
            .map(|value| value as u32)
    );
    assert_eq!(
        rmcp.completion.has_more,
        handrolled["completion"]["hasMore"].as_bool()
    );
}

fn handrolled_tools_call(
    fixture: &ToolFixture,
    tool_name: &str,
    args: &Value,
) -> anyhow::Result<Value> {
    let request_params = json!({
        "name": tool_name,
        "arguments": args,
    });
    let stripped_args = Some(strip_repo_selector_fields(args.clone()));
    let runtime_context = crate::runtime_context::RequestContext::new(
        std::sync::Arc::new(|_| Ok(())),
        crate::runtime_context::ClientInteractionCapabilities::default(),
        "stdio",
        None,
        None,
        "1",
        "tools/call",
        Some(request_params.clone()),
    );
    crate::runtime_context::install(runtime_context);
    crate::tasks::install_tool_call_request_params(Some(&request_params));
    let result = crate::tasks::execute_tool_call(
        tool_name,
        stripped_args,
        &fixture.repo_root,
        &fixture.db_path,
    );
    crate::tasks::uninstall_tool_call_request_params();
    crate::runtime_context::uninstall();
    result
}

fn session_event_count(repo_root: &str, db_path: &str) -> i64 {
    tool_body(
        &session_tools::tool_get_session_status(None, repo_root, db_path, OutputFormat::Json)
            .expect("session status"),
    )["event_count"]
        .as_i64()
        .expect("event_count")
}

fn tool_body(result: &Value) -> Value {
    result
        .get("structuredContent")
        .cloned()
        .or_else(|| {
            result
                .get("content")
                .and_then(|content| content.get(0))
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str(text).ok())
        })
        .expect("tool body")
}

struct ToolFixture {
    _dir: TempDir,
    repo_root: String,
    db_path: String,
    server: AtlasRmcpServer,
}

struct RepoSelectionFixture {
    _dir: TempDir,
    db_path: String,
}

impl ToolFixture {
    fn new() -> Self {
        let (dir, _db_path_path, db_path) = setup_repo();
        let repo_root = dir.path().to_string_lossy().into_owned();
        crate::tools::call(
            "build_or_update_graph",
            Some(&json!({"operation": {"kind": "build"}, "output_format": "json"})),
            &repo_root,
            &db_path,
        )
        .expect("build graph");
        seed_schema_graph(&db_path);
        Self {
            server: AtlasRmcpServer::new(&repo_root, &db_path, ServerOptions::default()),
            _dir: dir,
            repo_root,
            db_path,
        }
    }
}

fn seed_durable_task(
    fixture: &ToolFixture,
    task_id: &str,
    status: DurableTaskStatus,
    result: Option<Value>,
    error: Option<Value>,
    input_requests: Option<Value>,
    request_state: Option<&str>,
) {
    let mut store = SessionStore::open_in_repo(&fixture.repo_root).expect("open session store");
    store
        .create_durable_task(&NewDurableTask {
            task_id: task_id.to_owned(),
            originating_method: "tools/call".to_owned(),
            request_id: Some("request-1".to_owned()),
            tool_name: Some("doctor".to_owned()),
            transport_kind: Some("rmcp".to_owned()),
            session_id: None,
            status: DurableTaskStatus::Working,
            status_message: Some("working".to_owned()),
            ttl_ms: Some(5_000),
        })
        .expect("create durable task");
    store
        .update_durable_task(
            task_id,
            &DurableTaskUpdate {
                status: Some(status),
                status_message: Some(status.as_str().to_owned()),
                result,
                error,
                input_requests,
                request_state: request_state.map(str::to_owned),
                ..Default::default()
            },
        )
        .expect("update durable task");
}

fn setup_graph_repo_fixture(
    primary_file: &str,
    primary_name: &str,
    primary_qn: &str,
) -> RepoSelectionFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join(".git")).expect("create git dir");
    let db_path = dir.path().join(".atlas").join("worldtree.db");
    fs::create_dir_all(db_path.parent().expect("atlas dir")).expect("create atlas dir");
    if let Some(parent) = Path::new(primary_file).parent() {
        fs::create_dir_all(dir.path().join(parent)).expect("create primary parent dir");
    }
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");
    let primary = make_node(
        atlas_core::NodeKind::Function,
        primary_name,
        primary_qn,
        primary_file,
    );
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

    RepoSelectionFixture { _dir: dir, db_path }
}

fn setup_repo() -> (TempDir, PathBuf, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(
        src_dir.join("lib.rs"),
        "pub mod service;\npub fn greet() -> &'static str { \"hi\" }\n",
    )
    .expect("write fixture source");
    fs::write(
        src_dir.join("service.rs"),
        "pub fn compute() -> i32 { 1 }\n",
    )
    .expect("write fixture service source");
    fs::write(
        src_dir.join("api.rs"),
        "pub fn handle_request() -> i32 { crate::service::compute() }\n",
    )
    .expect("write fixture api source");
    let tests_dir = dir.path().join("tests");
    fs::create_dir_all(&tests_dir).expect("create tests dir");
    fs::write(
        tests_dir.join("service_test.rs"),
        "#[test]\nfn compute_test() { assert_eq!(crate::service::compute(), 1); }\n",
    )
    .expect("write fixture test source");
    fs::write(
        dir.path().join("README.md"),
        "# Fixture Repo\n\n## Status\n\nFixture status content.\n",
    )
    .expect("write fixture readme");
    fs::create_dir_all(dir.path().join("config")).expect("create config dir");
    fs::write(dir.path().join("config/app.toml"), "name = \"fixture\"\n")
        .expect("write fixture config");
    fs::create_dir_all(dir.path().join("templates")).expect("create templates dir");
    fs::write(
        dir.path().join("templates/index.html"),
        "<html><body>{{ greet }}</body></html>\n",
    )
    .expect("write fixture template");
    fs::create_dir_all(dir.path().join("queries")).expect("create queries dir");
    fs::write(dir.path().join("queries/example.sql"), "select 1;\n").expect("write fixture sql");
    git(dir.path(), &["init", "--quiet"]);
    git(dir.path(), &["config", "user.name", "Atlas Tests"]);
    git(
        dir.path(),
        &["config", "user.email", "atlas-tests@example.com"],
    );
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "--quiet", "-m", "fixture baseline"]);
    let db_path = dir.path().join(".atlas").join("worldtree.db");
    (dir, db_path.clone(), db_path.to_string_lossy().into_owned())
}

fn git(repo: &Path, args: &[&str]) {
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

fn make_node(kind: NodeKind, name: &str, qn: &str, file: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_owned(),
        qualified_name: qn.to_owned(),
        file_path: file.to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: None,
        is_test: kind == NodeKind::Test,
        file_hash: format!("hash:{file}"),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    }
}

fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file.to_owned(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: None,
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    }
}

fn seed_schema_graph(db_path: &str) {
    let mut store = Store::open(db_path).expect("open store");

    let greet = make_node(
        NodeKind::Function,
        "greet",
        "src/lib.rs::fn::greet",
        "src/lib.rs",
    );
    store
        .replace_file_graph(
            "src/lib.rs",
            "hash:src/lib.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&greet),
            &[],
        )
        .expect("seed lib graph");

    let compute = make_node(
        NodeKind::Function,
        "compute",
        "src/service.rs::fn::compute",
        "src/service.rs",
    );
    store
        .replace_file_graph(
            "src/service.rs",
            "hash:src/service.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute),
            &[],
        )
        .expect("seed service graph");

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
        .expect("seed api graph");

    let compute_test = make_node(
        NodeKind::Test,
        "compute_test",
        "tests/service_test.rs::fn::compute_test",
        "tests/service_test.rs",
    );
    let test_targets_compute = make_edge(
        EdgeKind::Tests,
        "tests/service_test.rs::fn::compute_test",
        "src/service.rs::fn::compute",
        "tests/service_test.rs",
    );
    store
        .replace_file_graph(
            "tests/service_test.rs",
            "hash:tests/service_test.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute_test),
            &[test_targets_compute],
        )
        .expect("seed test graph");
}
