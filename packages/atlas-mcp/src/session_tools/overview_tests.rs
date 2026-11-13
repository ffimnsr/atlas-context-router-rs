//! Unit tests for the overview MCP tools (ICM-G).

use super::test_util::{setup_db_path, tool_body};
use super::*;
use tempfile::TempDir;

fn setup() -> (TempDir, String, String) {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_string_lossy().into_owned();
    (dir, repo_root, db_path)
}

fn json_call(
    tool: fn(Option<&serde_json::Value>, &str, &str, OutputFormat) -> anyhow::Result<Value>,
    args: serde_json::Value,
    repo_root: &str,
    db_path: &str,
) -> Value {
    tool_body(&tool(Some(&args), repo_root, db_path, OutputFormat::Json).unwrap())
}

fn project_analysis(repo_root: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "subject": {
            "kind": "project",
            "repo_root": repo_root,
            "commit_sha": "no-head"
        },
        "title": "Atlas project",
        "summary": "Repo-wide summary for tests",
        "description": "Bounded project overview body",
        "responsibilities": ["index code", "answer context"],
        "tags": ["project"],
        "confidence": 0.8,
        "source_kind": "manual"
    })
}

#[test]
fn overview_ingest_show_search_and_refresh_plan_flow() {
    let (_dir, repo_root, db_path) = setup();

    let ingested = json_call(
        tool_overview_ingest,
        serde_json::json!({ "analysis": project_analysis(&repo_root) }),
        &repo_root,
        &db_path,
    );
    assert_eq!(ingested["summary"]["status"], "unverified");
    let overview_id = ingested["summary"]["overview_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let shown = json_call(
        tool_overview_show,
        serde_json::json!({ "subject": overview_id }),
        &repo_root,
        &db_path,
    );
    assert_eq!(shown["record"]["summary"], "Repo-wide summary for tests");

    let searched = json_call(
        tool_overview_search,
        serde_json::json!({ "query": "Repo-wide summary" }),
        &repo_root,
        &db_path,
    );
    assert_eq!(searched["summary"]["hit_count"], 1);

    let plan = json_call(
        tool_overview_refresh_plan,
        serde_json::json!({ "apply": false }),
        &repo_root,
        &db_path,
    );
    assert_eq!(plan["plan"]["apply"], false);
}

#[test]
fn overview_show_miss_returns_found_false_object() {
    let (_dir, repo_root, db_path) = setup();

    let shown = json_call(
        tool_overview_show,
        serde_json::json!({ "subject": "no-such-subject" }),
        &repo_root,
        &db_path,
    );

    assert_eq!(shown["found"], serde_json::json!(false));
    assert_eq!(shown["code"], serde_json::json!("subject_not_found"));
    assert_eq!(shown["subject"], serde_json::json!("no-such-subject"));
    assert!(shown["record"].is_null());
    assert_eq!(shown["matches"], serde_json::json!([]));
    assert_eq!(shown["warnings"].as_array().map(Vec::len), Some(1));
}

#[test]
fn overview_ingest_invalid_payload_returns_structured_invalid_input() {
    let (_dir, repo_root, db_path) = setup();

    let raw = tool_overview_ingest(
        Some(&serde_json::json!({ "analysis": { "schema_version": 1 } })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .expect("invalid ingest must still return a tool result envelope");

    assert_eq!(raw["isError"], serde_json::json!(true));
    assert_eq!(
        raw["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        raw["structuredContent"]["tool"],
        serde_json::json!("overview_ingest")
    );
    assert!(
        raw["structuredContent"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "structured error must carry a message"
    );
    assert!(
        raw["structuredContent"]["retry_guidance"]
            .as_str()
            .is_some_and(|guidance| guidance.contains("analysis")),
        "structured error must carry retry guidance"
    );
}

#[test]
fn overview_export_reports_max_tokens_clamp() {
    let (_dir, repo_root, db_path) = setup();

    let exported = json_call(
        tool_overview_export,
        serde_json::json!({ "scope": "project", "max_tokens": 200 }),
        &repo_root,
        &db_path,
    );

    assert_eq!(exported["summary"]["requested_max_tokens"], 200);
    assert_eq!(exported["summary"]["max_tokens"], 256);
    let warnings = exported["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .filter_map(|warning| warning.as_str())
            .any(|warning| warning.contains("clamped")),
        "clamped max_tokens must surface a warning, got: {warnings:?}"
    );
}

#[test]
fn overview_export_packet_ingest_validates_packet_id() {
    let (_dir, repo_root, db_path) = setup();

    let exported = json_call(
        tool_overview_export,
        serde_json::json!({ "scope": "project" }),
        &repo_root,
        &db_path,
    );
    let packets = exported["result"]["packets"].as_array().unwrap();
    assert_eq!(packets.len(), 1);
    let packet = &packets[0];
    let packet_id = packet["packet_id"].as_str().unwrap();

    let mut analysis = project_analysis(&repo_root);
    analysis["packet_id"] = serde_json::json!(packet_id);
    analysis["evidence_ids"] = packet["evidence_ids"].clone();
    analysis["subject"]["commit_sha"] = packet["commit_sha"].clone();
    analysis["source_kind"] = serde_json::json!("external_llm");

    let ingested = json_call(
        tool_overview_ingest,
        serde_json::json!({ "analysis": analysis }),
        &repo_root,
        &db_path,
    );
    assert_eq!(ingested["summary"]["packet_validated"], true);
    assert_eq!(ingested["summary"]["status"], "fresh");

    let mut mismatch = project_analysis(&repo_root);
    mismatch["packet_id"] = serde_json::json!("not-a-real-packet");
    let error = tool_overview_ingest(
        Some(&serde_json::json!({ "analysis": mismatch })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("packet_id mismatch"), "{error}");
}
