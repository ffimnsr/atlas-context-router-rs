//! tools/call behavior tests: handrolled parity, error mapping, durable
//! task persistence, purge confirmation, and request-context propagation.

use super::*;
use crate::mrtr::RequestStateBinding;
use rmcp::model::ErrorCode;
use std::collections::BTreeMap;

#[test]
fn call_tool_query_graph_matches_handrolled_structured_content() {
    let fixture = ToolFixture::new();
    let args = json!({"text": "greet", "output_format": "json"});
    assert_call_tool_structured_content_matches_handrolled(&fixture, "query_graph", args);
}

#[test]
fn call_tool_status_matches_handrolled_structured_content() {
    let fixture = ToolFixture::new();
    let args = json!({"output_format": "json"});
    assert_call_tool_structured_content_matches_handrolled(&fixture, "status", args);
}

#[test]
fn call_tool_get_context_matches_handrolled_structured_content() {
    let fixture = ToolFixture::new();
    let args = json!({
        "target": {"kind": "query", "query": "greet"},
        "output_format": "json"
    });
    assert_call_tool_structured_content_matches_handrolled(&fixture, "get_context", args);
}

#[test]
fn call_tool_search_files_matches_handrolled_structured_content() {
    let fixture = ToolFixture::new();
    let args = json!({"pattern": "*.rs", "output_format": "json"});
    assert_call_tool_structured_content_matches_handrolled(&fixture, "search_files", args);
}

#[test]
fn call_tool_invalid_input_returns_user_visible_error_result() {
    let fixture = ToolFixture::new();
    let args = json!({"query": "(", "is_regex": true, "output_format": "json"});

    let handrolled = handrolled_tools_call(&fixture, "search_content", &args).expect("handrolled");
    let rmcp = fixture
        .server
        .call_tool_for_tests(call_tool_request("search_content", Some(args.clone())))
        .expect("rmcp call");
    let rmcp_complete = expect_complete(rmcp);

    assert_eq!(rmcp_complete.is_error, Some(true));
    assert_eq!(
        rmcp_complete.structured_content,
        handrolled.get("structuredContent").cloned()
    );
    assert_eq!(
        rmcp_complete
            .content
            .first()
            .and_then(|item| item.as_text())
            .map(|text| text.text.as_str()),
        handrolled
            .pointer("/content/0/text")
            .and_then(Value::as_str)
    );
}

#[test]
fn call_tool_unknown_tool_returns_protocol_error() {
    let fixture = ToolFixture::new();
    let error = fixture
        .server
        .call_tool_for_tests(call_tool_request("missing_tool", Some(json!({}))))
        .expect_err("unknown tool must fail");
    assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(error.message.as_ref(), "unknown tool: missing_tool");
    assert_eq!(
        error
            .data
            .as_ref()
            .and_then(|data| data.get("atlas_error_code")),
        Some(&json!("method_not_found"))
    );
}

#[test]
fn call_tool_query_graph_fails_closed_on_sqlite_corrupt_graph_db() {
    let fixture = ToolFixture::new();
    overwrite_graph_db_with_garbage(&fixture);

    let rmcp = fixture
        .server
        .call_tool_for_tests(call_tool_request(
            "query_graph",
            Some(json!({"text": "greet", "output_format": "json"})),
        ))
        .expect("rmcp query_graph blocked response");
    let rmcp_complete = expect_complete(rmcp);
    let rmcp_body = rmcp_complete
        .structured_content
        .clone()
        .expect("rmcp structured content");

    assert_eq!(rmcp_complete.is_error, Some(true));
    assert_eq!(rmcp_body["ok"], json!(false));
    assert_eq!(rmcp_body["blocked"], json!(true));
    assert_eq!(rmcp_body["error_code"], json!("sqlite_corrupt"));
    assert_eq!(rmcp_body["health_class"], json!("sqlite_corrupt"));
    assert_eq!(rmcp_body["execution_state"], json!("corrupt"));
    assert_eq!(
        rmcp_body["recommended_rebuild_command"],
        json!("atlas build")
    );
    assert_eq!(rmcp_body["quarantine_path"], json!(null));
    assert_eq!(rmcp_body["atlas_readiness"]["blocked"], json!(true));
    assert_eq!(rmcp_body["atlas_freshness"]["blocked"], json!(true));

    let handrolled = handrolled_tools_call(
        &fixture,
        "query_graph",
        &json!({"text": "greet", "output_format": "json"}),
    )
    .expect("handrolled query_graph blocked response");
    assert_eq!(
        rmcp_body, handrolled["structuredContent"],
        "rmcp and handrolled blocked payload must match"
    );
}

