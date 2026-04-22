use super::*;
use rusqlite::Connection;

#[test]
fn status_healthy_repo_returns_ok() {
    let fixture = setup_mcp_fixture();
    let store = Store::open(&fixture.db_path).expect("open");
    store
        .finish_build(
            "/repo",
            atlas_store_sqlite::BuildFinishStats {
                files_discovered: 3,
                files_processed: 3,
                files_failed: 0,
                nodes_written: 3,
                edges_written: 2,
            },
        )
        .expect("finish_build");

    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("status", Some(&args), "/repo", &fixture.db_path).expect("status call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["error_code"].as_str(), Some("none"));
    assert!(v["message"].as_str().is_some());
    assert!(v["suggestions"].as_array().is_some());
    assert_eq!(v["build_state"].as_str(), Some("built"));
    assert!(v["node_count"].as_i64().unwrap_or(0) > 0);
}

#[test]
fn status_missing_db_returns_error_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir
        .path()
        .join("no_such_subdir")
        .join("atlas.db")
        .to_string_lossy()
        .to_string();

    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("status", Some(&args), "/repo", &missing).expect("status should not error");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["ok"].as_bool(), Some(false));
    assert_eq!(v["error_code"].as_str(), Some("missing_graph_db"));
    assert!(v["message"].as_str().is_some());
    assert!(!v["suggestions"].as_array().expect("suggestions").is_empty());
    assert_eq!(v["db_exists"].as_bool(), Some(false));
}

#[test]
fn status_build_failed_returns_error_code() {
    let fixture = setup_mcp_fixture();
    let store = Store::open(&fixture.db_path).expect("open");
    store.begin_build("/repo").expect("begin_build");
    store
        .fail_build("/repo", "parse error in src/main.rs")
        .expect("fail_build");

    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("status", Some(&args), "/repo", &fixture.db_path).expect("status call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["ok"].as_bool(), Some(false));
    assert_eq!(v["error_code"].as_str(), Some("failed_build"));
    assert!(v["message"].as_str().is_some());
    assert_eq!(v["build_state"].as_str(), Some("build_failed"));
}

#[test]
fn status_interrupted_build_returns_category() {
    let fixture = setup_mcp_fixture();
    let store = Store::open(&fixture.db_path).expect("open");
    store.begin_build("/repo").expect("begin_build");

    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("status", Some(&args), "/repo", &fixture.db_path).expect("status call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["ok"].as_bool(), Some(false));
    assert_eq!(v["error_code"].as_str(), Some("interrupted_build"));
    assert!(v["message"].as_str().is_some());
    assert_eq!(v["build_state"].as_str(), Some("building"));
}

#[test]
fn status_stale_graph_returns_error_code() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        fixture._dir.path(),
        "src/service.rs",
        "pub fn compute() -> i32 { 2 }\n",
    );

    let args = serde_json::json!({ "output_format": "json" });
    let resp =
        call("status", Some(&args), &fixture.repo_root, &fixture.db_path).expect("status call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["ok"].as_bool(), Some(false));
    assert_eq!(v["error_code"].as_str(), Some("stale_index"));
    assert_eq!(v["stale_index"].as_bool(), Some(true));
    assert_eq!(v["pending_graph_change_count"].as_u64(), Some(1));
    assert_eq!(
        v["pending_graph_changes"][0].as_str(),
        Some("src/service.rs")
    );
}

#[test]
fn status_schema_mismatch_returns_error_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let conn = Connection::open(&db_path).expect("open malformed db");
    conn.execute_batch(
        "
        CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        INSERT INTO metadata (key, value) VALUES ('schema_version', '6');
        CREATE TABLE files (
            path TEXT PRIMARY KEY,
            hash TEXT NOT NULL,
            language TEXT,
            indexed_at TEXT,
            node_count INTEGER
        );
        CREATE TABLE nodes (
            id INTEGER PRIMARY KEY,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            qualified_name TEXT NOT NULL,
            file_path TEXT NOT NULL,
            line_start INTEGER NOT NULL,
            line_end INTEGER NOT NULL,
            language TEXT,
            parent_name TEXT,
            params TEXT,
            return_type TEXT,
            modifiers TEXT,
            is_test INTEGER NOT NULL,
            file_hash TEXT NOT NULL,
            extra_json TEXT NOT NULL
        );
        CREATE TABLE edges (
            id INTEGER PRIMARY KEY,
            kind TEXT NOT NULL,
            source_qualified TEXT NOT NULL,
            target_qualified TEXT NOT NULL,
            file_path TEXT NOT NULL,
            line INTEGER,
            confidence REAL NOT NULL,
            confidence_tier TEXT,
            extra_json TEXT NOT NULL
        );
        CREATE TABLE graph_build_state (
            repo_root TEXT PRIMARY KEY,
            state TEXT NOT NULL
        );
        ",
    )
    .expect("seed malformed schema");

    let db_path = db_path.to_string_lossy().to_string();
    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("status", Some(&args), "/repo", &db_path).expect("status call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["ok"].as_bool(), Some(false));
    assert_eq!(v["error_code"].as_str(), Some("schema_mismatch"));
    assert!(
        v["graph_query_error"]
            .as_str()
            .is_some_and(|text| text.contains("graph_build_state"))
    );
}

#[test]
fn doctor_returns_checks_array() {
    let fixture = setup_mcp_fixture();
    let dir_path = fixture._dir.path().to_string_lossy().to_string();
    let args = serde_json::json!({ "output_format": "json" });

    let resp = call("doctor", Some(&args), &dir_path, &fixture.db_path).expect("doctor call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(v.get("ok").is_some());
    assert!(v["error_code"].as_str().is_some());
    assert!(v["message"].as_str().is_some());
    assert!(v["suggestions"].as_array().is_some());
    let checks = v["checks"].as_array().expect("checks must be an array");
    assert!(!checks.is_empty());
    for item in checks {
        assert!(item.get("check").is_some());
        assert!(item.get("ok").is_some());
        assert!(item.get("detail").is_some());
    }
    let db_item = checks.iter().find(|c| c["check"] == "db_file");
    assert!(db_item.is_some());
    assert_eq!(db_item.unwrap()["ok"].as_bool(), Some(true));
}

#[test]
fn doctor_missing_db_fails_db_check() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir
        .path()
        .join("no_such_subdir")
        .join("atlas.db")
        .to_string_lossy()
        .to_string();
    let dir_path = dir.path().to_string_lossy().to_string();
    let args = serde_json::json!({ "output_format": "json" });

    let resp = call("doctor", Some(&args), &dir_path, &missing).expect("doctor must not error");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["ok"].as_bool(), Some(false));
    assert_eq!(v["error_code"].as_str(), Some("checks_failed"));
    assert!(v["message"].as_str().is_some());
    let checks = v["checks"].as_array().expect("checks array");
    let db_file_item = checks.iter().find(|c| c["check"] == "db_file");
    assert!(db_file_item.is_some());
    assert_eq!(db_file_item.unwrap()["ok"].as_bool(), Some(false));
}

