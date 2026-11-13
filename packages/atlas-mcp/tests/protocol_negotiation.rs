//! Protocol lifecycle negotiation regression tests.
//!
//! MCP 2026-07-28 negotiates through `server/discover` and `_meta`, not the
//! `initialize` handshake. A client that sends `initialize` with the
//! 2026-07-28 version therefore receives the newest supported version that
//! still has an `initialize` handshake (2025-11-25).

use std::path::Path;
use std::process::Command;

use atlas_mcp::testing::run_stdio_jsonrpc_session_for_tests;
use atlas_mcp::{MCP_PROTOCOL_VERSION, ServerOptions};
use serde_json::{Value, json};
use tempfile::TempDir;

fn setup_repo() -> (TempDir, String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("README.md"),
        "# protocol fixture\n\nNegotiation regression fixture.\n",
    )
    .expect("write readme");
    run_git(dir.path(), &["init", "--quiet"]);
    run_git(dir.path(), &["config", "user.name", "Atlas Tests"]);
    run_git(
        dir.path(),
        &["config", "user.email", "atlas-tests@example.com"],
    );
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "--quiet", "-m", "fixture baseline"]);
    let repo_root = dir.path().to_string_lossy().into_owned();
    let db_path = dir
        .path()
        .join(".atlas")
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();
    (dir, repo_root, db_path)
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdio_session(repo_root: &str, db_path: &str, messages: &[Value]) -> Vec<Value> {
    let input = messages
        .iter()
        .map(|message| serde_json::to_string(message).expect("serialize message"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    run_stdio_jsonrpc_session_for_tests(&input, repo_root, db_path, ServerOptions::default())
        .expect("run stdio jsonrpc session")
}

fn initialize_request(id: u64, protocol_version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": protocol_version,
            "capabilities": {},
            "clientInfo": { "name": "protocol-test", "version": "1.0.0" }
        }
    })
}

#[test]
fn initialize_with_discover_lifecycle_version_falls_back_to_previous() {
    let (_dir, repo_root, db_path) = setup_repo();
    let responses = stdio_session(
        &repo_root,
        &db_path,
        &[initialize_request(1, MCP_PROTOCOL_VERSION)],
    );
    let response = responses
        .iter()
        .find(|value| value["id"] == json!(1))
        .expect("initialize response");

    assert_eq!(
        response["result"]["protocolVersion"],
        json!("2025-11-25"),
        "2026-07-28 has no initialize handshake, so initialize must fall back"
    );
    assert_eq!(response["result"]["serverInfo"]["name"], json!("atlas"));
}

#[test]
fn initialize_with_previous_version_is_echoed() {
    let (_dir, repo_root, db_path) = setup_repo();
    let responses = stdio_session(&repo_root, &db_path, &[initialize_request(1, "2025-11-25")]);
    let response = responses
        .iter()
        .find(|value| value["id"] == json!(1))
        .expect("initialize response");

    assert_eq!(response["result"]["protocolVersion"], json!("2025-11-25"));
}

#[test]
fn discover_advertises_both_supported_versions() {
    let (_dir, repo_root, db_path) = setup_repo();
    let responses = stdio_session(
        &repo_root,
        &db_path,
        &[
            initialize_request(1, MCP_PROTOCOL_VERSION),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "server/discover",
                "params": {
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL_VERSION,
                        "io.modelcontextprotocol/clientCapabilities": {},
                        "io.modelcontextprotocol/clientInfo": {
                            "name": "protocol-test",
                            "version": "1.0.0"
                        }
                    }
                }
            }),
        ],
    );
    let discover = responses
        .iter()
        .find(|value| value["id"] == json!(2))
        .expect("discover response");
    let versions = discover["result"]["supportedVersions"]
        .as_array()
        .unwrap_or_else(|| panic!("supportedVersions array: {discover:#}"));

    assert!(versions.contains(&json!(MCP_PROTOCOL_VERSION)));
    assert!(versions.contains(&json!("2025-11-25")));
}
