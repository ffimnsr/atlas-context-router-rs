use super::*;
use std::fs;
use std::path::Path;

fn read_golden_text(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name);
    fs::read_to_string(path).expect("golden text")
}

#[test]
fn man_text_output_matches_golden() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);

    let output = run_atlas(repo.path(), &["man", "mcp", "resolve_symbol"]);
    assert!(output.status.success(), "man text command failed");
    assert_eq!(
        stdout_text(&output),
        read_golden_text("man_resolve_symbol.txt"),
        "man text snapshot must stay stable"
    );
}

#[test]
fn man_json_output_matches_golden() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);

    let output = run_atlas(repo.path(), &["--json", "man", "mcp", "resolve_symbol"]);
    assert!(output.status.success(), "man json command failed");
    assert_eq!(
        stdout_text(&output),
        read_golden_text("man_resolve_symbol_stdout.json"),
        "man json snapshot must stay stable"
    );
}

#[test]
fn man_cli_and_mcp_payloads_match_for_resolve_symbol() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);

    let cli = read_json_data_output(
        "man",
        run_atlas(repo.path(), &["--json", "man", "mcp", "resolve_symbol"]),
    );

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"man\",\"arguments\":{\"namespace\":\"mcp\",\"tool_name\":\"resolve_symbol\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(output.status.success(), "serve man failed");

    let mcp = read_json_tool_result(&output, 2);
    assert_eq!(cli, mcp, "CLI and MCP manual payloads must stay aligned");

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn man_unknown_tool_error_mentions_deterministic_suggestion() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);

    let output = run_atlas_capture(repo.path(), &["man", "mcp", "query_grap"]);
    assert!(!output.status.success(), "unknown manual lookup must fail");

    let stderr = String::from_utf8(output.stderr).expect("stderr utf-8");
    assert!(
        stderr.contains("query_graph"),
        "stderr missing nearest suggestion: {stderr}"
    );
}
