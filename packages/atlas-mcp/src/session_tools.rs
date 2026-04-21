//! CM7 — MCP session-continuity and saved-context tools.
//!
//! Implements the six new MCP tools that expose session identity, resume
//! snapshots, and content-store search/save/purge to agents.  Session event
//! emission for the four existing continuity tools is also handled here via
//! `emit_session_event_best_effort`.
//!
//! Design constraints (from Core Design Rules):
//! - Never store saved context in the graph database.
//! - Never block the primary tool result on session persistence failure.
//! - Return previews / pointers instead of raw large blobs.
//! - Restore context through retrieval, not transcript replay.

use anyhow::Result;
use atlas_adapters::bridge::{BRIDGE_DIR, bridge_file_count, purge_all_bridge_files};
use atlas_contentstore::{ContentStore, OutputRouting, SearchFilters, SourceMeta};
use atlas_session::{
    NewSessionEvent, ResumeSnapshot, SessionEventType, SessionId, SessionMeta, SessionStore,
};
use serde::Serialize;
use serde_json::Value;
use tracing::warn;

use crate::output::{OutputFormat, render_serializable};

// ---------------------------------------------------------------------------
// DB path helpers
// ---------------------------------------------------------------------------

/// Derive session DB path from the graph DB path.
///
/// Graph DB: `.atlas/worldtree.db` → Session DB: `.atlas/session.db`
pub(crate) fn derive_session_db_path(db_path: &str) -> String {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        parent.join("session.db").to_string_lossy().into_owned()
    } else {
        "session.db".to_string()
    }
}

/// Derive content DB path from the graph DB path.
///
/// Graph DB: `.atlas/worldtree.db` → Content DB: `.atlas/context.db`
pub(crate) fn derive_content_db_path(db_path: &str) -> String {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        parent.join("context.db").to_string_lossy().into_owned()
    } else {
        "context.db".to_string()
    }
}

/// Derive the bridge artifact directory from the graph DB path.
///
/// Graph DB: `.atlas/worldtree.db` → Bridge dir: `.atlas/bridge/`
pub(crate) fn derive_bridge_dir(db_path: &str) -> std::path::PathBuf {
    if let Some(parent) = std::path::Path::new(db_path).parent() {
        parent.join(BRIDGE_DIR)
    } else {
        std::path::PathBuf::from(BRIDGE_DIR)
    }
}

/// Derive the MCP session id for a given repo root.
///
/// Uses `worktree_id = ""` and `frontend = "mcp"` as stable anchors.
fn mcp_session_id(repo_root: &str) -> SessionId {
    SessionId::derive(repo_root, "", "mcp")
}

// ---------------------------------------------------------------------------
// CM7: best-effort session event emission for existing continuity tools
// ---------------------------------------------------------------------------

/// Emit a session event after a successful tool call. Called from the tool
/// dispatcher for the four continuity tools. Failures are logged and swallowed
/// — they must never block the primary tool result.
pub fn emit_session_event_best_effort(
    tool_name: &str,
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
) {
    let Some((event_type, payload)) = continuity_event_spec(tool_name, args) else {
        return;
    };

    let session_id = mcp_session_id(repo_root);
    let session_db = derive_session_db_path(db_path);

    let outcome: std::result::Result<(), Box<dyn std::error::Error>> = (|| {
        let mut store = SessionStore::open(&session_db)?;
        store.upsert_session_meta(session_id.clone(), repo_root, "mcp", None)?;
        store.append_event(NewSessionEvent {
            session_id,
            event_type,
            priority: 0,
            payload,
            created_at: None,
        })?;
        Ok(())
    })();

    if let Err(e) = outcome {
        warn!(tool = tool_name, err = %e, "session event emit failed (best-effort, ignored)");
    }
}

