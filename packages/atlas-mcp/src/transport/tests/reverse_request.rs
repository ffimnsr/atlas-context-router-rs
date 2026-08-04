use super::*;

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
