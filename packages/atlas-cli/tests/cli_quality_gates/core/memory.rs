use super::*;

/// Stores a memory via the CLI and returns the JSON `data` payload.
fn store_memory(repo_root: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json", "memory", "store"];
    full.extend_from_slice(args);
    read_json_data_output("memory.store", run_atlas(repo_root, &full))
}

fn memory_id(payload: &Value) -> String {
    payload["memory"]["id"]
        .as_str()
        .expect("memory id string")
        .to_owned()
}

/// Calls an MCP tool over `atlas serve` and returns the parsed tool body.
fn mcp_call(repo_root: &Path, id: u64, name: &str, arguments: &str) -> Value {
    let request = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": serde_json::from_str::<Value>(arguments).expect("arguments json"),
        },
    }))
    .expect("serialize request");
    let requests = format!("{}{}\n", initialized_session_prelude(1), request);
    let output = run_serve_jsonrpc_session(repo_root, &["serve"], requests);
    read_json_tool_result(&output, id)
}

/// Backdates a stored memory row so decay math sees it as old.
fn backdate_memory(repo: &Path, id: &str, updated_at: &str) {
    rewrite_memory(repo, id, |record| {
        record.updated_at = updated_at.to_owned();
    });
}

/// Returns the canonical repo root the CLI stores memory ownership under.
fn memory_repo_root(repo: &Path) -> String {
    canonical_path(repo).to_string_lossy().into_owned()
}

/// Rewrites one markdown memory record through the truth store.
fn rewrite_memory(repo: &Path, id: &str, apply: impl FnOnce(&mut atlas_session::MemoryRecord)) {
    let session_db = repo.join(".atlas").join("session.db");
    let mut store = atlas_session::SessionStore::open(session_db.to_str().expect("db path"))
        .expect("open session store");
    let repo_root = memory_repo_root(repo);
    let mut record = store
        .get_memory(&repo_root, id)
        .expect("read memory")
        .expect("memory exists");
    apply(&mut record);
    store.rewrite_memory(&record).expect("rewrite memory");
}

mod consolidate;
mod curation;
mod mcp;
mod recall;
mod store;
