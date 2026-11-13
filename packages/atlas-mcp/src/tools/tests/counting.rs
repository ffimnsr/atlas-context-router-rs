//! Counting-parity drift tests.
//!
//! `status`, `list_graph_stats`, `debug_graph`, and `list_symbols` must report
//! the same live graph totals. `doctor` splits its counts: the `graph_stats`
//! check carries live totals while `graph_build_state` carries build-time
//! telemetry (`nodes_written_last_build`).

use super::*;

fn call_json(
    tool: &str,
    args: serde_json::Value,
    repo_root: &str,
    db_path: &str,
) -> serde_json::Value {
    let with_format = {
        let mut value = args;
        value["output_format"] = serde_json::json!("json");
        value
    };
    call(tool, Some(&with_format), repo_root, db_path).expect("tool call should succeed")
}

fn parse_body(resp: &serde_json::Value) -> serde_json::Value {
    let text = unwrap_tool_text(resp.clone());
    serde_json::from_str(&text).expect("parse json body")
}

fn seed_synthetic_owner_node(db_path: &str, repo_root: &str) {
    let mut store = Store::open(db_path).expect("open store");
    let repo_id = atlas_repo::stable_repo_id(camino::Utf8Path::new(repo_root));
    let synthetic_path = ".atlas/synthetic/owners/package/cargo/demo.atlas";
    let synthetic = make_node(NodeKind::Package, "demo", "package::demo", synthetic_path);
    store
        .replace_file_graph_for_repo(
            &repo_id,
            synthetic_path,
            "hash:synthetic",
            Some("toml"),
            Some(1),
            std::slice::from_ref(&synthetic),
            &[],
        )
        .expect("insert synthetic owner node");
}

fn doctor_check_message(doctor: &serde_json::Value, name: &str) -> String {
    doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|check| check["name"].as_str() == Some(name))
        .and_then(|check| check["message"].as_str())
        .unwrap_or_else(|| panic!("doctor check {name} missing"))
        .to_owned()
}

#[test]
fn live_graph_counts_match_across_status_stats_debug_and_symbols() {
    let fixture = setup_mcp_fixture();
    // Synthetic owner nodes are excluded from `graph_stats()` but must not leak
    // into `list_symbols.total`.
    seed_synthetic_owner_node(&fixture.db_path, &fixture.repo_root);
    let repo_root = fixture.repo_root.as_str();
    let db_path = fixture.db_path.as_str();

    let status = parse_body(&call_json(
        "status",
        serde_json::json!({}),
        repo_root,
        db_path,
    ));
    let stats = parse_body(&call_json(
        "list_graph_stats",
        serde_json::json!({}),
        repo_root,
        db_path,
    ));
    let debug = parse_body(&call_json(
        "debug_graph",
        serde_json::json!({}),
        repo_root,
        db_path,
    ));
    let symbols = parse_body(&call_json(
        "list_symbols",
        serde_json::json!({ "limit": 1 }),
        repo_root,
        db_path,
    ));
    let doctor = parse_body(&call_json(
        "doctor",
        serde_json::json!({}),
        repo_root,
        db_path,
    ));

    let node_count = status["node_count"].as_u64().expect("status.node_count");
    let file_count = status["indexed_file_count"]
        .as_u64()
        .expect("status.indexed_file_count");

    assert_eq!(
        stats["node_count"].as_u64(),
        Some(node_count),
        "list_graph_stats must match status"
    );
    assert_eq!(
        stats["file_count"].as_u64(),
        Some(file_count),
        "list_graph_stats file count must match status"
    );
    assert_eq!(
        debug["summary"]["node_count"].as_u64(),
        Some(node_count),
        "debug_graph must match status"
    );
    assert_eq!(
        debug["summary"]["file_count"].as_u64(),
        Some(file_count),
        "debug_graph file count must match status"
    );
    assert_eq!(
        symbols["total"].as_u64(),
        Some(node_count),
        "list_symbols.total must match the canonical live node count"
    );

    let graph_stats_check = doctor_check_message(&doctor, "graph_stats");
    assert_eq!(
        graph_stats_check,
        format!(
            "files={file_count} nodes={node_count} edges={}",
            status["edge_count"].as_u64().expect("status.edge_count")
        ),
        "doctor graph_stats check must match status"
    );
}

#[test]
fn doctor_build_state_labels_build_telemetry() {
    let fixture = setup_mcp_fixture();
    let store = Store::open(&fixture.db_path).expect("open store");
    store
        .finish_build_for_repo(
            "repo_test",
            &fixture.repo_root,
            BuildFinishStats {
                state: atlas_store_sqlite::GraphBuildState::Built,
                files_discovered: 3,
                files_processed: 3,
                files_accepted: 3,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 300,
                bytes_skipped: 0,
                // Deliberately different from the live node count so a field
                // rename regression is visible.
                nodes_written: 999,
                edges_written: 1,
                budget_stop_reason: None,
            },
        )
        .expect("finish build");

    let doctor = parse_body(&call_json(
        "doctor",
        serde_json::json!({}),
        &fixture.repo_root,
        &fixture.db_path,
    ));
    let message = doctor_check_message(&doctor, "graph_build_state");
    assert!(
        message.contains("nodes_written_last_build=999"),
        "build telemetry must be labelled as build-time, got: {message}"
    );
    assert!(
        !message.contains(" nodes=999"),
        "build telemetry must not read as a live node count, got: {message}"
    );
}