#[test]
fn call_tool_get_context_fails_closed_on_logical_inconsistency() {
    let fixture = ToolFixture::new();
    seed_dangling_graph_edge(&fixture);

    let rmcp = fixture
        .server
        .call_tool_for_tests(call_tool_request(
            "get_context",
            Some(json!({
                "target": {"kind": "query", "query": "greet"},
                "output_format": "json"
            })),
        ))
        .expect("rmcp get_context blocked response");
    let rmcp_complete = expect_complete(rmcp);
    let rmcp_body = rmcp_complete
        .structured_content
        .clone()
        .expect("rmcp structured content");

    assert_eq!(rmcp_complete.is_error, Some(true));
    assert_eq!(rmcp_body["ok"], json!(false));
    assert_eq!(rmcp_body["blocked"], json!(true));
    assert_eq!(rmcp_body["error_code"], json!("logical_inconsistency"));
    assert_eq!(rmcp_body["health_class"], json!("logical_inconsistency"));
    assert_eq!(rmcp_body["execution_state"], json!("corrupt"));
    assert_eq!(
        rmcp_body["recommended_rebuild_command"],
        json!("atlas build")
    );
    assert_eq!(
        rmcp_body["atlas_readiness"]["error_code"],
        json!("logical_inconsistency")
    );
}

#[test]
fn call_tool_query_graph_increments_session_event_count() {
    let rmcp_fixture = ToolFixture::new();
    let rmcp_before = session_event_count(&rmcp_fixture.repo_root, &rmcp_fixture.db_path);
    rmcp_fixture
        .server
        .call_tool_for_tests(call_tool_request(
            "query_graph",
            Some(json!({"text": "greet", "output_format": "json"})),
        ))
        .expect("rmcp query_graph");
    let rmcp_after = session_event_count(&rmcp_fixture.repo_root, &rmcp_fixture.db_path);

    let handrolled_fixture = ToolFixture::new();
    let handrolled_before =
        session_event_count(&handrolled_fixture.repo_root, &handrolled_fixture.db_path);
    handrolled_tools_call(
        &handrolled_fixture,
        "query_graph",
        &json!({"text": "greet", "output_format": "json"}),
    )
    .expect("handrolled query_graph");
    let handrolled_after =
        session_event_count(&handrolled_fixture.repo_root, &handrolled_fixture.db_path);

    assert_eq!(
        rmcp_after - rmcp_before,
        handrolled_after - handrolled_before,
        "rmcp delta={}, handrolled delta={}",
        rmcp_after - rmcp_before,
        handrolled_after - handrolled_before,
    );
}

