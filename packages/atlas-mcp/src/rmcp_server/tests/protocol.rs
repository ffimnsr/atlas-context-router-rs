//! Protocol-behavior tests: info, versions, tasks, ping, initialization,
//! levels, trace, logging, custom requests, and dynamic-root refresh.

use super::*;
use crate::spec;
use crate::transport::TraceLevel;
use rmcp::ServerHandler;
use rmcp::model::{
    CancelTaskParams, ErrorCode, GetTaskParams, LoggingLevel, SetLevelRequestParams,
    UpdateTaskParams,
};
use std::collections::{BTreeMap, HashMap};

#[test]
fn constructor_preserves_repo_and_db_paths_exactly() {
    let options = ServerOptions {
        worker_threads: 7,
        tool_timeout_ms: 42_000,
        tool_timeout_ms_by_tool: HashMap::from([("query_graph".to_owned(), 9_001)]),
        #[cfg(feature = "http-transport")]
        http_auth: None,
    };
    let server = AtlasRmcpServer::new(
        "./relative/../repo-root",
        "./relative/../repo-root/.atlas/graph.db",
        options.clone(),
    );
    assert_eq!(server.repo_root(), "./relative/../repo-root");
    assert_eq!(server.db_path(), "./relative/../repo-root/.atlas/graph.db");
    assert_eq!(server.options().worker_threads, options.worker_threads);
    assert_eq!(server.options().tool_timeout_ms, options.tool_timeout_ms);
    assert_eq!(
        server.options().tool_timeout_ms_by_tool,
        options.tool_timeout_ms_by_tool
    );
}

#[test]
fn supported_protocol_versions_only_exposes_current_version() {
    let versions = server().supported_protocol_versions();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].as_str(), spec::MCP_PROTOCOL_VERSION);
}

#[test]
fn get_task_returns_official_completed_detailed_task() {
    let fixture = ToolFixture::new();
    seed_durable_task(
        &fixture,
        "task-completed",
        DurableTaskStatus::Completed,
        Some(json!({
            "resultType": "complete",
            "content": [{"type": "text", "text": "done"}],
            "structuredContent": {"ok": true}
        })),
        None,
        None,
        None,
    );

    let result = fixture
        .server
        .get_task_result(GetTaskParams::new("task-completed"))
        .expect("rmcp get_task");

    assert_eq!(result.task.task.task_id, "task-completed");
    assert_eq!(result.task.task.status, rmcp::model::TaskStatus::Completed);
    match result.task.payload {
        rmcp::model::TaskPayload::Completed { result } => {
            assert_eq!(result.get("resultType"), Some(&json!("complete")));
            assert_eq!(result.get("structuredContent"), Some(&json!({"ok": true})));
        }
        other => panic!("expected completed payload, got {other:?}"),
    }
}

#[test]
fn get_task_returns_official_input_required_detailed_task() {
    let fixture = ToolFixture::new();
    seed_durable_task(
        &fixture,
        "task-input-required",
        DurableTaskStatus::InputRequired,
        None,
        None,
        Some(json!({
            "confirmation": {
                "method": "elicitation/create",
                "params": {
                    "message": "Confirm destructive action",
                    "requestedSchema": {
                        "type": "object",
                        "properties": {
                            "confirmation": {"type": "string"}
                        },
                        "required": ["confirmation"]
                    }
                }
            }
        })),
        Some("sealed-request-state"),
    );
    let mut store = SessionStore::open_in_repo(&fixture.repo_root).expect("open session store");
    store
        .update_durable_task(
            "task-input-required",
            &DurableTaskUpdate {
                progress: Some(json!({"message": "awaiting confirmation", "percentage": 10})),
                ..Default::default()
            },
        )
        .expect("seed progress");

    let result = fixture
        .server
        .get_task_result(GetTaskParams::new("task-input-required"))
        .expect("rmcp get_task input_required");

    assert_eq!(result.task.task.task_id, "task-input-required");
    assert_eq!(
        result.task.task.status,
        rmcp::model::TaskStatus::InputRequired
    );
    assert_eq!(
        result
            .meta
            .as_ref()
            .and_then(|meta| meta.0.get(crate::rmcp_types::ATLAS_TASK_META_PROGRESS)),
        Some(&json!({"message": "awaiting confirmation", "percentage": 10}))
    );
    assert_eq!(
        result
            .meta
            .as_ref()
            .and_then(|meta| meta.0.get(crate::rmcp_types::ATLAS_TASK_META_REQUEST_STATE)),
        Some(&json!("sealed-request-state"))
    );
    match result.task.payload {
        rmcp::model::TaskPayload::InputRequired { input_requests } => {
            assert!(input_requests.contains_key("confirmation"));
        }
        other => panic!("expected input_required payload, got {other:?}"),
    }
}

