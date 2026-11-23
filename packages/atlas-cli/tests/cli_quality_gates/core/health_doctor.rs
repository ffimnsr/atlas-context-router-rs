use super::*;

#[test]
fn doctor_reports_mcp_serve_config() {
    let repo = setup_fixture_repo();

    // Standard profile keeps MCP worker expectations machine-independent.
    run_atlas(repo.path(), &["init", "--profile", "standard"]);
    run_atlas(repo.path(), &["build"]);

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    let doctor = read_json_data_output("doctor", output);
    let checks = doctor["checks"].as_array().expect("doctor checks array");
    let mcp_check = checks
        .iter()
        .find(|item| item["check"] == json!("mcp_serve_config"))
        .expect("mcp serve config check present");

    assert_eq!(mcp_check["ok"], json!(true));
    assert_eq!(mcp_check["detail"], json!("workers=2 timeout_ms=300000"));
}

#[test]
fn doctor_reports_retrieval_index_unavailable_issue_code() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let content_db = repo.path().join(".atlas").join("context.db");
    fs::remove_file(&content_db).expect("remove content db");

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    assert!(
        !output.status.success(),
        "doctor should fail when retrieval index is unavailable"
    );

    let doctor = read_json_data_output("doctor", output);
    let retrieval = doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|item| item["check"] == json!("retrieval_index"))
        .expect("retrieval index check present");

    assert_eq!(doctor["error_code"], json!("checks_failed"));
    assert_eq!(retrieval["ok"], json!(false));
    assert_eq!(
        retrieval["issue_code"],
        json!("retrieval_index_unavailable")
    );
}

#[test]
fn doctor_fails_clearly_on_invalid_memory_decay_config() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        "[memory.decay]\nlow_days = 0\n",
    )
    .expect("write invalid config");

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    assert!(
        !output.status.success(),
        "doctor must fail on invalid memory.decay retention days"
    );

    let doctor = read_json_data_output("doctor", output);
    let config_check = doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|item| item["check"] == json!("mcp_serve_config"))
        .expect("config check present");
    assert_eq!(config_check["ok"], json!(false));
    let detail = config_check["detail"].as_str().expect("detail");
    assert!(
        detail.contains("memory.decay.low_days"),
        "doctor must name the offending field: {detail}"
    );
}

#[test]
fn doctor_reports_noncanonical_content_path_identity() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let context_db = repo.path().join(".atlas").join("context.db");
    let conn = Connection::open(&context_db).expect("open context db");
    conn.execute(
        "INSERT INTO sources (
             id, session_id, source_type, label, repo_root, identity_kind, identity_value, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "bad-source",
            "sess1",
            "review_context",
            "bad source",
            repo.path().to_string_lossy().to_string(),
            "repo_path",
            "./src/lib.rs",
            "2025-01-01T00:00:00Z"
        ],
    )
    .expect("seed noncanonical source row");

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    assert!(
        !output.status.success(),
        "doctor should fail when content path identity is noncanonical"
    );

    let doctor = read_json_data_output("doctor", output);
    let content_check = doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|item| item["check"] == json!("path_canonicality"))
        .expect("content path identity check present");

    assert_eq!(content_check["ok"], json!(false));
    assert_eq!(content_check["issue_code"], json!("noncanonical_path_rows"));
    assert!(
        content_check["detail"]
            .as_str()
            .is_some_and(|text| text.contains("canonical=src/lib.rs"))
    );
}

#[test]
fn status_and_doctor_report_stale_parser_fingerprint() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["parser_fingerprint"]["state"], json!("current"));

    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        "[[parsers.external]]\nextractor = \"php\"\ngrammar_dir = \"grammars/tree-sitter-php\"\n",
    )
    .expect("write profile config");

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["parser_fingerprint"]["state"], json!("stale"));

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    assert!(
        !output.status.success(),
        "doctor should fail on a stale parser fingerprint"
    );
    let doctor = read_json_data_output("doctor", output);
    let check = doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|item| item["check"] == json!("parser_fingerprint"))
        .expect("parser fingerprint check present");
    assert_eq!(check["ok"], json!(false));
    assert_eq!(check["issue_code"], json!("parser_fingerprint_stale"));
}

#[test]
fn status_and_doctor_report_missing_parser_fingerprint() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let conn = Connection::open(repo.path().join(".atlas").join("worldtree.db"))
        .expect("open graph db");
    conn.execute(
        "DELETE FROM metadata WHERE key LIKE 'atlas_parser_fingerprint:%'",
        [],
    )
    .expect("delete parser fingerprint");

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["parser_fingerprint"]["state"], json!("missing"));

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    assert!(
        !output.status.success(),
        "doctor should fail on a missing parser fingerprint"
    );
    let doctor = read_json_data_output("doctor", output);
    let check = doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|item| item["check"] == json!("parser_fingerprint"))
        .expect("parser fingerprint check present");
    assert_eq!(check["ok"], json!(false));
    assert_eq!(check["issue_code"], json!("parser_fingerprint_missing"));
}

#[test]
fn doctor_warns_on_tracked_offloaded_language_without_parser() {
    let repo = setup_repo(&[
        ("src/app.php", "<?php\nclass App {}\n"),
        ("src/main.rs", "fn main() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    assert!(
        !output.status.success(),
        "doctor should fail when a tracked offloaded language has no parser"
    );

    let doctor = read_json_data_output("doctor", output);
    let check = doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|item| item["check"] == json!("offloaded_language_support"))
        .expect("offloaded language check present");

    assert_eq!(check["ok"], json!(false));
    assert_eq!(
        check["issue_code"],
        json!("unregistered_offloaded_language")
    );
    assert!(
        check["detail"]
            .as_str()
            .is_some_and(|text| text.contains(".php (1)")),
        "unexpected detail: {}",
        check["detail"]
    );
}

