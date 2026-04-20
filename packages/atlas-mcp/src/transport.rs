//! JSON-RPC 2.0 / MCP stdio transport loop.
//!
//! Reads newline-delimited JSON from stdin, dispatches each request, and
//! writes newline-delimited JSON responses to stdout.  Follows the MCP
//! 2024-11-05 protocol specification.

use std::io::BufReader;
use std::io::{BufRead, Write};

use anyhow::{Context, Result};

use crate::tools;

/// Run the MCP server until stdin closes.
///
/// * `repo_root` – absolute path to the git repository root; used by tools
///   that need git context (e.g. `detect_changes`).
/// * `db_path`   – path to the Atlas SQLite database.
pub fn run_server(repo_root: &str, db_path: &str) -> Result<()> {
    eprintln!("atlas-mcp: server ready (repo={repo_root}, db={db_path})");
    eprintln!("atlas-mcp: reading JSON-RPC requests from stdin");

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = BufReader::new(stdin.lock());
    let mut writer = std::io::BufWriter::new(stdout.lock());

    run_server_io(reader, &mut writer, repo_root, db_path)
}

fn run_server_io<R: BufRead, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
) -> Result<()> {
    process_requests(reader, writer, repo_root, db_path)
}

fn process_requests<R: BufRead, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
) -> Result<()> {
    for line in reader.lines() {
        let line = line.context("stdin read error")?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                let resp =
                    jsonrpc_error(serde_json::Value::Null, -32700, format!("parse error: {e}"));
                writeln!(writer, "{resp}")?;
                writer.flush()?;
                continue;
            }
        };

        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = request.get("params");

        // Notifications — no response required.
        if method == "initialized" || method.starts_with("notifications/") {
            continue;
        }

        let response = match dispatch(method, params, repo_root, db_path) {
            Ok(result) => jsonrpc_ok(id, result),
            Err(e) => {
                tracing::warn!("MCP method '{method}' error: {e}");
                jsonrpc_error(id, -32000, e.to_string())
            }
        };

        writeln!(writer, "{response}")?;
        writer.flush()?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Method dispatch
// ---------------------------------------------------------------------------

fn dispatch(
    method: &str,
    params: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "atlas",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),

        "tools/list" => Ok(tools::tool_list()),

        "tools/call" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing tool name"))?;
            let args = params.and_then(|p| p.get("arguments"));
            tools::call(name, args, repo_root, db_path)
        }

        other => Err(anyhow::anyhow!("method not found: {other}")),
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC helpers
// ---------------------------------------------------------------------------

fn jsonrpc_ok(id: serde_json::Value, result: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn jsonrpc_error(id: serde_json::Value, code: i32, message: String) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    use atlas_core::EdgeKind;
    use atlas_core::kinds::NodeKind;
    use atlas_core::model::{Edge, Node, NodeId};
    use atlas_store_sqlite::Store;
    use std::io::Cursor;
    use tempfile::TempDir;

    struct TransportFixture {
        _dir: TempDir,
        db_path: String,
    }

    fn make_node(kind: NodeKind, name: &str, qualified_name: &str, file_path: &str) -> Node {
        Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            file_path: file_path.to_owned(),
            line_start: 1,
            line_end: 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("()".to_owned()),
            return_type: None,
            modifiers: Some("pub".to_owned()),
            is_test: kind == NodeKind::Test,
            file_hash: format!("hash:{file_path}"),
            extra_json: serde_json::json!({}),
        }
    }

    fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file_path: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: source_qn.to_owned(),
            target_qn: target_qn.to_owned(),
            file_path: file_path.to_owned(),
            line: Some(1),
            confidence: 1.0,
            confidence_tier: Some("high".to_owned()),
            extra_json: serde_json::json!({}),
        }
    }

    fn setup_fixture() -> TransportFixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");

        let compute = make_node(
            NodeKind::Function,
            "compute",
            "src/service.rs::fn::compute",
            "src/service.rs",
        );
        store
            .replace_file_graph(
                "src/service.rs",
                "hash:src/service.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&compute),
                &[],
            )
            .expect("replace service graph");

        let handle = make_node(
            NodeKind::Function,
            "handle_request",
            "src/api.rs::fn::handle_request",
            "src/api.rs",
        );
        let handle_calls_compute = make_edge(
            EdgeKind::Calls,
            "src/api.rs::fn::handle_request",
            "src/service.rs::fn::compute",
            "src/api.rs",
        );
        store
            .replace_file_graph(
                "src/api.rs",
                "hash:src/api.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&handle),
                &[handle_calls_compute],
            )
            .expect("replace api graph");

        TransportFixture { _dir: dir, db_path }
    }

    fn parse_output_lines(output: Vec<u8>) -> Vec<serde_json::Value> {
        String::from_utf8(output)
            .expect("utf8 output")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("jsonrpc response line"))
            .collect()
    }

    #[test]
    fn stdio_transport_handles_initialize_list_and_tool_calls() {
        let fixture = setup_fixture();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"compute\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"query\":\"compute\"}}}\n"
        );
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut writer = Vec::new();

        run_server_io(reader, &mut writer, "/ignored", &fixture.db_path).expect("run server io");

        let responses = parse_output_lines(writer);
        assert_eq!(
            responses.len(),
            4,
            "initialized notification must not emit a response"
        );

        assert_eq!(responses[0]["id"], 1);
        assert_eq!(responses[0]["result"]["protocolVersion"], "2024-11-05");

        assert_eq!(responses[1]["id"], 2);
        let tools = responses[1]["result"]["tools"]
            .as_array()
            .expect("tools/list result tools array");
        assert!(
            tools.iter().any(|tool| tool["name"] == "get_context"),
            "tools/list must expose get_context"
        );

        assert_eq!(responses[2]["id"], 3);
        assert_eq!(
            responses[2]["result"]["atlas_output_format"], "json",
            "query_graph transport response must preserve JSON default"
        );
        let query_text = responses[2]["result"]["content"][0]["text"]
            .as_str()
            .expect("query_graph text content");
        let query_value: serde_json::Value =
            serde_json::from_str(query_text).expect("query_graph payload json");
        assert_eq!(query_value[0]["qn"], "src/service.rs::fn::compute");

        assert_eq!(responses[3]["id"], 4);
        assert_eq!(
            responses[3]["result"]["atlas_output_format"], "toon",
            "get_context transport response must preserve TOON default"
        );
        let context_text = responses[3]["result"]["content"][0]["text"]
            .as_str()
            .expect("get_context text content");
        assert!(context_text.contains("intent: symbol"));
        assert!(context_text.contains("src/service.rs::fn::compute"));
    }

    #[test]
    fn stdio_transport_returns_jsonrpc_errors_for_parse_and_method_failures() {
        let fixture = setup_fixture();
        let input = concat!(
            "not-json\n",
            "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"missing/method\",\"params\":{}}\n"
        );
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut writer = Vec::new();

        run_server_io(reader, &mut writer, "/ignored", &fixture.db_path).expect("run server io");

        let responses = parse_output_lines(writer);
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert_eq!(responses[0]["id"], serde_json::Value::Null);
        assert_eq!(responses[1]["id"], 7);
        assert_eq!(responses[1]["error"]["code"], -32000);
        assert!(
            responses[1]["error"]["message"]
                .as_str()
                .expect("error message")
                .contains("method not found")
        );
    }
}
