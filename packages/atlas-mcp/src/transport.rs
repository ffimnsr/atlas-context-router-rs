//! JSON-RPC 2.0 / MCP stdio transport loop.
//!
//! Reads newline-delimited JSON from stdin, dispatches each request, and
//! writes newline-delimited JSON responses to stdout.  Follows the MCP
//! 2024-11-05 protocol specification.

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
    let reader = std::io::BufReader::new(stdin.lock());
    let mut writer = std::io::BufWriter::new(stdout.lock());

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
