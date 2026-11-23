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
mod determinism_support;
mod docs_section;
mod feedback;
mod health;
mod health_doctor;
mod history;
mod hooks;
mod insights;
mod listing;
mod man;
mod memory;
mod memory_fixtures;
mod memory_layer_cli;
mod memory_snapshots;
mod metrics;
mod postprocess;
mod query;
mod readiness;
mod review;
mod serve;
mod session;
mod snapshots;
mod trace;
mod version;
mod wake_up;
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
    let data = normalize_mcp_request_stream(requests);
    let expected_ids = jsonrpc_request_ids(&data);
    assert!(
        !expected_ids.is_empty(),
        "serve session must include at least one JSON-RPC request"
    );

    let mut child = spawn_serve_child(repo_root, args);

    // Write every request, then keep stdin open until all responses arrive:
    // closing the write half signals EOF, the daemon treats EOF as the end of
    // the session and cancels in-flight tool calls, so closing early races
    // heavy tools and drops their responses.
    let mut stdin = child.stdin.take().expect("serve stdin");
    stdin.write_all(&data).expect("write serve requests");
    stdin.flush().expect("flush serve requests");

    let stdout_rx = spawn_line_reader(child.stdout.take().expect("serve stdout"));
    let stderr_rx = spawn_line_reader(child.stderr.take().expect("serve stderr"));

    // Bounded wait for the exact observable condition: every requested id
    // answered. Panics carry the missing ids plus broker stderr for triage.
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut stdout_lines: Vec<String> = Vec::new();
    let mut seen_ids: Vec<u64> = Vec::new();
    loop {
        if expected_ids.iter().all(|id| seen_ids.contains(id)) {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let stderr = drain_lines(&stderr_rx).join("\n");
            panic!(
                "serve responses incomplete: expected ids={expected_ids:?}, seen={seen_ids:?}; broker stderr:\n{stderr}"
            );
        }
        match stdout_rx.recv_timeout(remaining) {
            Ok(line) => {
                if let Ok(response) = serde_json::from_str::<Value>(&line)
                    && let Some(id) = response["id"].as_u64()
                    && !seen_ids.contains(&id)
                {
                    seen_ids.push(id);
                }
                stdout_lines.push(line);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    drop(stdin);

    // Drain broker-owned lines until its relay exits so output stays complete.
    loop {
        match stdout_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => stdout_lines.push(line),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => break,
        }
    }
    let status = child.wait().expect("wait for atlas serve");
    let stderr_lines = drain_lines(&stderr_rx);

    let stdout = stdout_lines.join("\n");
    let mut stdout_bytes = stdout.into_bytes();
    if !stdout_bytes.is_empty() {
        stdout_bytes.push(b'\n');
    }
    let stderr = stderr_lines.join("\n");
    let mut stderr_bytes = stderr.into_bytes();
    if !stderr_bytes.is_empty() {
        stderr_bytes.push(b'\n');
    }
    Output {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
    }
}

fn jsonrpc_request_ids(requests: &[u8]) -> Vec<u64> {
    String::from_utf8(requests.to_vec())
        .expect("utf8 request stream")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let value: Value = serde_json::from_str(line).expect("parse request line");
            value["id"].as_u64()
        })
        .collect()
}

pub(super) fn read_json_tool_result(output: &Output, id: u64) -> Value {
    let responses = parse_jsonrpc_lines(&output.stdout);
    let response = responses
        .iter()
        .find(|response| response["id"] == json!(id))
        .cloned()
        .unwrap_or_else(|| {
            let seen_ids: Vec<u64> = responses
                .iter()
                .filter_map(|response| response["id"].as_u64())
                .collect();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr_tail: String = stderr
                .chars()
                .rev()
                .take(2_000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            panic!(
                "missing JSON-RPC response id={id}; seen response ids={seen_ids:?}; broker stderr tail:\n{stderr_tail}"
            );
        });

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
        format!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"query_graph\",\"arguments\":{{\"text\":\"greet_twice\"}},{meta}}}}}\n"),
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"save_context_artifact\",\"arguments\":{{\"label\":\"broker-artifact\",\"content\":\"{}\",\"content_type\":\"text/plain\"}},{}}}}}\n",
            artifact,
            meta
        ),
        format!("{{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_session_status\",\"arguments\":{{}},{meta}}}}}\n"),
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
