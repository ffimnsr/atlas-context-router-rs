use super::*;
use atlas_adapters::normalize_event;
use atlas_session::{SessionEventType, SessionId, SessionStore};
use rusqlite::Connection;
use serde_json::Value;
use std::ffi::OsString;
use std::io::Write;
use std::process::{Output, Stdio};

mod analysis;
mod contracts;
mod determinism;
mod docs_section;
mod health;
mod history;
mod hooks;
mod insights;
mod man;
mod postprocess;
mod query;
mod readiness;
mod review;
mod serve;
mod session;
mod snapshots;
mod version;
mod worktree;

fn run_installed_hook(repo_root: &Path, frontend: &str, event: &str, payload: &str) {
    let runner = repo_root.join(".atlas").join("hooks").join("atlas-hook");
    let atlas_bin = Path::new(env!("CARGO_BIN_EXE_atlas"));
    let mut path_value = OsString::from(atlas_bin.parent().expect("atlas binary dir"));
    if let Some(existing_path) = std::env::var_os("PATH") {
        path_value.push(":");
        path_value.push(existing_path);
    }

    let mut child = sanitized_command(runner.to_str().expect("runner path"))
        .args([frontend, event])
        .current_dir(repo_root)
        .env("PATH", path_value)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn installed hook runner");

    child
        .stdin
        .as_mut()
        .expect("runner stdin")
        .write_all(payload.as_bytes())
        .expect("write hook payload");

    let output = child
        .wait_with_output()
        .expect("wait for installed hook runner");
    assert!(
        output.status.success(),
        "installed hook runner failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn spawn_serve_child(repo_root: &Path, args: &[&str]) -> std::process::Child {
    sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(args)
        .current_dir(repo_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn atlas serve {:?}: {err}", args))
}

fn maybe_attach_request_meta(mut value: Value) -> Value {
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if matches!(
        method,
        "initialize" | "server/discover" | "initialized" | "notifications/initialized"
    ) || method.starts_with("notifications/")
    {
        return value;
    }
    let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) else {
        return value;
    };
    params.entry("_meta".to_owned()).or_insert_with(|| {
        json!({
            atlas_mcp::spec::META_PROTOCOL_VERSION: atlas_mcp::MCP_PROTOCOL_VERSION,
            atlas_mcp::spec::META_CLIENT_CAPABILITIES: {
                "elicitation": { "form": {}, "url": {} }
            },
            atlas_mcp::spec::META_CLIENT_INFO: { "name": "zed", "version": "1.0.0" }
        })
    });
    value
}

fn normalize_mcp_request_stream(requests: impl AsRef<[u8]>) -> Vec<u8> {
    let input = String::from_utf8(requests.as_ref().to_vec()).expect("utf8 request stream");
    let mut output = String::new();
    for line in input.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).expect("parse request line");
        let value = maybe_attach_request_meta(value);
        output.push_str(&serde_json::to_string(&value).expect("serialize request line"));
        output.push('\n');
    }
    output.into_bytes()
}

fn run_serve_jsonrpc_session(
    repo_root: &Path,
    args: &[&str],
    requests: impl AsRef<[u8]>,
) -> Output {
    let mut child = spawn_serve_child(repo_root, args);
    // Write stdin in a separate thread so the write end of the pipe stays open
    // until after the write completes. On macOS, closing the write end inside
    // wait_with_output (which happens before the broker relay loop has a chance
    // to read) causes the relay to see EOF before any data arrives. Keeping
    // stdin open in the writer thread until write_all returns avoids that race.
    let data = normalize_mcp_request_stream(requests);
    let mut stdin = child.stdin.take().expect("serve stdin");
    let writer = std::thread::spawn(move || {
        stdin.write_all(&data).expect("write serve requests");
        // stdin (the write end) is explicitly dropped here, after the write
        // completes, signaling EOF to the broker relay loop.
    });
    // wait_with_output will not try to close stdin since child.stdin is None.
    let output = child
        .wait_with_output()
        .expect("wait for atlas serve output");
    let _ = writer.join();
    output
}

pub(super) fn read_json_tool_result(output: &Output, id: u64) -> Value {
    let response = parse_jsonrpc_lines(&output.stdout)
        .into_iter()
        .find(|response| response["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing JSON-RPC response id={id}"));
    assert_eq!(
        response["result"]["_meta"]["atlas:outputFormat"],
        json!("json")
    );
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result content text");
    serde_json::from_str(text).expect("tool result JSON payload")
}

pub(super) fn initialize_request_line(id: u64) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{}\",\"capabilities\":{{\"roots\":{{\"listChanged\":true}},\"sampling\":{{}},\"elicitation\":{{\"form\":{{}},\"url\":{{}}}}}},\"clientInfo\":{{\"name\":\"zed\",\"version\":\"1.0.0\"}},\"_meta\":{{\"clientTag\":\"quality-gate\"}}}}}}\n",
        id,
        atlas_mcp::MCP_PROTOCOL_VERSION
    )
}

pub(super) fn initialized_notification_line() -> &'static str {
    "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n"
}

pub(super) fn initialized_session_prelude(id: u64) -> String {
    format!(
        "{}{}",
        initialize_request_line(id),
        initialized_notification_line()
    )
}

fn request_meta_json() -> String {
    format!(
        "\"_meta\":{{\"{}\":\"{}\",\"{}\":{{\"elicitation\":{{\"form\":{{}},\"url\":{{}}}}}},\"{}\":{{\"name\":\"zed\",\"version\":\"1.0.0\"}}}}",
        atlas_mcp::spec::META_PROTOCOL_VERSION,
        atlas_mcp::MCP_PROTOCOL_VERSION,
        atlas_mcp::spec::META_CLIENT_CAPABILITIES,
        atlas_mcp::spec::META_CLIENT_INFO,
    )
}

fn serve_requests() -> String {
    let meta = request_meta_json();
    [
        initialize_request_line(1),
        initialized_notification_line().to_owned(),
        format!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{{{meta}}}}}\n"),
        format!("{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"query_graph\",\"arguments\":{{\"text\":\"greet_twice\"}},{meta}}}}}\n"),
        format!("{{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_context\",\"arguments\":{{\"target\":{{\"kind\":\"query\",\"query\":\"greet_twice\"}}}},{meta}}}}}\n"),
    ]
    .concat()
}

fn serve_requests_with_session_tools() -> String {
    let artifact = std::iter::repeat_n("broker artifact payload with safe spacing", 40)
        .collect::<Vec<_>>()
        .join(" ");
    let meta = request_meta_json();
    [
        initialize_request_line(1),
        initialized_notification_line().to_owned(),
        format!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"query_graph\",\"arguments\":{{\"text\":\"greet_twice\",\"output_format\":\"json\"}},{meta}}}}}\n"),
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"save_context_artifact\",\"arguments\":{{\"label\":\"broker-artifact\",\"content\":\"{}\",\"content_type\":\"text/plain\",\"output_format\":\"json\"}},{}}}}}\n",
            artifact,
            meta
        ),
        format!("{{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_session_status\",\"arguments\":{{\"output_format\":\"json\"}},{meta}}}}}\n"),
    ]
    .concat()
}

fn cleanup_mcp_daemons(repo_root: &Path) {
    for metadata in list_mcp_instance_metadata(repo_root) {
        if let Some(pid) = metadata["pid"].as_u64() {
            let pid = pid as u32;
            if pid_exists(pid) {
                kill_pid(pid);
                wait_until(Duration::from_secs(2), || !pid_exists(pid));
            }
        }
    }
}
