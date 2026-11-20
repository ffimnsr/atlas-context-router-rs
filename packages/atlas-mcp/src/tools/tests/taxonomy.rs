//! Read-surface tests for the persisted taxonomy tools: `list_communities`
//! and `list_flows`, plus the `status` postprocess freshness signal.

use super::*;
use atlas_core::{
    PostprocessExecutionMode, PostprocessRunState, PostprocessRunSummary, PostprocessStageStatus,
    PostprocessStageSummary,
};
use atlas_store_sqlite::BuildFinishStats;

fn call_json(tool: &str, args: serde_json::Value, fixture: &McpFixture) -> serde_json::Value {
    let mut with_format = args;
    with_format["output_format"] = serde_json::json!("json");
    call(
        tool,
        Some(&with_format),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .unwrap_or_else(|error| panic!("{tool} call failed: {error}"))
}

fn parse_body(resp: &serde_json::Value) -> serde_json::Value {
    let text = unwrap_tool_text(resp.clone());
    serde_json::from_str(&text).expect("parse json body")
}

fn seed_taxonomy(db_path: &str) {
    let store = Store::open(db_path).expect("open store");

    let community = store
        .create_community("cluster-a", Some("louvain"), Some(0), None)
        .expect("create community");
    store
        .add_community_node(community, "src/service.rs::fn::compute")
        .expect("add community node");
    store
        .add_community_node(community, "src/api.rs::fn::handle_request")
        .expect("add community node");

    let flow = store
        .create_flow("checkout", Some("feature"), Some("cart to charge"))
        .expect("create flow");
    store
        .add_flow_member(flow, "src/cart.rs::fn::add", Some(1), Some("entry"))
        .expect("add flow member");
    store
        .add_flow_member(flow, "src/pay.rs::fn::charge", Some(2), None)
        .expect("add flow member");

    let other = store
        .create_flow("auth", None, None)
        .expect("create other flow");
    store
        .add_flow_member(other, "src/pay.rs::fn::charge", Some(1), Some("final"))
        .expect("add other flow member");
}

#[test]
fn list_communities_reports_counts_and_optional_nodes() {
    let fixture = setup_mcp_fixture();
    seed_taxonomy(&fixture.db_path);

    let body = parse_body(&call_json(
        "list_communities",
        serde_json::json!({}),
        &fixture,
    ));
    assert_eq!(body["summary"]["community_count"].as_u64(), Some(1));
    assert_eq!(body["summary"]["total_memberships"].as_u64(), Some(2));
    assert_eq!(
        body["summary"]["include_nodes"].as_bool(),
        Some(false),
        "include_nodes must default to false"
    );
    let community = &body["communities"][0];
    assert_eq!(community["name"].as_str(), Some("cluster-a"));
    assert_eq!(community["algorithm"].as_str(), Some("louvain"));
    assert_eq!(community["member_count"].as_u64(), Some(2));
    assert!(
        community.get("nodes").is_none(),
        "nodes must stay hidden without include_nodes"
    );

    let body = parse_body(&call_json(
        "list_communities",
        serde_json::json!({ "include_nodes": true }),
        &fixture,
    ));
    let community = &body["communities"][0];
    let nodes = community["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 2);
    assert_eq!(
        community["nodes_truncated"].as_bool(),
        Some(false),
        "two members must not truncate"
    );
    // Membership ordering is by qualified name, not insertion order.
    let node_names = nodes
        .iter()
        .map(|node| node.as_str().expect("node qn").to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        node_names,
        [
            "src/api.rs::fn::handle_request".to_owned(),
            "src/service.rs::fn::compute".to_owned(),
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    );
}

#[test]
fn list_communities_caps_nodes_with_truncated_flag() {
    let fixture = setup_mcp_fixture();
    let store = Store::open(&fixture.db_path).expect("open store");
    let community = store
        .create_community("big", Some("louvain"), Some(0), None)
        .expect("create community");
    for index in 0..101 {
        store
            .add_community_node(community, &format!("src/big.rs::fn::step_{index}"))
            .expect("add community node");
    }

    let body = parse_body(&call_json(
        "list_communities",
        serde_json::json!({ "include_nodes": true }),
        &fixture,
    ));
    let community = &body["communities"][0];
    assert_eq!(community["member_count"].as_u64(), Some(101));
    assert_eq!(
        community["nodes"].as_array().expect("nodes").len(),
        100,
        "node list must cap at 100"
    );
    assert_eq!(community["nodes_truncated"].as_bool(), Some(true));
}

#[test]
fn list_communities_empty_state_returns_zero_counts() {
    let fixture = setup_mcp_fixture();
    let body = parse_body(&call_json(
        "list_communities",
        serde_json::json!({}),
        &fixture,
    ));
    assert_eq!(
        body["communities"].as_array().expect("communities").len(),
        0
    );
    assert_eq!(body["summary"]["community_count"].as_u64(), Some(0));
}

#[test]
fn list_flows_returns_members_and_both_filters() {
    let fixture = setup_mcp_fixture();
    seed_taxonomy(&fixture.db_path);

    let body = parse_body(&call_json("list_flows", serde_json::json!({}), &fixture));
    assert_eq!(body["summary"]["flow_count"].as_u64(), Some(2));
    assert_eq!(body["summary"]["filtered_by"].as_str(), None);
    let flow_names = body["flows"]
        .as_array()
        .expect("flows")
        .iter()
        .map(|flow| flow["name"].as_str().expect("name").to_owned())
        .collect::<Vec<_>>();
    assert!(flow_names.contains(&"checkout".to_owned()));
    assert!(flow_names.contains(&"auth".to_owned()));

    let body = parse_body(&call_json(
        "list_flows",
        serde_json::json!({ "name": "checkout" }),
        &fixture,
    ));
    assert_eq!(body["summary"]["flow_count"].as_u64(), Some(1));
    assert_eq!(body["summary"]["filtered_by"].as_str(), Some("name"));
    assert_eq!(body["summary"]["matched"].as_bool(), Some(true));
    let flow = &body["flows"][0];
    assert_eq!(flow["kind"].as_str(), Some("feature"));
    assert_eq!(flow["description"].as_str(), Some("cart to charge"));
    let members = flow["members"].as_array().expect("members");
    assert_eq!(members.len(), 2);
    assert_eq!(members[0]["node_qn"].as_str(), Some("src/cart.rs::fn::add"));
    assert_eq!(members[0]["position"].as_i64(), Some(1));
    assert_eq!(members[0]["role"].as_str(), Some("entry"));
    assert_eq!(
        members[1]["node_qn"].as_str(),
        Some("src/pay.rs::fn::charge")
    );
    assert_eq!(flow["members_truncated"].as_bool(), Some(false));

    let body = parse_body(&call_json(
        "list_flows",
        serde_json::json!({ "node_qn": "src/pay.rs::fn::charge" }),
        &fixture,
    ));
    assert_eq!(body["summary"]["filtered_by"].as_str(), Some("node_qn"));
    assert_eq!(body["summary"]["flow_count"].as_u64(), Some(2));
}

#[test]
fn list_flows_name_and_node_filters_fail_closed() {
    let fixture = setup_mcp_fixture();
    seed_taxonomy(&fixture.db_path);

    let resp = call(
        "list_flows",
        Some(&serde_json::json!({
            "name": "checkout",
            "node_qn": "src/pay.rs::fn::charge",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("list_flows conflict result");
    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("list_flows accepts either name or node_qn, not both")
    );
}

#[test]
fn list_flows_name_miss_returns_empty_with_warning() {
    let fixture = setup_mcp_fixture();
    seed_taxonomy(&fixture.db_path);

    let body = parse_body(&call_json(
        "list_flows",
        serde_json::json!({ "name": "missing-flow" }),
        &fixture,
    ));
    assert_eq!(body["flows"].as_array().expect("flows").len(), 0);
    assert_eq!(body["summary"]["matched"].as_bool(), Some(false));
    let warnings = body["warnings"].as_array().expect("warnings");
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .expect("warning text")
            .contains("missing-flow")),
        "warnings must name the missing flow: {warnings:?}"
    );
}

fn postprocess_summary(repo_root: &str, finished_at_ms: i64, stage: &str) -> PostprocessRunSummary {
    PostprocessRunSummary {
        repo_root: repo_root.to_owned(),
        ok: true,
        noop: false,
        noop_reason: None,
        error_code: "none".to_owned(),
        message: "postprocess complete".to_owned(),
        suggestions: Vec::new(),
        graph_built: true,
        state: PostprocessRunState::Succeeded,
        requested_mode: PostprocessExecutionMode::Full,
        stage_filter: None,
        dry_run: false,
        changed_files: Vec::new(),
        started_at_ms: finished_at_ms - 1_000,
        finished_at_ms,
        total_elapsed_ms: 1_000,
        stages: vec![PostprocessStageSummary {
            stage: stage.to_owned(),
            status: PostprocessStageStatus::Completed,
            mode: PostprocessExecutionMode::Full,
            affected_file_count: 1,
            item_count: 1,
            elapsed_ms: 1,
            error_code: None,
            message: None,
            details: serde_json::json!({}),
        }],
        supported_stages: atlas_engine::supported_postprocess_stages(),
    }
}

fn finish_built(repo_root: &str, db_path: &str) {
    let store = Store::open(db_path).expect("open store");
    let source_repo_id = atlas_repo::stable_repo_id(camino::Utf8Path::new(repo_root));
    store
        .finish_build_for_repo(
            &source_repo_id,
            repo_root,
            BuildFinishStats {
                state: atlas_store_sqlite::GraphBuildState::Built,
                files_discovered: 4,
                files_processed: 4,
                files_accepted: 4,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 400,
                bytes_skipped: 0,
                nodes_written: 5,
                edges_written: 2,
                budget_stop_reason: None,
            },
        )
        .expect("finish build");
}

#[test]
fn status_reports_postprocess_block_with_communities_freshness() {
    let fixture = setup_mcp_fixture();
    let repo_root = fixture.repo_root.clone();
    let db_path = fixture.db_path.clone();

    // No build and no postprocess rows yet: the key exists but is null so the
    // output schema stays exact.
    let body = parse_body(&call_json("status", serde_json::json!({}), &fixture));
    assert_eq!(body["postprocess"], serde_json::json!(null));

    finish_built(&repo_root, &db_path);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();

    let store = Store::open(&db_path).expect("open store");
    store
        .begin_postprocess(&repo_root, PostprocessExecutionMode::Full, None, 0)
        .expect("begin postprocess");
    // Postprocess finished 60s before the latest build: communities must be
    // flagged stale with a warning.
    store
        .finish_postprocess(&postprocess_summary(
            &repo_root,
            now_ms - 60_000,
            "communities",
        ))
        .expect("finish postprocess");

    let body = parse_body(&call_json("status", serde_json::json!({}), &fixture));
    let postprocess = &body["postprocess"];
    assert_eq!(postprocess["state"].as_str(), Some("succeeded"));
    assert_eq!(postprocess["mode"].as_str(), Some("full"));
    assert_eq!(
        postprocess["communities_refreshed_after_last_build"].as_bool(),
        Some(false),
        "postprocess older than the build must flag stale communities"
    );
    assert_eq!(
        postprocess["finished_after_last_build"].as_bool(),
        Some(false)
    );
    assert_eq!(
        postprocess["stages"][0]["stage"].as_str(),
        Some("communities")
    );
    let warnings = body["warnings"].as_array().expect("warnings");
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .expect("warning text")
            .contains("run postprocess_graph with stage communities")),
        "stale communities must surface a postprocess warning: {warnings:?}"
    );

    // A postprocess pass that finished after the build clears the flag.
    store
        .finish_postprocess(&postprocess_summary(
            &repo_root,
            now_ms + 60_000,
            "communities",
        ))
        .expect("finish postprocess");
    let body = parse_body(&call_json("status", serde_json::json!({}), &fixture));
    assert_eq!(
        body["postprocess"]["communities_refreshed_after_last_build"].as_bool(),
        Some(true)
    );
    let warnings = body["warnings"].as_array().expect("warnings");
    assert!(
        !warnings.iter().any(|warning| warning
            .as_str()
            .expect("warning text")
            .contains("postprocess_graph with stage communities")),
        "fresh communities must not warn: {warnings:?}"
    );
}

#[test]
fn status_postprocess_block_reports_null_without_communities_stage() {
    let fixture = setup_mcp_fixture();
    let repo_root = fixture.repo_root.clone();
    let db_path = fixture.db_path.clone();
    finish_built(&repo_root, &db_path);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();

    let store = Store::open(&db_path).expect("open store");
    store
        .begin_postprocess(&repo_root, PostprocessExecutionMode::Full, None, 0)
        .expect("begin postprocess");
    // Stage-filtered run that never touched communities cannot vouch for it.
    store
        .finish_postprocess(&postprocess_summary(
            &repo_root,
            now_ms + 60_000,
            "query_hints",
        ))
        .expect("finish postprocess");

    let body = parse_body(&call_json("status", serde_json::json!({}), &fixture));
    assert_eq!(
        body["postprocess"]["communities_refreshed_after_last_build"],
        serde_json::json!(null),
        "runs without a communities stage must report unknown, not fresh"
    );
    assert!(
        !body["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning
                .as_str()
                .expect("warning text")
                .contains("postprocess_graph with stage communities")),
        "unknown freshness must not warn"
    );
}