/// Map a tool name to the event type and payload it should emit, or `None` if
/// the tool is not a continuity tool.
fn continuity_event_spec(
    tool_name: &str,
    args: Option<&Value>,
) -> Option<(SessionEventType, Value)> {
    match tool_name {
        "query_graph" => {
            let text = args
                .and_then(|a| a.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some((
                SessionEventType::ContextRequest,
                serde_json::json!({"tool": "query_graph", "query": text}),
            ))
        }
        "get_impact_radius" => {
            let files = args
                .and_then(|a| a.get("files"))
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            Some((
                SessionEventType::ImpactAnalysis,
                serde_json::json!({"tool": "get_impact_radius", "files": files}),
            ))
        }
        "get_review_context" => {
            let files = args
                .and_then(|a| a.get("files"))
                .cloned()
                .unwrap_or(Value::Array(vec![]));
            Some((
                SessionEventType::ReviewContext,
                serde_json::json!({"tool": "get_review_context", "files": files}),
            ))
        }
        "detect_changes" => {
            let base = args
                .and_then(|a| a.get("base"))
                .cloned()
                .unwrap_or(Value::Null);
            let staged = args
                .and_then(|a| a.get("staged"))
                .cloned()
                .unwrap_or(Value::Bool(false));
            Some((
                SessionEventType::CommandRun,
                serde_json::json!({"tool": "detect_changes", "base": base, "staged": staged}),
            ))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// get_session_status
// ---------------------------------------------------------------------------

/// Return the status of the current (or specified) MCP session.
///
/// If no session exists yet for this repo, returns `"status": "no_session"`.
pub fn tool_get_session_status(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let session_id = resolve_session_id(args, repo_root);
    let session_db = derive_session_db_path(db_path);

    let store = match SessionStore::open(&session_db) {
        Ok(s) => s,
        Err(e) => {
            return tool_result_value(
                &serde_json::json!({
                    "session_id": session_id.as_str(),
                    "status": "no_session",
                    "error": e.to_string(),
                }),
                output_format,
            );
        }
    };

    let meta: Option<SessionMeta> = store.get_session_meta(&session_id)?;
    let event_count = store.list_events(&session_id)?.len();
    let snapshot = store.get_resume_snapshot(&session_id)?;

    let result = if let Some(m) = meta {
        let snap_consumed = snapshot.as_ref().map(|s| s.consumed);
        serde_json::json!({
            "session_id": m.session_id.as_str(),
            "status": "active",
            "repo_root": m.repo_root,
            "frontend": m.frontend,
            "worktree_id": m.worktree_id,
            "created_at": m.created_at,
            "updated_at": m.updated_at,
            "last_resume_at": m.last_resume_at,
            "event_count": event_count,
            "has_resume_snapshot": snapshot.is_some(),
            "snapshot_consumed": snap_consumed,
        })
    } else {
        serde_json::json!({
            "session_id": session_id.as_str(),
            "status": "no_session",
            "event_count": 0,
            "has_resume_snapshot": false,
        })
    };

    tool_result_value(&result, output_format)
}

// ---------------------------------------------------------------------------
// resume_session
// ---------------------------------------------------------------------------

/// Return the resume snapshot for the current (or specified) session.
///
/// Builds a snapshot on demand if one does not exist yet.  Marks the snapshot
/// consumed by default so agents do not receive stale context on subsequent
/// calls.
pub fn tool_resume_session(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let session_id = resolve_session_id(args, repo_root);
    let mark_consumed = args
        .and_then(|a| a.get("mark_consumed"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let session_db = derive_session_db_path(db_path);
    let mut store = SessionStore::open(&session_db)?;

    // Ensure session meta exists before building snapshot.
    store.upsert_session_meta(session_id.clone(), repo_root, "mcp", None)?;

    let snapshot: ResumeSnapshot = match store.get_resume_snapshot(&session_id)? {
        Some(s) => s,
        None => store.build_resume(&session_id)?,
    };

    if mark_consumed {
        let _ = store.mark_resume_consumed(&session_id, true);
    }

    // Record the resume event (best-effort, failure ignored).
    let _ = store.append_event(NewSessionEvent {
        session_id: session_id.clone(),
        event_type: SessionEventType::SessionResume,
        priority: 1,
        payload: serde_json::json!({"tool": "resume_session"}),
        created_at: None,
    });

    let result = serde_json::json!({
        "session_id": snapshot.session_id.as_str(),
        "snapshot": snapshot.snapshot,
        "event_count": snapshot.event_count,
        "consumed": mark_consumed,
        "created_at": snapshot.created_at,
    });

    tool_result_value(&result, output_format)
}

// ---------------------------------------------------------------------------
// search_saved_context
// ---------------------------------------------------------------------------

/// Search saved artifacts in the content store using BM25 + trigram fallback.
///
/// Returns previews (first 256 chars) instead of full blobs.  Use the
/// returned `source_id` with subsequent searches to narrow to one source.
pub fn tool_search_saved_context(
    args: Option<&Value>,
    _repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let query = args
        .and_then(|a| a.get("query"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: query"))?;
    let session_id_filter = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let source_type_filter = args
        .and_then(|a| a.get("source_type"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let limit = args
        .and_then(|a| a.get("limit"))
        .and_then(|v| v.as_u64())
        .unwrap_or(10) as usize;

    let content_db = derive_content_db_path(db_path);
    let mut cs = ContentStore::open(&content_db)?;
    let _ = cs.migrate();

    let filters = SearchFilters {
        session_id: session_id_filter,
        source_type: source_type_filter,
        repo_root: None,
    };

    let chunks = cs.search_with_fallback(query, &filters)?;

    #[derive(Serialize)]
    struct ChunkPreview {
        source_id: String,
        chunk_index: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// First 256 chars only — full content available via source_id.
        preview: String,
        content_type: String,
    }

    let results: Vec<ChunkPreview> = chunks
        .into_iter()
        .take(limit)
        .map(|c| ChunkPreview {
            source_id: c.source_id,
            chunk_index: c.chunk_index,
            title: c.title,
            preview: c.content.chars().take(256).collect(),
            content_type: c.content_type,
        })
        .collect();

    let total = results.len();
    tool_result_value(
        &serde_json::json!({"query": query, "results": results, "total": total}),
        output_format,
    )
}

// ---------------------------------------------------------------------------
// save_context_artifact
// ---------------------------------------------------------------------------

/// Index and store a large output in the content store.
///
/// Routing:
/// - Small (≤ 512 B) → returned raw, not indexed.
/// - Medium (≤ 4 KB) → indexed, preview returned.
/// - Large           → indexed, pointer (`source_id`) returned only.
///
/// The `source_id` is derived from `label` + content hash so identical
/// content is deduplicated automatically.
pub fn tool_save_context_artifact(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let content = args
        .and_then(|a| a.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: content"))?;
    let label = args
        .and_then(|a| a.get("label"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required argument: label"))?;
    let source_type = args
        .and_then(|a| a.get("source_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("mcp_artifact");
    let content_type = args
        .and_then(|a| a.get("content_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("text/plain");
    // Accept an explicit session_id or derive from repo_root.
    let session_id_str = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| mcp_session_id(repo_root).as_str().to_string());

    let source_id = generate_source_id(label, content);

    let content_db = derive_content_db_path(db_path);
    let mut cs = ContentStore::open(&content_db)?;
    let _ = cs.migrate();

    let meta = SourceMeta {
        id: source_id,
        session_id: Some(session_id_str),
        source_type: source_type.to_string(),
        label: label.to_string(),
        repo_root: Some(repo_root.to_string()),
    };

    let routing = cs.route_output(meta, content, content_type)?;

    let result = match routing {
        OutputRouting::Raw(raw) => serde_json::json!({
            "routing": "raw",
            "source_id": Value::Null,
            "content": raw,
        }),
        OutputRouting::Preview {
            source_id: sid,
            preview,
        } => serde_json::json!({
            "routing": "preview",
            "source_id": sid,
            "preview": preview,
        }),
        OutputRouting::Pointer { source_id: sid } => serde_json::json!({
            "routing": "pointer",
            "source_id": sid,
            "retrieval_hint": format!(
                "use search_saved_context to retrieve content for source_id={sid}"
            ),
        }),
    };

    tool_result_value(&result, output_format)
}

// ---------------------------------------------------------------------------
// get_context_stats
// ---------------------------------------------------------------------------

/// Return storage statistics for the current (or specified) session.
pub fn tool_get_context_stats(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let session_id = resolve_session_id(args, repo_root);
    let session_db = derive_session_db_path(db_path);
    let content_db = derive_content_db_path(db_path);
    let bridge_dir = derive_bridge_dir(db_path);

    // Session stats (best-effort — store may not exist for brand-new repos).
    let event_count = SessionStore::open(&session_db)
        .ok()
        .and_then(|s| s.list_events(&session_id).ok())
        .map(|e| e.len())
        .unwrap_or(0);

    // Content stats + retrieval index state (best-effort).
    let (source_count, chunk_count) = ContentStore::open(&content_db)
        .ok()
        .and_then(|mut cs| {
            let _ = cs.migrate();
            cs.stats(Some(session_id.as_str())).ok()
        })
        .unwrap_or((0, 0));

    // Retrieval index status for this repo (best-effort).
    let retrieval_index = ContentStore::open(&content_db)
        .ok()
        .and_then(|mut cs| {
            let _ = cs.migrate();
            cs.get_index_status(repo_root).ok().flatten()
        })
        .map(|s| {
            serde_json::json!({
                "state": s.state,
                "files_discovered": s.files_discovered,
                "files_indexed": s.files_indexed,
                "chunks_written": s.chunks_written,
                "chunks_reused": s.chunks_reused,
                "last_indexed_at": s.last_indexed_at,
                "last_error": s.last_error,
                "updated_at": s.updated_at,
                "searchable": s.state == atlas_contentstore::IndexState::Indexed,
            })
        });

    // Bridge artifact count.
    let bridge_file_pending = bridge_file_count(&bridge_dir);

    tool_result_value(
        &serde_json::json!({
            "session_id": session_id.as_str(),
            "event_count": event_count,
            "source_count": source_count,
            "chunk_count": chunk_count,
            "bridge_file_count": bridge_file_pending,
            "content_db_path": content_db,
            "session_db_path": session_db,
            "bridge_dir_path": bridge_dir.to_string_lossy(),
            "retrieval_index": retrieval_index,
        }),
        output_format,
    )
}

// ---------------------------------------------------------------------------
// purge_saved_context
// ---------------------------------------------------------------------------

/// Delete saved artifacts from the content store.
///
/// Supports two modes:
/// - `session_id` provided → delete all sources for that session.
/// - `session_id` omitted  → age-based cleanup: delete sources older than
///   `keep_days` days (default 30).
///
/// Pass `purge_bridge_files: true` to also delete pending bridge artifact
/// files from `.atlas/bridge/`.
pub fn tool_purge_saved_context(
    args: Option<&Value>,
    _repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let session_id_filter = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let keep_days = args
        .and_then(|a| a.get("keep_days"))
        .and_then(|v| v.as_u64())
        .unwrap_or(30) as u32;
    let purge_bridge = args
        .and_then(|a| a.get("purge_bridge_files"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let content_db = derive_content_db_path(db_path);
    let bridge_dir = derive_bridge_dir(db_path);

    let mut cs = ContentStore::open(&content_db)?;
    let _ = cs.migrate();

    let deleted = if let Some(sid) = session_id_filter {
        cs.delete_session_sources(&sid)?
    } else {
        cs.cleanup(keep_days)?
    };

    let deleted_bridge = if purge_bridge {
        purge_all_bridge_files(&bridge_dir)
    } else {
        0
    };

    tool_result_value(
        &serde_json::json!({
            "deleted_source_count": deleted,
            "deleted_bridge_file_count": deleted_bridge,
            "keep_days": keep_days,
        }),
        output_format,
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve session_id from args or derive it from repo_root.
fn resolve_session_id(args: Option<&Value>, repo_root: &str) -> SessionId {
    if let Some(sid) = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
    {
        SessionId(sid.to_string())
    } else {
        mcp_session_id(repo_root)
    }
}

/// Generate a stable source_id from label + content hash.
///
/// Uses SHA-256 over `label\0content` (first 64 KB sampled for speed).
/// Encoded as 32 lowercase hex chars (first 16 bytes of the digest).
fn generate_source_id(label: &str, content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(label.as_bytes());
    h.update(b"\x00");
    // Sample at most 64 KB to keep hashing fast for huge inputs.
    let sample = &content.as_bytes()[..content.len().min(65_536)];
    h.update(sample);
    let digest = h.finalize();
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Wrap structured output in an MCP tool-result content envelope.
pub(crate) fn tool_result_value<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<Value> {
    let rendered = render_serializable(value, output_format)?;
    let mut response = serde_json::json!({
        "content": [{
            "type": "text",
            "text": rendered.text,
            "mimeType": rendered.actual_format.mime_type(),
        }],
        "atlas_output_format": rendered.actual_format.as_str(),
        "atlas_requested_output_format": rendered.requested_format.as_str(),
    });
    if let Some(reason) = rendered.fallback_reason {
        response["atlas_fallback_reason"] = Value::String(reason);
    }
    Ok(response)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_db_path(dir: &TempDir) -> String {
        dir.path()
            .join(".atlas")
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn test_derive_session_db_path() {
        let p = derive_session_db_path("/repo/.atlas/worldtree.db");
        assert!(p.ends_with("session.db"), "got: {p}");
        assert!(p.contains(".atlas"), "got: {p}");
    }

    #[test]
    fn test_derive_content_db_path() {
        let p = derive_content_db_path("/repo/.atlas/worldtree.db");
        assert!(p.ends_with("context.db"), "got: {p}");
    }

    #[test]
    fn test_generate_source_id_deterministic() {
        let id1 = generate_source_id("my label", "some content");
        let id2 = generate_source_id("my label", "some content");
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 32);
    }

    #[test]
    fn test_generate_source_id_different_for_different_input() {
        let id1 = generate_source_id("label a", "content a");
        let id2 = generate_source_id("label b", "content b");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_get_session_status_no_session() {
        let dir = TempDir::new().unwrap();
        let db_path = setup_db_path(&dir);
        let result = tool_get_session_status(
            None,
            dir.path().to_str().unwrap(),
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let content = result["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(content).unwrap();
        // Session store doesn't exist yet — should return no_session.
        assert_eq!(body["status"].as_str().unwrap(), "no_session");
    }

    #[test]
    fn test_search_saved_context_missing_query() {
        let dir = TempDir::new().unwrap();
        let db_path = setup_db_path(&dir);
        let err = tool_search_saved_context(
            Some(&serde_json::json!({})),
            dir.path().to_str().unwrap(),
            &db_path,
            OutputFormat::Json,
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_save_and_search_artifact() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        // Save a small artifact (will be routed as raw — under DEFAULT_SMALL_OUTPUT_BYTES).
        let args = serde_json::json!({
            "content": "hello world",
            "label": "test artifact",
            "source_type": "test",
        });
        let result =
            tool_save_context_artifact(Some(&args), repo_root, &db_path, OutputFormat::Json)
                .unwrap();
        let content = result["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(content).unwrap();
        // Short content → raw routing.
        assert_eq!(body["routing"].as_str().unwrap(), "raw");
    }

    #[test]
    fn test_get_context_stats_empty() {
        let dir = TempDir::new().unwrap();
        let db_path = setup_db_path(&dir);
        let result = tool_get_context_stats(
            None,
            dir.path().to_str().unwrap(),
            &db_path,
            OutputFormat::Json,
        )
        .unwrap();
        let content = result["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(content).unwrap();
        assert_eq!(body["source_count"].as_u64().unwrap(), 0);
        assert_eq!(body["chunk_count"].as_u64().unwrap(), 0);
        assert_eq!(body["event_count"].as_u64().unwrap(), 0);
        // Bridge dir does not exist yet → count must be 0.
        assert_eq!(body["bridge_file_count"].as_u64().unwrap(), 0);
        assert!(body["bridge_dir_path"].is_string());
    }

    #[test]
    fn test_purge_saved_context_age_based() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        // Nothing to purge — should return 0 deleted.
        let args = serde_json::json!({"keep_days": 30});
        let result =
            tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
        let content = result["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(content).unwrap();
        assert_eq!(body["deleted_source_count"].as_u64().unwrap(), 0);
        assert_eq!(body["deleted_bridge_file_count"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_purge_saved_context_purges_bridge_files() {
        use atlas_adapters::bridge::{BridgeEvent, write_bridge_file};
        use atlas_session::SessionId;

        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let db_path = setup_db_path(&dir);
        let repo_root = dir.path().to_str().unwrap();

        // Write two bridge files so there is something to purge.
        let bridge_dir = derive_bridge_dir(&db_path);
        let sid = SessionId::derive(repo_root, "", "mcp");
        let ev = BridgeEvent {
            event_type: "COMMAND_RUN".to_string(),
            priority: 0,
            payload_json: r#"{"command":"atlas build"}"#.to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        write_bridge_file(&bridge_dir, &sid, "mcp", std::slice::from_ref(&ev)).unwrap();
        write_bridge_file(&bridge_dir, &sid, "mcp", &[ev]).unwrap();

        let args = serde_json::json!({"purge_bridge_files": true, "keep_days": 30});
        let result =
            tool_purge_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
        let content = result["content"][0]["text"].as_str().unwrap();
        let body: Value = serde_json::from_str(content).unwrap();
        assert_eq!(body["deleted_bridge_file_count"].as_u64().unwrap(), 2);
    }

    #[test]
    fn test_derive_bridge_dir() {
        let dir = derive_bridge_dir("/repo/.atlas/worldtree.db");
        assert!(dir.ends_with("bridge"), "got: {}", dir.display());
        assert!(
            dir.to_string_lossy().contains(".atlas"),
            "must be inside .atlas"
        );
    }

    #[test]
    fn test_continuity_event_spec_known_tools() {
        let args = serde_json::json!({"text": "find foo"});
        let spec = continuity_event_spec("query_graph", Some(&args));
        assert!(spec.is_some());
        let (et, payload) = spec.unwrap();
        assert_eq!(et, SessionEventType::ContextRequest);
        assert_eq!(payload["query"].as_str().unwrap(), "find foo");
    }

    #[test]
    fn test_continuity_event_spec_unknown_tool() {
        let spec = continuity_event_spec("list_graph_stats", None);
        assert!(spec.is_none());
    }
}
