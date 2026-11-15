//! `feedback_record` MCP envelope tests.

use crate::output::OutputFormat;
use serde_json::json;
use tempfile::TempDir;

use super::feedback::tool_feedback_record;
use super::test_util::{setup_db_path, tool_body};

#[test]
fn feedback_record_reports_stored_true() {
    let dir = TempDir::new().unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let args = json!({
        "tool": "analyze_safety",
        "symbol": "src/lib.rs::fn::parse",
        "predicted": "safety score >= 0.5",
        "actual": "safety score 0.39",
        "correction": "recompute with fan_in 4"
    });

    let result =
        tool_feedback_record(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);

    assert_eq!(body["stored"].as_bool(), Some(true));
    assert_eq!(body["summary"]["stored"].as_bool(), Some(true));
    assert_eq!(body["tool"].as_str(), Some("feedback_record"));
    assert!(body["record"]["id"].is_string());
}
