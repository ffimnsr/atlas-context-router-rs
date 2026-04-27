//! Optional HTTP + Server-Sent Events (SSE) transport for atlas-mcp.
//!
//! Enabled with the `http-transport` Cargo feature.
//!
//! ## Protocol
//!
//! Each MCP message is a standard JSON-RPC 2.0 object.  Three routes:
//!
//! * `POST /` — Submit a JSON-RPC request.  Non-tool methods return inline.
//!   `tools/call` is dispatched to a blocking thread; the result is returned
//!   inline **and** broadcast to every active SSE subscriber.
//! * `GET /sse` — Subscribe to the Server-Sent Events stream.  The server
//!   pushes `$/progress` notifications and final tool responses as
//!   `data: <json>\n\n` SSE events.
//! * `GET /health` — Unauthenticated liveness probe.  Returns `{"ok":true}`.
//!
//! ## Authentication
//!
//! When `ATLAS_HTTP_AUTH_TOKEN` is set every request to `/` and `/sse`
//! **must** carry `Authorization: Bearer <token>`.  Requests without a valid
//! token receive `401 Unauthorized`.
//!
//! ## Environment variables
//!
//! | Variable                 | Default           | Description                        |
//! |--------------------------|-------------------|------------------------------------|
//! | `ATLAS_HTTP_BIND`        | `127.0.0.1:7070`  | `host:port` to listen on           |
//! | `ATLAS_HTTP_AUTH_TOKEN`  | *(none)*           | Bearer token; omit to disable auth |

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result};
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::cors::{Any, CorsLayer};

use crate::tools;
use crate::tools::health::mark_server_started;
use crate::transport::MCP_PROTOCOL_VERSION;

const DEFAULT_BIND: &str = "127.0.0.1:7070";
/// Broadcast channel capacity.  Slow SSE consumers lose old events.
const SSE_BROADCAST_CAPACITY: usize = 256;

// ── Shared server state ──────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    repo_root: Arc<String>,
    db_path: Arc<String>,
    auth_token: Option<Arc<String>>,
    sse_tx: broadcast::Sender<String>,
}

// ── Entry points ─────────────────────────────────────────────────────────────

/// Run the HTTP+SSE MCP server with default options.
pub fn run_http_server(repo_root: &str, db_path: &str) -> Result<()> {
    run_http_server_with_options(repo_root, db_path, crate::transport::ServerOptions::default())
}

/// Run the HTTP+SSE MCP server with explicit options.
pub fn run_http_server_with_options(
    repo_root: &str,
    db_path: &str,
    _options: crate::transport::ServerOptions,
) -> Result<()> {
    mark_server_started();

    let bind_addr: SocketAddr = std::env::var("ATLAS_HTTP_BIND")
        .unwrap_or_else(|_| DEFAULT_BIND.to_owned())
        .parse()
        .context("ATLAS_HTTP_BIND must be a valid socket address (e.g. 127.0.0.1:7070)")?;

    let auth_token = std::env::var("ATLAS_HTTP_AUTH_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .map(|t| Arc::new(t));

    if auth_token.is_none() {
        eprintln!(
            "atlas-mcp[http]: WARNING — no auth token configured; \
             set ATLAS_HTTP_AUTH_TOKEN for production use"
        );
    }

    let (sse_tx, _) = broadcast::channel::<String>(SSE_BROADCAST_CAPACITY);

    let state = AppState {
        repo_root: Arc::new(repo_root.to_owned()),
        db_path: Arc::new(db_path.to_owned()),
        auth_token,
        sse_tx,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(handle_health))
        .route("/", post(handle_jsonrpc))
        .route("/sse", get(handle_sse))
        .layer(cors)
        .with_state(state);

    eprintln!(
        "atlas-mcp[http]: listening on http://{bind_addr} (repo={repo_root}, db={db_path})"
    );

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("atlas-mcp:http-rt")
        .build()
        .context("cannot build tokio runtime for HTTP transport")?
        .block_on(async move {
            let listener = tokio::net::TcpListener::bind(bind_addr)
                .await
                .with_context(|| format!("cannot bind HTTP server to {bind_addr}"))?;
            axum::serve(listener, app)
                .await
                .context("HTTP server error")
        })
}

// ── Route handlers ────────────────────────────────────────────────────────────

/// `GET /health` — unauthenticated liveness probe.
async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// `GET /sse` — authenticated Server-Sent Events stream.
async fn handle_sse(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }

    let rx = state.sse_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| match item {
        Ok(line) => Some(Ok::<_, Infallible>(format!("data: {line}\n\n"))),
        Err(_) => None, // lagged: skip silently
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// `POST /` — authenticated JSON-RPC 2.0 endpoint.
async fn handle_jsonrpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }

    let request: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return jsonrpc_error_response(Value::Null, -32700, format!("parse error: {e}"));
        }
    };

    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_owned();
    let params = request.get("params").cloned();

    match method.as_str() {
        "initialize" => jsonrpc_ok_response(
            id,
            serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {},
                    "prompts": { "listChanged": false },
                    "logging": {},
                    "experimental": { "progressNotifications": true }
                },
                "serverInfo": {
                    "name": "atlas",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "initialized" | "notifications/initialized" => StatusCode::NO_CONTENT.into_response(),
        m if m.starts_with("notifications/") => StatusCode::NO_CONTENT.into_response(),
        "tools/list" => jsonrpc_ok_response(id, tools::tool_list()),
        "tools/call" => {
            let name = match params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            {
                Some(n) => n.to_owned(),
                None => return jsonrpc_error_response(id, -32602, "missing tool name".to_owned()),
            };
            let args = params.as_ref().and_then(|p| p.get("arguments")).cloned();
            let progress_token = params
                .as_ref()
                .and_then(|p| {
                    p.get("_meta")
                        .and_then(|m| m.get("progressToken"))
                        .or_else(|| p.get("progressToken"))
                })
                .cloned();

            dispatch_tool_call(state, id, name, args, progress_token).await
        }
        "prompts/list" => jsonrpc_ok_response(id, crate::prompts::prompt_list()),
        "prompts/get" => {
            let name = match params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            {
                Some(n) => n.to_owned(),
                None => {
                    return jsonrpc_error_response(id, -32602, "missing prompt name".to_owned())
                }
            };
            let prompt_args = params.as_ref().and_then(|p| p.get("arguments")).cloned();
            match crate::prompts::prompt_get(&name, prompt_args.as_ref()) {
                Ok(result) => jsonrpc_ok_response(id, result),
                Err(e) => jsonrpc_error_response(id, -32603, e.to_string()),
            }
        }
        other => jsonrpc_error_response(id, -32601, format!("method not found: {other}")),
    }
}

