use super::*;

fn create_durable_task_with_times(
    store: &mut SessionStore,
    task_id: &str,
    created_at: &str,
    updated_at: &str,
) {
    store
        .create_durable_task(&NewDurableTask {
            task_id: task_id.to_owned(),
            originating_method: "tools/call".to_owned(),
            request_id: Some(task_id.to_owned()),
            tool_name: Some("doctor".to_owned()),
            transport_kind: Some("stdio".to_owned()),
            session_id: None,
            status: DurableTaskStatus::Working,
            status_message: Some("working".to_owned()),
            ttl_ms: Some(1000),
        })
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE durable_tasks SET created_at = ?2, updated_at = ?3 WHERE task_id = ?1",
            params![task_id, created_at, updated_at],
        )
        .unwrap();
}

#[test]
fn durable_task_round_trips_input_requests_and_request_state() {
    let (_dir, mut store) = open_store(16, 1024);
    store
        .create_durable_task(&NewDurableTask {
            task_id: "task-input".to_owned(),
            originating_method: "tools/call".to_owned(),
            request_id: Some("request-1".to_owned()),
            tool_name: Some("purge_saved_context".to_owned()),
            transport_kind: Some("rmcp".to_owned()),
            session_id: None,
            status: DurableTaskStatus::Working,
            status_message: Some("working".to_owned()),
            ttl_ms: Some(1000),
        })
        .unwrap();
    store
        .update_durable_task(
            "task-input",
            &DurableTaskUpdate {
                status: Some(DurableTaskStatus::InputRequired),
                status_message: Some("input required".to_owned()),
                input_requests: Some(serde_json::json!({
                    "confirmation": {
                        "method": "elicitation/create",
                        "params": {
                            "message": "Confirm destructive action",
                            "requestedSchema": {
                                "type": "object",
                                "properties": {
                                    "confirmation": { "type": "string" }
                                },
                                "required": ["confirmation"]
                            }
                        }
                    }
                })),
                request_state: Some("sealed-request-state".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();

    let task = store.get_durable_task("task-input").unwrap().unwrap();
    assert_eq!(task.status, DurableTaskStatus::InputRequired);
    assert_eq!(task.request_state.as_deref(), Some("sealed-request-state"));
    assert_eq!(
        task.input_requests
            .as_ref()
            .and_then(|value| value.get("confirmation")),
        Some(&serde_json::json!({
            "method": "elicitation/create",
            "params": {
                "message": "Confirm destructive action",
                "requestedSchema": {
                    "type": "object",
                    "properties": {
                        "confirmation": { "type": "string" }
                    },
                    "required": ["confirmation"]
                }
            }
        }))
    );
}

#[test]
fn list_durable_tasks_uses_recency_order_and_stable_cursor() {
    let (_dir, mut store) = open_store(16, 1024);
    create_durable_task_with_times(
        &mut store,
        "task-older-id",
        "2026-01-01T00:00:00Z",
        "2026-01-01T00:00:05Z",
    );
    create_durable_task_with_times(
        &mut store,
        "task-newer-id",
        "2026-01-01T00:00:01Z",
        "2026-01-01T00:00:05Z",
    );
    create_durable_task_with_times(
        &mut store,
        "task-newest",
        "2026-01-01T00:00:02Z",
        "2026-01-01T00:00:06Z",
    );

    let first_page = store.list_durable_tasks(None, 2).unwrap();
    let first_ids = first_page
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(first_ids, vec!["task-newest", "task-newer-id"]);

    let next_cursor = first_page.next_cursor.expect("next cursor present");
    let cursor_json: Value = serde_json::from_str(&next_cursor).unwrap();
    assert_eq!(
        cursor_json["task_id"],
        Value::String("task-newer-id".to_owned())
    );

    let second_page = store.list_durable_tasks(Some(&next_cursor), 2).unwrap();
    let second_ids = second_page
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(second_ids, vec!["task-older-id"]);
    assert!(second_page.next_cursor.is_none());
}

#[test]
fn list_durable_tasks_rejects_malformed_cursor() {
    let (_dir, store) = open_store(16, 1024);
    let error = store.list_durable_tasks(Some("not-json"), 5).unwrap_err();
    assert!(error.to_string().contains("invalid durable task cursor"));
}
