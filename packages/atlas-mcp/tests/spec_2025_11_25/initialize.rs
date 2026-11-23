use super::*;

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
    let session =
        InteractiveStdioTestSession::start(&repo_root, &db_path, ServerOptions::default())
            .expect("start stdio interactive test session");
    session
        .send_json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {}
            }
        }))
        .expect("send invalid initialize");
    let rejection = session
        .recv_json(Duration::from_secs(1))
        .expect("recv stdio rejection")
        .expect("rmcp stdio invalid initialize must return a typed error response");
    assert_eq!(
        initialize_rejection_stdio_snapshot(&rejection),
        read_fixture("initialize_rejection.stdio.json")
    );
    let _ = session.finish();
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
