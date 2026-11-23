use super::*;

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
            "extensions".to_owned(),
            "logging".to_owned(),
            "prompts".to_owned(),
            "resources".to_owned(),
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
        "tasks/update",
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

fn stdio_wait_for_task_result(repo_root: &str, db_path: &str, task_id: &str) -> Value {
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
        if task_status_value(&get) != json!("working") {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    task_completed_result(&stdio_initialized_call(
        repo_root,
        db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tasks/get",
            "params": { "taskId": task_id }
        }),
    ))
}

#[test]
fn spec_broker_status_direct_and_deferred_contracts_match_across_stdio_and_http() {
    let (_dir, repo_root, db_path) = setup_repo();

    let stdio_direct = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "broker_status", "arguments": {  } }
        }),
    );
    let stdio_deferred_create = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "broker_status",
                "arguments": with_task_ttl(json!({}))
            }
        }),
    );
    let stdio_task_id = task_id_from_call_tool_response(&stdio_deferred_create);
    let stdio_deferred = stdio_wait_for_task_result(&repo_root, &db_path, stdio_task_id);

    let harness = HttpTestHarness::new(&repo_root, &db_path);
    let session_id = http_session(&harness);
    let http_direct = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "broker_status", "arguments": {  } }
            }),
        )
        .expect("http direct broker_status")
        .json_body
        .expect("http direct json body");
    let direct_stdio = normalized_tool_result_contract(&stdio_direct["result"]);
    let deferred_stdio = normalized_tool_result_contract(&stdio_deferred);
    let direct_http = normalized_tool_result_contract(&http_direct["result"]);

    assert_eq!(direct_stdio, deferred_stdio);
    assert_eq!(direct_stdio, direct_http);
}

#[test]
fn spec_converted_stable_object_tools_match_across_direct_and_deferred_stdio_and_http() {
    let (_dir, repo_root, db_path) = setup_repo();
    build_fixture_graph(&repo_root, &db_path);
    seed_saved_context_and_decision(&repo_root, &db_path);

    let cases = vec![
        ("query_graph", json!({ "text": "greet" })),
        (
            "batch_query_graph",
            json!({
                "items": [{ "text": "greet" }, { "text": "greet_twice" }]
            }),
        ),
    ];

    let harness = HttpTestHarness::new(&repo_root, &db_path);
    let session_id = http_session(&harness);

    for (tool_name, arguments) in cases {
        let stdio_direct = stdio_initialized_call(
            &repo_root,
            &db_path,
            json!({
                "jsonrpc": "2.0",
                "id": 30,
                "method": "tools/call",
                "params": { "name": tool_name, "arguments": arguments.clone() }
            }),
        );
        let stdio_deferred_create = stdio_initialized_call(
            &repo_root,
            &db_path,
            json!({
                "jsonrpc": "2.0",
                "id": 31,
                "method": "tools/call",
                "params": {
                    "name": tool_name,
                    "arguments": with_task_ttl(arguments.clone())
                }
            }),
        );
        let stdio_task_id = task_id_from_call_tool_response(&stdio_deferred_create);
        let stdio_deferred = stdio_wait_for_task_result(&repo_root, &db_path, stdio_task_id);

        let http_direct = harness
            .post_jsonrpc(
                &http_headers(&session_id),
                &json!({
                    "jsonrpc": "2.0",
                    "id": 32,
                    "method": "tools/call",
                    "params": { "name": tool_name, "arguments": arguments.clone() }
                }),
            )
            .unwrap_or_else(|_| panic!("http direct {tool_name}"))
            .json_body
            .expect("http direct json body");
        let direct_stdio = normalized_tool_result_contract(&stdio_direct["result"]);
        let deferred_stdio = normalized_tool_result_contract(&stdio_deferred);
        let direct_http = normalized_tool_result_contract(&http_direct["result"]);

        assert_eq!(
            direct_stdio, deferred_stdio,
            "stdio deferred mismatch for {tool_name}"
        );
        assert_eq!(
            direct_stdio, direct_http,
            "http direct mismatch for {tool_name}"
        );
        assert!(
            direct_stdio["structuredContent"]["tool"]
                .as_str()
                .is_some_and(|name| name == tool_name),
            "structuredContent.tool must match {tool_name}"
        );
    }
}

#[test]
fn spec_tool_error_contract_matches_across_direct_and_deferred_stdio_and_http() {
    let (_dir, repo_root, db_path) = setup_repo();
    let invalid_args = json!({ "query": "(", "is_regex": true });

    let stdio_direct = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "search_content", "arguments": invalid_args.clone() }
        }),
    );
    let stdio_deferred_create = stdio_initialized_call(
        &repo_root,
        &db_path,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "search_content",
                "arguments": with_task_ttl(invalid_args.clone())
            }
        }),
    );
    let stdio_task_id = task_id_from_call_tool_response(&stdio_deferred_create);
    let stdio_deferred = stdio_wait_for_task_result(&repo_root, &db_path, stdio_task_id);

    let harness = HttpTestHarness::new(&repo_root, &db_path);
    let session_id = http_session(&harness);
    let http_direct = harness
        .post_jsonrpc(
            &http_headers(&session_id),
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "search_content", "arguments": invalid_args.clone() }
            }),
        )
        .expect("http direct search_content")
        .json_body
        .expect("http direct json body");
    let direct_stdio = normalized_tool_result_contract(&stdio_direct["result"]);
    let deferred_stdio = normalized_tool_result_contract(&stdio_deferred);
    let direct_http = normalized_tool_result_contract(&http_direct["result"]);

    assert_eq!(direct_stdio["isError"], json!(true));
    assert_eq!(direct_stdio, deferred_stdio);
    assert_eq!(direct_stdio, direct_http);
}
