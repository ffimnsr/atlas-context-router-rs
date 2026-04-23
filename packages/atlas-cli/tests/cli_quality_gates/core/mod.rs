use super::*;
use atlas_adapters::normalize_event;
use atlas_session::{SessionEventType, SessionId, SessionStore};
use rusqlite::Connection;
use std::ffi::OsString;
use std::io::Write;
use std::process::{Output, Stdio};

mod analysis;
mod health;
mod hooks;
mod query;
mod review;
mod serve;
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

    let output = child.wait_with_output().expect("wait for installed hook runner");
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

fn run_serve_jsonrpc_session(repo_root: &Path, args: &[&str], requests: impl AsRef<[u8]>) -> Output {
    let mut child = spawn_serve_child(repo_root, args);
    child
        .stdin
        .as_mut()
        .expect("serve stdin")
        .write_all(requests.as_ref())
        .expect("write serve requests");
    child.wait_with_output().expect("wait for atlas serve output")
}

fn read_json_tool_result(output: &Output, id: u64) -> Value {
    let response = parse_jsonrpc_lines(&output.stdout)
        .into_iter()
        .find(|response| response["id"] == json!(id))
        .unwrap_or_else(|| panic!("missing JSON-RPC response id={id}"));
    assert_eq!(response["result"]["atlas_output_format"], json!("json"));
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result content text");
    serde_json::from_str(text).expect("tool result JSON payload")
}

fn serve_requests() -> &'static str {
    concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"greet_twice\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"query\":\"greet_twice\"}}}\n"
    )
}

fn serve_requests_with_session_tools() -> String {
    let artifact = "0123456789abcdefghijklmnopqrstuvwxyz".repeat(32);
    format!(
        concat!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{}}}}\n",
            "{{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{{}}}}\n",
            "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"query_graph\",\"arguments\":{{\"text\":\"greet_twice\",\"output_format\":\"json\"}}}}}}\n",
            "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"save_context_artifact\",\"arguments\":{{\"label\":\"broker-artifact\",\"content\":\"{}\",\"content_type\":\"text/plain\",\"output_format\":\"json\"}}}}}}\n",
            "{{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_session_status\",\"arguments\":{{\"output_format\":\"json\"}}}}}}\n"
        ),
        artifact
    )
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