#[test]
fn explicit_task_persists_input_required_payload_for_rmcp_tasks_get() {
    let fixture = ToolFixture::new();
    let response = fixture
        .server
        .call_tool_for_tests_with_context(
            call_tool_request(
                "purge_saved_context",
                Some(json!({"keep_days": 30, "output_format": "json", "task": {"ttl": 1000}})),
            ),
            super::AtlasRmcpCallContext {
                request_id: "req-task-input".to_owned(),
                client_capabilities: Some(json!({
                    "elicitation": {"form": {}},
                    "extensions": {"io.modelcontextprotocol/tasks": {}}
                })),
                authenticated_principal: Some("user@example.com".to_owned()),
                progress: None,
            },
        )
        .expect("rmcp explicit task");
    let CallToolResponse::Task(task_result) = response else {
        panic!("expected task result");
    };
    let task_id = task_result.task.task_id.clone();

    for _ in 0..50 {
        let task = SessionStore::open_in_repo(&fixture.repo_root)
            .expect("open session store")
            .get_durable_task(&task_id)
            .expect("reload task")
            .expect("task exists");
        if task.status == DurableTaskStatus::InputRequired {
            assert!(task.request_state.as_deref().is_some());
            assert!(
                task.input_requests
                    .as_ref()
                    .and_then(|value| value.get("confirmation"))
                    .is_some()
            );
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for input_required durable task state");
}

#[test]
fn explicit_task_without_client_task_capability_fails_before_persistence() {
    let fixture = ToolFixture::new();
    let error = fixture
        .server
        .call_tool_for_tests_with_context(
            call_tool_request(
                "purge_saved_context",
                Some(json!({"keep_days": 30, "output_format": "json", "task": {"ttl": 1000}})),
            ),
            super::AtlasRmcpCallContext {
                request_id: "req-task-unsupported".to_owned(),
                client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                authenticated_principal: Some("user@example.com".to_owned()),
                progress: None,
            },
        )
        .expect_err("explicit task without Tasks capability must fail");

    assert_eq!(error.code, ErrorCode::MISSING_REQUIRED_CLIENT_CAPABILITY);
    assert_eq!(
        error
            .data
            .as_ref()
            .and_then(|data| data
                .pointer("/requiredCapabilities/extensions/io.modelcontextprotocol~1tasks")),
        Some(&json!({}))
    );
    assert!(
        SessionStore::open_in_repo(&fixture.repo_root)
            .expect("open session store")
            .list_durable_tasks(None, 10)
            .expect("list durable tasks")
            .tasks
            .is_empty()
    );
}

#[test]
fn purge_confirmation_accept_retry_completes_purge() {
    let fixture = ToolFixture::new();
    let first = fixture
        .server
        .call_tool_for_tests_with_context(
            call_tool_request(
                "purge_saved_context",
                Some(json!({"keep_days": 30, "output_format": "json"})),
            ),
            super::AtlasRmcpCallContext {
                request_id: "req-accept-1".to_owned(),
                client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                authenticated_principal: Some("user@example.com".to_owned()),
                progress: None,
            },
        )
        .expect("first purge call");
    let CallToolResponse::InputRequired(first) = first else {
        panic!("expected input_required first response");
    };
    let request_state = first.request_state.clone().expect("requestState");

    let second = fixture
        .server
        .call_tool_for_tests_with_context(
            call_tool_request(
                "purge_saved_context",
                Some(json!({"keep_days": 30, "output_format": "json"})),
            )
            .with_request_state(request_state)
            .with_input_responses(BTreeMap::from([(
                String::from("confirmation"),
                json!({
                    "action": "accept",
                    "content": {"confirmation": "confirm"}
                }),
            )])),
            super::AtlasRmcpCallContext {
                request_id: "req-accept-2".to_owned(),
                client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                authenticated_principal: Some("user@example.com".to_owned()),
                progress: None,
            },
        )
        .expect("accepted retry");
    let complete = expect_complete(second);
    assert_ne!(complete.is_error, Some(true));
}

#[test]
fn purge_confirmation_decline_retry_returns_user_visible_error() {
    let fixture = ToolFixture::new();
    let first = fixture
        .server
        .call_tool_for_tests_with_context(
            call_tool_request(
                "purge_saved_context",
                Some(json!({"keep_days": 30, "output_format": "json"})),
            ),
            super::AtlasRmcpCallContext {
                request_id: "req-decline-1".to_owned(),
                client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                authenticated_principal: Some("user@example.com".to_owned()),
                progress: None,
            },
        )
        .expect("first purge call");
    let CallToolResponse::InputRequired(first) = first else {
        panic!("expected input_required first response");
    };
    let request_state = first.request_state.clone().expect("requestState");

    let second = fixture
        .server
        .call_tool_for_tests_with_context(
            call_tool_request(
                "purge_saved_context",
                Some(json!({"keep_days": 30, "output_format": "json"})),
            )
            .with_request_state(request_state)
            .with_input_responses(BTreeMap::from([(
                String::from("confirmation"),
                json!({
                    "action": "decline"
                }),
            )])),
            super::AtlasRmcpCallContext {
                request_id: "req-decline-2".to_owned(),
                client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                authenticated_principal: Some("user@example.com".to_owned()),
                progress: None,
            },
        )
        .expect("declined retry");
    let complete = expect_complete(second);
    assert_eq!(complete.is_error, Some(true));
    assert!(
        complete
            .structured_content
            .as_ref()
            .and_then(|value| value.get("message"))
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("cancelled by client"))
    );
}

#[test]
fn tool_execution_sees_client_capabilities_and_authenticated_principal() {
    let fixture = ToolFixture::new();
    let response = fixture
        .server
        .call_tool_for_tests_with_context(
            call_tool_request(
                "purge_saved_context",
                Some(json!({"keep_days": 30, "output_format": "json"})),
            ),
            super::AtlasRmcpCallContext {
                request_id: "req-1".to_owned(),
                client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                authenticated_principal: Some("user@example.com".to_owned()),
                progress: None,
            },
        )
        .expect("rmcp purge_saved_context");
    let CallToolResponse::InputRequired(result) = response else {
        panic!("expected input_required response");
    };
    let request_state = result.request_state.as_deref().expect("requestState");
    assert!(
        result
            .input_requests
            .as_ref()
            .is_some_and(|requests| !requests.is_empty())
    );
    crate::mrtr::validate_request_state(
        request_state,
        RequestStateBinding {
            method: "tools/call",
            tool: "purge_saved_context",
            arguments: Some(&json!({"keep_days": 30, "output_format": "json"})),
            principal: Some("user@example.com"),
        },
    )
    .expect("requestState binds authenticated principal");
}
