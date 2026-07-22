use super::*;
use atlas_store_sqlite::BuildFinishStats;

// ── helpers ──────────────────────────────────────────────────────────────────

fn finish_build(db_path: &str, repo_root: &str, state: atlas_store_sqlite::GraphBuildState) {
    let store = Store::open(db_path).expect("open store");
    store
        .finish_build(
            repo_root,
            BuildFinishStats {
                state,
                files_discovered: 3,
                files_processed: 3,
                files_accepted: 3,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 300,
                bytes_skipped: 0,
                nodes_written: 3,
                edges_written: 1,
                budget_stop_reason: None,
            },
        )
        .expect("finish_build");
}

fn call_json(
    tool: &str,
    args: serde_json::Value,
    repo_root: &str,
    db_path: &str,
) -> serde_json::Value {
    let args_with_fmt = {
        let mut a = args.clone();
        a["output_format"] = serde_json::json!("json");
        a
    };
    call(tool, Some(&args_with_fmt), repo_root, db_path).expect("tool call should succeed")
}

fn parse_body(resp: &serde_json::Value) -> serde_json::Value {
    let text = unwrap_tool_text(resp.clone());
    serde_json::from_str(&text).expect("parse json body")
}

// ── status execution_state ───────────────────────────────────────────────────

#[test]
fn status_emits_execution_state_fresh_after_build() {
    let fixture = setup_mcp_fixture();
    finish_build(
        &fixture.db_path,
        "/repo",
        atlas_store_sqlite::GraphBuildState::Built,
    );

    let resp = call_json("status", serde_json::json!({}), "/repo", &fixture.db_path);
    let body = parse_body(&resp);

    assert_eq!(
        body["graph_state"]["execution_state"].as_str(),
        Some("fresh"),
        "expected execution_state=fresh after a successful build, got: {:?}",
        body["graph_state"]["execution_state"]
    );
}

#[test]
fn status_emits_execution_state_missing_when_no_db() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no_such.db").to_string_lossy().to_string();

    let resp = call_json("status", serde_json::json!({}), "/repo", &missing);
    let body = parse_body(&resp);

    assert_eq!(
        body["graph_state"]["execution_state"].as_str(),
        Some("missing"),
        "expected execution_state=missing when db absent"
    );
}

#[test]
fn status_emits_execution_state_partial_after_degraded_build() {
    let fixture = setup_mcp_fixture();
    finish_build(
        &fixture.db_path,
        "/repo",
        atlas_store_sqlite::GraphBuildState::Degraded,
    );

    let resp = call_json("status", serde_json::json!({}), "/repo", &fixture.db_path);
    let body = parse_body(&resp);

    assert_eq!(
        body["graph_state"]["execution_state"].as_str(),
        Some("partial"),
        "expected execution_state=partial after a degraded build"
    );
}

// ── graph-backed tools blocked when db missing ───────────────────────────────

fn assert_tool_blocked_missing(tool: &str, args: serde_json::Value) {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no_such.db").to_string_lossy().to_string();

    let resp = call(tool, Some(&args), "/repo", &missing).expect("blocked tool should return Ok");
    assert_eq!(
        resp["isError"].as_bool(),
        Some(true),
        "{tool}: expected isError=true when db missing"
    );
    assert_eq!(resp["content"][0]["type"].as_str(), Some("text"));
    assert_eq!(
        resp["structuredContent"]["code"].as_str(),
        Some("graph_stale")
    );
    assert_eq!(resp["structuredContent"]["tool"].as_str(), Some(tool));
    let es = resp["atlas_readiness"]["execution_state"].as_str();
    assert_eq!(
        es,
        Some("missing"),
        "{tool}: expected atlas_readiness.execution_state=missing"
    );
    assert_eq!(
        resp["structuredContent"]["details"]["execution_state"].as_str(),
        Some("missing"),
        "{tool}: expected structuredContent.details.execution_state=missing"
    );
    let blocked = resp["atlas_readiness"]["blocked"].as_bool();
    assert_eq!(
        blocked,
        Some(true),
        "{tool}: expected atlas_readiness.blocked=true"
    );
}

#[test]
fn query_graph_blocked_when_db_missing() {
    assert_tool_blocked_missing(
        "query_graph",
        serde_json::json!({ "text": "compute", "output_format": "json" }),
    );
}

#[test]
fn symbol_neighbors_blocked_when_db_missing() {
    assert_tool_blocked_missing(
        "symbol_neighbors",
        serde_json::json!({ "qualified_name": "src/service.rs::fn::compute", "output_format": "json" }),
    );
}

#[test]
fn traverse_graph_blocked_when_db_missing() {
    assert_tool_blocked_missing(
        "traverse_graph",
        serde_json::json!({ "qualified_name": "src/service.rs::fn::compute", "output_format": "json" }),
    );
}

#[test]
fn get_context_blocked_when_db_missing() {
    assert_tool_blocked_missing(
        "get_context",
        serde_json::json!({ "query": "compute", "output_format": "json" }),
    );
}

#[test]
fn get_impact_radius_blocked_when_db_missing() {
    assert_tool_blocked_missing(
        "get_impact_radius",
        serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" }),
    );
}

#[test]
fn get_review_context_blocked_when_db_missing() {
    assert_tool_blocked_missing(
        "get_review_context",
        serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" }),
    );
}

#[test]
fn analyze_safety_blocked_when_db_missing() {
    assert_tool_blocked_missing(
        "analyze_safety",
        serde_json::json!({ "qualified_name": "src/service.rs::fn::compute", "output_format": "json" }),
    );
}

// ── graph-backed tools stamp atlas_readiness when graph ready ─────────────────