// ── Tool dispatch ─────────────────────────────────────────────────────────────

/// Dispatch a `tools/call` to a blocking thread.
///
/// Mid-execution progress notifications are pushed to the SSE broadcast
/// channel so subscribers see them in real time.  The final response is
/// returned inline **and** broadcast.
async fn dispatch_tool_call(
    state: AppState,
    id: Value,
    name: String,
    args: Option<Value>,
    progress_token: Option<Value>,
) -> Response {
    let repo_root = Arc::clone(&state.repo_root);
    let db_path = Arc::clone(&state.db_path);
    let sse_tx = state.sse_tx.clone();
    let sse_tx_for_progress = state.sse_tx.clone();
    let progress_token_for_closure = progress_token;
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_worker = Arc::clone(&cancel_flag);

    let result = tokio::task::spawn_blocking(move || {
        let sse = sse_tx_for_progress;
        let pt = progress_token_for_closure;

        crate::progress::install(
            move |msg, pct| {
                if let Some(token) = &pt {
                    let value = serde_json::json!({
                        "kind": "report",
                        "message": match pct {
                            Some(p) => format!("{msg} ({p}%)"),
                            None => msg.to_owned(),
                        },
                    });
                    let notification = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "$/progress",
                        "params": { "token": token, "value": value },
                    });
                    let _ = sse.send(notification.to_string());
                }
            },
            cancel_flag_worker,
        );

        let call_result = tools::call(&name, args.as_ref(), &repo_root, &db_path);
        crate::progress::uninstall();
        call_result.map_err(|e| e.to_string())
    })
    .await;

    match result {
        Ok(Ok(tool_result)) => {
            let rsp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": &id,
                "result": &tool_result,
            });
            let _ = sse_tx.send(rsp.to_string());
            jsonrpc_ok_response(id, tool_result)
        }
        Ok(Err(err_msg)) => {
            let rsp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": &id,
                "error": { "code": -32001, "message": &err_msg,
                    "data": { "atlas_error_code": "tool_execution_failed" } }
            });
            let _ = sse_tx.send(rsp.to_string());
            jsonrpc_error_response(id, -32001, err_msg)
        }
        Err(join_err) => {
            jsonrpc_error_response(id, -32603, format!("worker panicked: {join_err}"))
        }
    }
}

// ── Authentication ─────────────────────────────────────────────────────────────

/// Verify `Authorization: Bearer <token>` using a timing-safe comparison.
fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let Some(expected) = &state.auth_token else {
        return Ok(());
    };

    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    // Constant-time comparison: accumulate XOR differences.
    // Short-circuit only on length mismatch (not on content).
    let expected_bytes = expected.as_bytes();
    let provided_bytes = provided.as_bytes();
    let lengths_equal = expected_bytes.len() == provided_bytes.len();
    let diff: u8 = expected_bytes
        .iter()
        .zip(provided_bytes.iter().chain(std::iter::repeat(&0u8)))
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));

    if lengths_equal && diff == 0 {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "unauthorized",
                "message": "valid Bearer token required",
            })),
        )
            .into_response())
    }
}

// ── JSON-RPC response helpers ─────────────────────────────────────────────────

fn jsonrpc_ok_response(id: Value, result: Value) -> Response {
    Json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .into_response()
}

fn jsonrpc_error_response(id: Value, code: i32, message: String) -> Response {
    let status = match code {
        -32700 | -32600 | -32601 | -32602 => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message,
                "data": { "atlas_error_code": "tool_execution_failed" }
            }
        })),
    )
        .into_response()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(token: Option<&str>) -> AppState {
        AppState {
            repo_root: Arc::new("repo".to_owned()),
            db_path: Arc::new("db".to_owned()),
            auth_token: token.map(|t| Arc::new(t.to_owned())),
            sse_tx: broadcast::channel(1).0,
        }
    }

    #[test]
    fn auth_passes_when_no_token_configured() {
        let state = make_state(None);
        assert!(check_auth(&state, &HeaderMap::new()).is_ok());
    }

    #[test]
    fn auth_rejects_missing_header() {
        let state = make_state(Some("secret"));
        assert!(check_auth(&state, &HeaderMap::new()).is_err());
    }

    #[test]
    fn auth_rejects_wrong_token() {
        let state = make_state(Some("correct"));
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer wrong".parse().unwrap());
        assert!(check_auth(&state, &headers).is_err());
    }

    #[test]
    fn auth_passes_with_correct_token() {
        let state = make_state(Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(check_auth(&state, &headers).is_ok());
    }

    #[test]
    fn auth_rejects_prefix_match() {
        let state = make_state(Some("secret-long"));
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
        assert!(check_auth(&state, &headers).is_err());
    }
}