#[test]
fn update_task_accepts_rmcp_input_responses() {
    let fixture = ToolFixture::new();
    seed_durable_task(
        &fixture,
        "task-input",
        DurableTaskStatus::InputRequired,
        None,
        None,
        Some(json!({
            "confirmation": {
                "method": "elicitation/create",
                "params": {
                    "message": "Confirm destructive action",
                    "requestedSchema": {
                        "type": "object",
                        "properties": {
                            "confirmation": {"type": "string"}
                        },
                        "required": ["confirmation"]
                    }
                }
            }
        })),
        Some("sealed-request-state"),
    );

    fixture
        .server
        .update_task_result(UpdateTaskParams::new(
            "task-input",
            BTreeMap::from([(String::from("confirmation"), json!({"action": "accept"}))]),
        ))
        .expect("rmcp update_task");

    let updated = SessionStore::open_in_repo(&fixture.repo_root)
        .expect("open session store")
        .get_durable_task("task-input")
        .expect("reload task")
        .expect("task exists");
    assert_eq!(updated.status, DurableTaskStatus::Working);
    assert_eq!(
        updated.progress,
        Some(json!({"clientInput": {"confirmation": {"action": "accept"}}}))
    );
}

#[test]
fn cancel_task_marks_durable_task_cancelled() {
    let fixture = ToolFixture::new();
    seed_durable_task(
        &fixture,
        "task-working",
        DurableTaskStatus::Working,
        None,
        None,
        None,
        None,
    );

    fixture
        .server
        .cancel_task_result(CancelTaskParams::new("task-working"))
        .expect("rmcp cancel_task");

    let cancelled = SessionStore::open_in_repo(&fixture.repo_root)
        .expect("open session store")
        .get_durable_task("task-working")
        .expect("reload task")
        .expect("task exists");
    assert_eq!(cancelled.status, DurableTaskStatus::Cancelled);
    assert!(cancelled.cancel_requested);
}

#[test]
fn ping_returns_successful_empty_result() {
    let fixture = ToolFixture::new();
    fixture.server.ping_result().expect("ping");
}

#[test]
fn initialized_marks_server_ready() {
    let fixture = ToolFixture::new();
    assert!(!fixture.server.is_initialized());
    fixture.server.mark_initialized();
    assert!(fixture.server.is_initialized());
}

#[test]
fn set_level_maps_rmcp_levels_to_atlas_thresholds() {
    let fixture = ToolFixture::new();
    fixture
        .server
        .set_level_result(SetLevelRequestParams::new(LoggingLevel::Warning));
    assert_eq!(
        fixture.server.requested_log_level(),
        Some(crate::logging::LogLevel::Warning)
    );

    fixture
        .server
        .set_level_result(SetLevelRequestParams::new(LoggingLevel::Critical));
    assert_eq!(
        fixture.server.requested_log_level(),
        Some(crate::logging::LogLevel::Error)
    );
}

#[test]
fn set_trace_accepts_supported_values_and_rejects_invalid_values() {
    let fixture = ToolFixture::new();
    fixture
        .server
        .set_trace_level_result(Some(&json!({"value": "messages"})))
        .expect("messages trace level");
    assert_eq!(fixture.server.trace_level(), TraceLevel::Messages);

    fixture
        .server
        .set_trace_level_result(Some(&json!({"value": "verbose"})))
        .expect("verbose trace level");
    assert_eq!(fixture.server.trace_level(), TraceLevel::Verbose);

    let error = fixture
        .server
        .set_trace_level_result(Some(&json!({"value": "loud"})))
        .expect_err("invalid trace must fail");
    assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
    assert!(error.message.as_ref().contains("invalid $/setTrace value"));
}

#[test]
fn logging_notification_shape_uses_official_notification_payload() {
    let notification = super::logging_message_notification_param(
        crate::logging::LogLevel::Warning,
        "demo message",
    );
    let value = serde_json::to_value(notification).expect("serialize logging notification");
    assert_eq!(value["level"], json!("warning"));
    assert_eq!(value["logger"], json!("atlas-mcp"));
    assert_eq!(value["data"], json!("demo message"));
}

#[test]
fn logging_threshold_filters_completion_diagnostics() {
    let ok_response = CallToolResponse::Complete(rmcp::model::CallToolResult::success(vec![]));
    let err_response = CallToolResponse::Complete(rmcp::model::CallToolResult::error(vec![]));

    assert_eq!(
        super::tool_call_log_level(&ok_response),
        crate::logging::LogLevel::Info
    );
    assert_eq!(
        super::tool_call_log_level(&err_response),
        crate::logging::LogLevel::Error
    );
    assert!(!crate::logging::should_emit(
        Some(crate::logging::LogLevel::Warning),
        super::tool_call_log_level(&ok_response)
    ));
    assert!(crate::logging::should_emit(
        Some(crate::logging::LogLevel::Warning),
        super::tool_call_log_level(&err_response)
    ));
}

#[test]
fn custom_trace_unknown_method_returns_method_not_found() {
    let error = crate::rmcp_error::method_not_found("$/unknownTrace".to_owned());
    assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
    assert_eq!(error.message.as_ref(), "$/unknownTrace");
}

#[test]
fn dynamic_roots_refresh_only_runs_when_repo_context_missing() {
    let fixed = ToolFixture::new();
    assert!(!fixed.server.should_refresh_dynamic_roots());

    let dynamic = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
    assert!(dynamic.should_refresh_dynamic_roots());

    dynamic.set_candidate_roots_for_tests(Some(vec!["/tmp/repo".to_owned()]));
    assert!(!dynamic.should_refresh_dynamic_roots());
}