#[test]
fn query_graph_stamps_atlas_readiness_when_graph_ready() {
    let fixture = setup_mcp_fixture();
    finish_build(
        &fixture.db_path,
        "/repo",
        atlas_store_sqlite::GraphBuildState::Built,
    );

    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("query_graph should succeed");

    let readiness = &resp["atlas_readiness"];
    assert!(
        readiness.is_object(),
        "query_graph response should have atlas_readiness field"
    );
    assert_eq!(
        readiness["execution_state"].as_str(),
        Some("fresh"),
        "query_graph: atlas_readiness.execution_state should be fresh"
    );
    assert_eq!(
        readiness["safe_to_answer"].as_bool(),
        Some(true),
        "query_graph: atlas_readiness.safe_to_answer should be true"
    );
}

#[test]
fn get_context_stamps_atlas_readiness_when_graph_ready() {
    let fixture = setup_mcp_fixture();
    finish_build(
        &fixture.db_path,
        "/repo",
        atlas_store_sqlite::GraphBuildState::Built,
    );

    let args = serde_json::json!({ "query": "compute", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/repo", &fixture.db_path)
        .expect("get_context should succeed");

    let readiness = &resp["atlas_readiness"];
    assert!(
        readiness.is_object(),
        "get_context response must have atlas_readiness"
    );
    assert_eq!(readiness["execution_state"].as_str(), Some("fresh"));
}

// ── consistency: status and graph tools agree on execution_state ──────────────

#[test]
fn status_and_query_graph_agree_on_execution_state() {
    let fixture = setup_mcp_fixture();
    finish_build(
        &fixture.db_path,
        "/repo",
        atlas_store_sqlite::GraphBuildState::Built,
    );

    let status_resp = call_json("status", serde_json::json!({}), "/repo", &fixture.db_path);
    let status_body = parse_body(&status_resp);
    let status_es = status_body["graph_state"]["execution_state"]
        .as_str()
        .expect("status.graph_state.execution_state");

    let qg_args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let qg_resp =
        call("query_graph", Some(&qg_args), "/repo", &fixture.db_path).expect("query_graph");
    let qg_es = qg_resp["atlas_readiness"]["execution_state"]
        .as_str()
        .expect("query_graph.atlas_readiness.execution_state");

    assert_eq!(
        status_es, qg_es,
        "status and query_graph must agree on execution_state for the same repo/db"
    );
}

// ── partial state blocks analysis tools ──────────────────────────────────────

#[test]
fn get_context_blocked_on_partial_state() {
    let fixture = setup_mcp_fixture();
    finish_build(
        &fixture.db_path,
        "/repo",
        atlas_store_sqlite::GraphBuildState::Degraded,
    );

    let args = serde_json::json!({ "query": "compute", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/repo", &fixture.db_path)
        .expect("blocked tool should return Ok");

    assert_eq!(
        resp["isError"].as_bool(),
        Some(true),
        "get_context should be blocked on partial state"
    );
    assert_eq!(
        resp["atlas_readiness"]["execution_state"].as_str(),
        Some("partial")
    );
}

#[test]
fn query_graph_allowed_with_allow_partial_on_partial_state() {
    // Analysis (get_context, etc.) is always blocked in Partial state even
    // with allow_partial=true.  SymbolLookup (query_graph) IS allowed with
    // allow_partial=true when the graph is partial.
    let fixture = setup_mcp_fixture();
    finish_build(
        &fixture.db_path,
        "/repo",
        atlas_store_sqlite::GraphBuildState::Degraded,
    );

    // Without allow_partial, query_graph should be blocked on partial.
    let args_no_override = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp_blocked = call(
        "query_graph",
        Some(&args_no_override),
        "/repo",
        &fixture.db_path,
    )
    .expect("blocked tool should return Ok");
    assert_eq!(
        resp_blocked["isError"].as_bool(),
        Some(true),
        "query_graph without allow_partial should be blocked on partial state"
    );

    // With allow_partial=true, query_graph should be allowed.
    let args_with_override =
        serde_json::json!({ "text": "compute", "allow_partial": true, "output_format": "json" });
    let resp_allowed = call(
        "query_graph",
        Some(&args_with_override),
        "/repo",
        &fixture.db_path,
    )
    .expect("query_graph with allow_partial=true should succeed");
    assert_ne!(
        resp_allowed["isError"].as_bool(),
        Some(true),
        "query_graph with allow_partial=true must not be blocked"
    );
    let readiness = &resp_allowed["atlas_readiness"];
    assert_eq!(readiness["execution_state"].as_str(), Some("partial"));
    // safe_to_answer=false because graph is partial
    assert_eq!(readiness["safe_to_answer"].as_bool(), Some(false));
}

// ── blocked responses still include atlas_provenance ─────────────────────────

#[test]
fn blocked_response_includes_atlas_provenance() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no_such.db").to_string_lossy().to_string();

    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("query_graph", Some(&args), "/repo", &missing).expect("blocked tool should return Ok");

    assert!(
        resp.get("atlas_provenance").is_some(),
        "blocked response must include atlas_provenance"
    );
    assert_eq!(
        resp["atlas_provenance"]["repo_root"].as_str(),
        Some("/repo")
    );
}

#[test]
fn blocked_response_does_not_emit_legacy_text_wrapper_shape() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no_such.db").to_string_lossy().to_string();

    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("query_graph", Some(&args), "/repo", &missing).expect("blocked tool should return Ok");

    assert!(
        resp.get("Text").is_none(),
        "legacy Text wrapper must not appear at top level"
    );
    assert!(
        resp["content"]
            .as_array()
            .is_some_and(|content| content.iter().all(|item| item.get("Text").is_none())),
        "legacy Text wrapper must not appear inside content blocks"
    );
}