#[test]
fn doctor_retrieval_index_unavailable_sets_issue_code() {
    let fixture = setup_git_mcp_fixture();
    let content_db_path = atlas_engine::paths::content_db_path(&fixture.db_path);
    std::fs::remove_file(&content_db_path).expect("remove content db");
    let args = serde_json::json!({ "output_format": "json" });

    let resp =
        call("doctor", Some(&args), &fixture.repo_root, &fixture.db_path).expect("doctor call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let checks = v["checks"].as_array().expect("checks array");
    let retrieval = checks
        .iter()
        .find(|item| item["check"] == "retrieval_index")
        .expect("retrieval check");
    assert_eq!(retrieval["ok"].as_bool(), Some(false));
    assert_eq!(
        retrieval["issue_code"].as_str(),
        Some("retrieval_index_unavailable")
    );
}

#[test]
fn db_check_healthy_returns_ok() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let resp = call("db_check", Some(&args), "/repo", &fixture.db_path).expect("db_check call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["error_code"].as_str(), Some("none"));
    assert!(v["message"].as_str().is_some());
    assert!(v["suggestions"].as_array().is_some());
    let issues = v["integrity_issues"]
        .as_array()
        .expect("integrity_issues array");
    assert_eq!(issues.len(), 0);
}

#[test]
fn db_check_on_path_in_missing_dir_returns_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bad_path = dir
        .path()
        .join("no_such_subdir")
        .join("atlas.db")
        .to_string_lossy()
        .to_string();

    let result = call("db_check", None, "/repo", &bad_path);
    assert!(result.is_err());
}

#[test]
fn debug_graph_returns_node_counts() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let resp =
        call("debug_graph", Some(&args), "/repo", &fixture.db_path).expect("debug_graph call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(v["nodes"].as_i64().unwrap_or(0) > 0);
    assert!(v["edges"].as_i64().unwrap_or(0) > 0);
    assert!(v["files"].as_i64().unwrap_or(0) > 0);
    assert!(v.get("edges_by_kind").is_some());
    assert!(v.get("top_files_by_node_count").is_some());
}

#[test]
fn status_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let resp = call("status", None, "/repo", &fixture.db_path).expect("status call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn doctor_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let dir_path = fixture._dir.path().to_string_lossy().to_string();
    let resp = call("doctor", None, &dir_path, &fixture.db_path).expect("doctor call");
    assert_provenance(&resp, &dir_path, &fixture.db_path);
}

#[test]
fn db_check_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let resp = call("db_check", None, "/repo", &fixture.db_path).expect("db_check call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn debug_graph_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let resp = call("debug_graph", None, "/repo", &fixture.db_path).expect("debug_graph call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}
