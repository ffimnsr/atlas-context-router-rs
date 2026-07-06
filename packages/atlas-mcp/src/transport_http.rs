//! Streamable HTTP transport for atlas-mcp.
//!
//! Enabled with `http-transport` Cargo feature.
//!
//! Routes:
//! - `POST /mcp` — JSON-RPC request ingress. Successful `initialize` creates
//!   negotiated HTTP session and returns `Mcp-Session-Id` header.
//! - `GET /mcp` — per-session SSE polling/resume endpoint using `Last-Event-ID`.
//! - `DELETE /mcp` — explicit session termination.
//! - `GET /health` — unauthenticated liveness probe.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result};
use atlas_core::error_code_docs_ref;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

use crate::auth::{self, ProtectedResourceAuthPolicy};
use crate::http_sessions::{HttpSession, PollEventsError, SessionLookupError, SessionManager};
use crate::spec;
use crate::tools;
use crate::tools::health::mark_server_started;
use crate::{completion, logging, resources};

const DEFAULT_BIND: &str = "127.0.0.1:7070";
const MCP_SESSION_ID_HEADER: &str = "Mcp-Session-Id";
const MCP_PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";
const LAST_EVENT_ID_HEADER: &str = "Last-Event-ID";
const PROTECTED_RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

#[derive(Clone)]
struct AppState {
    repo_root: Arc<String>,
    db_path: Arc<String>,
    auth_policy: Option<Arc<ProtectedResourceAuthPolicy>>,
    allowed_origins: Arc<HashSet<String>>,
    sessions: SessionManager,
}

pub fn run_http_server(repo_root: &str, db_path: &str) -> Result<()> {
    run_http_server_with_options(
        repo_root,
        db_path,
        crate::transport::ServerOptions::default(),
    )
}

pub fn run_http_server_with_options(
    repo_root: &str,
    db_path: &str,
    options: crate::transport::ServerOptions,
) -> Result<()> {
    mark_server_started();

    let bind_addr: SocketAddr = std::env::var("ATLAS_HTTP_BIND")
        .unwrap_or_else(|_| DEFAULT_BIND.to_owned())
        .parse()
        .context("ATLAS_HTTP_BIND must be a valid socket address (e.g. 127.0.0.1:7070)")?;

    eprintln!("atlas-mcp[http]: listening on http://{bind_addr} (repo={repo_root}, db={db_path})");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("atlas-mcp:http-rt")
        .build()
        .context("cannot build tokio runtime for HTTP transport")?
        .block_on(async move {
            let auth_policy = match options.http_auth {
                Some(config) => Some(Arc::new(ProtectedResourceAuthPolicy::load(config).await?)),
                None => None,
            };
            let allowed_origins = Arc::new(
                auth_policy
                    .as_ref()
                    .map(|policy| policy.allowed_origins().iter().cloned().collect())
                    .unwrap_or_default(),
            );
            let state = AppState {
                repo_root: Arc::new(repo_root.to_owned()),
                db_path: Arc::new(db_path.to_owned()),
                auth_policy,
                allowed_origins,
                sessions: SessionManager::from_env(),
            };
            let app = build_router(state);
            let listener = tokio::net::TcpListener::bind(bind_addr)
                .await
                .with_context(|| format!("cannot bind HTTP server to {bind_addr}"))?;
            axum::serve(listener, app)
                .await
                .context("HTTP server error")
        })
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handle_health))
        .route(
            PROTECTED_RESOURCE_METADATA_PATH,
            get(handle_protected_resource_metadata),
        )
        .route(
            "/mcp",
            post(handle_post_mcp)
                .get(handle_get_mcp)
                .delete(handle_delete_mcp),
        )
        .with_state(state)
}

async fn handle_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn handle_protected_resource_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let origin = match validate_origin(&state, &headers) {
        Ok(origin) => origin,
        Err(response) => return *response,
    };
    let Some(auth_policy) = state.auth_policy.as_ref() else {
        return apply_origin_headers(StatusCode::NOT_FOUND.into_response(), origin.as_deref());
    };
    apply_origin_headers(
        Json(auth_policy.metadata_json()).into_response(),
        origin.as_deref(),
    )
}

async fn handle_post_mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = authorize_request(&state, &headers) {
        return response;
    }
    let origin = match validate_origin(&state, &headers) {
        Ok(origin) => origin,
        Err(response) => return *response,
    };

    let request: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return apply_origin_headers(
                jsonrpc_error_response(Value::Null, -32700, format!("parse error: {error}")),
                origin.as_deref(),
            );
        }
    };

    if request.is_array() {
        return apply_origin_headers(
            jsonrpc_error_response(
                Value::Null,
                -32600,
                "JSON-RPC batch requests are not supported".to_owned(),
            ),
            origin.as_deref(),
        );
    }

    if !request.is_object()
        || request.get("jsonrpc").and_then(|value| value.as_str()) != Some("2.0")
        || request
            .get("method")
            .and_then(|value| value.as_str())
            .is_none()
    {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        return apply_origin_headers(
            jsonrpc_error_response(
                id,
                -32600,
                "invalid request: expected jsonrpc='2.0' and string method".to_owned(),
            ),
            origin.as_deref(),
        );
    }

    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    let params = request.get("params").cloned();

    if method == "initialize" {
        let response = match spec::negotiate_initialize(params.as_ref()) {
            Ok(result) => {
                let client_info = params
                    .as_ref()
                    .and_then(|value| value.get("clientInfo"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let session = state
                    .sessions
                    .create_session(spec::MCP_PROTOCOL_VERSION, client_info);
                let mut response = jsonrpc_ok_response(id, result);
                response.headers_mut().insert(
                    MCP_SESSION_ID_HEADER,
                    HeaderValue::from_str(session.id())
                        .expect("generated session id must be valid header value"),
                );
                response.headers_mut().insert(
                    MCP_PROTOCOL_VERSION_HEADER,
                    HeaderValue::from_static(spec::MCP_PROTOCOL_VERSION),
                );
                response
            }
            Err(error) => jsonrpc_error_response(id, -32602, error.to_string()),
        };
        return apply_origin_headers(response, origin.as_deref());
    }

    if let Some(response) = validate_protocol_version_header(&headers) {
        return apply_origin_headers(response, origin.as_deref());
    }

    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return apply_origin_headers(*response, origin.as_deref()),
    };
    debug_assert_eq!(session.protocol_version(), spec::MCP_PROTOCOL_VERSION);
    let _ = session.client_info();

    let response = match method.as_str() {
        "initialized" | "notifications/initialized" => {
            session.mark_initialized();
            StatusCode::NO_CONTENT.into_response()
        }
        m if m.starts_with("notifications/") => StatusCode::NO_CONTENT.into_response(),
        "logging/setLevel" => dispatch_logging_set_level(session, id, params),
        "tools/list" => jsonrpc_ok_response(id, tools::tool_list()),
        "resources/list" => match resources::resources_list(params.as_ref()) {
            Ok(result) => jsonrpc_ok_response(id, result),
            Err(error) => jsonrpc_error_response(id, -32602, error.to_string()),
        },
        "resources/templates/list" => match resources::resources_templates_list(params.as_ref()) {
            Ok(result) => jsonrpc_ok_response(id, result),
            Err(error) => jsonrpc_error_response(id, -32602, error.to_string()),
        },
        "resources/read" => {
            match resources::resources_read(params.as_ref(), &state.repo_root, &state.db_path) {
                Ok(result) => jsonrpc_ok_response(id, result),
                Err(error) => jsonrpc_error_response(id, -32602, error.to_string()),
            }
        }
        "completion/complete" => match completion::complete(params.as_ref(), &state.repo_root) {
            Ok(result) => jsonrpc_ok_response(id, result),
            Err(error) => jsonrpc_error_response(id, -32602, error.to_string()),
        },
        "tools/call" => dispatch_tool_call(state, session, id, params).await,
        "prompts/list" => jsonrpc_ok_response(id, crate::prompts::prompt_list()),
        "prompts/get" => {
            let name = match params
                .as_ref()
                .and_then(|value| value.get("name"))
                .and_then(|value| value.as_str())
            {
                Some(name) => name.to_owned(),
                None => {
                    return apply_origin_headers(
                        jsonrpc_error_response(id, -32602, "missing prompt name".to_owned()),
                        origin.as_deref(),
                    );
                }
            };
            let prompt_args = params
                .as_ref()
                .and_then(|value| value.get("arguments"))
                .cloned();
            match crate::prompts::prompt_get(&name, prompt_args.as_ref()) {
                Ok(result) => jsonrpc_ok_response(id, result),
                Err(error) => jsonrpc_error_response(id, -32603, error.to_string()),
            }
        }
        other => jsonrpc_error_response(id, -32601, format!("method not found: {other}")),
    };
    apply_origin_headers(response, origin.as_deref())
}

async fn handle_get_mcp(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize_request(&state, &headers) {
        return response;
    }
    let origin = match validate_origin(&state, &headers) {
        Ok(origin) => origin,
        Err(response) => return *response,
    };
    if let Some(response) = validate_protocol_version_header(&headers) {
        return apply_origin_headers(response, origin.as_deref());
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return apply_origin_headers(*response, origin.as_deref()),
    };

    session.mark_stream_open();
    let last_event_id = headers
        .get(LAST_EVENT_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let response = match session
        .wait_for_events(last_event_id.as_deref(), state.sessions.poll_wait())
        .await
    {
        Ok(events) => sse_response(session.id(), events),
        Err(PollEventsError::ResumeWindowExpired) => protocol_error_response(
            StatusCode::GONE,
            "Last-Event-ID is outside the retained resume window".to_owned(),
        ),
        Err(PollEventsError::InvalidEventId) => protocol_error_response(
            StatusCode::BAD_REQUEST,
            "invalid Last-Event-ID for this session".to_owned(),
        ),
        Err(PollEventsError::MissingSession) => protocol_error_response(
            StatusCode::BAD_REQUEST,
            "missing or invalid Mcp-Session-Id".to_owned(),
        ),
    };
    apply_origin_headers(response, origin.as_deref())
}

async fn handle_delete_mcp(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = authorize_request(&state, &headers) {
        return response;
    }
    let origin = match validate_origin(&state, &headers) {
        Ok(origin) => origin,
        Err(response) => return *response,
    };
    if let Some(response) = validate_protocol_version_header(&headers) {
        return apply_origin_headers(response, origin.as_deref());
    }
    let session_id = match session_id_from_headers(&headers) {
        Ok(session_id) => session_id,
        Err(response) => return apply_origin_headers(*response, origin.as_deref()),
    };
    if !state.sessions.delete_session(&session_id) {
        return apply_origin_headers(
            protocol_error_response(
                StatusCode::BAD_REQUEST,
                "missing or invalid Mcp-Session-Id".to_owned(),
            ),
            origin.as_deref(),
        );
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        MCP_PROTOCOL_VERSION_HEADER,
        HeaderValue::from_static(spec::MCP_PROTOCOL_VERSION),
    );
    apply_origin_headers(response, origin.as_deref())
}

fn dispatch_logging_set_level(
    session: Arc<HttpSession>,
    id: Value,
    params: Option<Value>,
) -> Response {
    let level = match logging::parse_set_level_params(params.as_ref()) {
        Ok(level) => level,
        Err(error) => return jsonrpc_error_response(id, -32602, error.to_string()),
    };
    logging::set_level(level);
    session.set_log_level(level);
    emit_http_log(
        &session,
        level,
        "transport",
        format!("log level set to {}", level.as_str()),
    );
    jsonrpc_ok_response(id, serde_json::json!({ "level": level.as_str() }))
}

fn emit_http_log(
    session: &Arc<HttpSession>,
    level: logging::LogLevel,
    logger: &str,
    message: String,
) {
    if !session.should_emit_log(level) {
        return;
    }
    session.enqueue_event(logging::log_notification(level, logger, message).to_string());
}

async fn dispatch_tool_call(
    state: AppState,
    session: Arc<HttpSession>,
    id: Value,
    params: Option<Value>,
) -> Response {
    let name = match params
        .as_ref()
        .and_then(|value| value.get("name"))
        .and_then(|value| value.as_str())
    {
        Some(name) => name.to_owned(),
        None => return jsonrpc_error_response(id, -32602, "missing tool name".to_owned()),
    };
    let args = params
        .as_ref()
        .and_then(|value| value.get("arguments"))
        .cloned();
    let progress_token = params
        .as_ref()
        .and_then(|value| {
            value
                .get("_meta")
                .and_then(|meta| meta.get("progressToken"))
                .or_else(|| value.get("progressToken"))
        })
        .cloned();
    let repo_root = Arc::clone(&state.repo_root);
    let db_path = Arc::clone(&state.db_path);
    let session_for_progress = Arc::clone(&session);
    let session_for_result = Arc::clone(&session);
    let progress_token_for_closure = progress_token.clone();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_worker = Arc::clone(&cancel_flag);
    let name_for_log = name.clone();

    let result = tokio::task::spawn_blocking(move || {
        crate::progress::install(
            move |message, percentage| {
                if let Some(token) = &progress_token_for_closure {
                    let value = serde_json::json!({
                        "kind": "report",
                        "message": match percentage {
                            Some(pct) => format!("{message} ({pct}%)"),
                            None => message.to_owned(),
                        },
                    });
                    let notification = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "$/progress",
                        "params": { "token": token, "value": value },
                    });
                    session_for_progress.enqueue_event(notification.to_string());
                }
            },
            cancel_flag_worker,
        );
        let call_result = tools::call(&name, args.as_ref(), &repo_root, &db_path);
        crate::progress::uninstall();
        call_result.map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(tool_result)) => {
            emit_http_log(
                &session_for_result,
                logging::LogLevel::Info,
                "tools/call",
                format!("tool={} success=true", name_for_log),
            );
            let response_json = serde_json::json!({
                "jsonrpc": "2.0",
                "id": &id,
                "result": &tool_result,
            });
            session_for_result.enqueue_event(response_json.to_string());
            jsonrpc_ok_response(id, tool_result)
        }
        Ok(Err(error_message)) => {
            emit_http_log(
                &session_for_result,
                logging::LogLevel::Error,
                "tools/call",
                format!(
                    "tool={} success=false error={}",
                    name_for_log, error_message
                ),
            );
            let response_json = serde_json::json!({
                "jsonrpc": "2.0",
                "id": &id,
                "error": {
                    "code": -32001,
                    "message": &error_message,
                    "data": {
                        "atlas_error_code": "tool_execution_failed",
                        "atlas_error_code_docs": error_code_docs_ref("tool_execution_failed")
                    }
                }
            });
            session_for_result.enqueue_event(response_json.to_string());
            jsonrpc_error_response(id, -32001, error_message)
        }
        Err(join_error) => {
            jsonrpc_error_response(id, -32603, format!("worker panicked: {join_error}"))
        }
    }
}

fn require_session(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<Arc<HttpSession>, Box<Response>> {
    let session_id = session_id_from_headers(headers)?;
    state
        .sessions
        .require_session(&session_id)
        .map_err(|error| match error {
            SessionLookupError::Missing => Box::new(protocol_error_response(
                StatusCode::BAD_REQUEST,
                "missing or invalid Mcp-Session-Id".to_owned(),
            )),
        })
}

fn session_id_from_headers(headers: &HeaderMap) -> std::result::Result<String, Box<Response>> {
    headers
        .get(MCP_SESSION_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            Box::new(protocol_error_response(
                StatusCode::BAD_REQUEST,
                "missing or invalid Mcp-Session-Id".to_owned(),
            ))
        })
}

fn validate_protocol_version_header(headers: &HeaderMap) -> Option<Response> {
    match headers
        .get(MCP_PROTOCOL_VERSION_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        Some(version) if version == spec::MCP_PROTOCOL_VERSION => None,
        Some(version) => Some(protocol_error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "unsupported MCP-Protocol-Version '{version}'; supported version: {}",
                spec::MCP_PROTOCOL_VERSION
            ),
        )),
        None => Some(protocol_error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "missing MCP-Protocol-Version header; expected {}",
                spec::MCP_PROTOCOL_VERSION
            ),
        )),
    }
}

fn validate_origin(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<Option<String>, Box<Response>> {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(None);
    };
    let Ok(origin) = origin.to_str() else {
        return Err(Box::new(forbidden_origin_response()));
    };
    if state.allowed_origins.is_empty() || state.allowed_origins.contains(origin) {
        Ok(Some(origin.to_owned()))
    } else {
        Err(Box::new(forbidden_origin_response()))
    }
}

fn forbidden_origin_response() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "error": "forbidden",
            "message": "Origin is not allowed for this MCP server",
        })),
    )
        .into_response()
}

fn authorize_request(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let policy = state.auth_policy.as_ref()?;
    match policy.authorize(headers, auth::ROUTE_FAMILY_MCP) {
        Ok(_) => None,
        Err(challenge) => Some(auth_challenge_response(challenge)),
    }
}

fn auth_challenge_response(challenge: auth::AuthChallenge) -> Response {
    let mut response = (
        challenge.status,
        Json(serde_json::json!({
            "error": challenge.body_error,
            "message": challenge.body_message,
        })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&challenge.www_authenticate) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

fn sse_response(session_id: &str, events: Vec<crate::http_sessions::RetainedEvent>) -> Response {
    let mut body = String::new();
    for event in events {
        body.push_str("event: message\n");
        body.push_str("id: ");
        body.push_str(&event.id);
        body.push('\n');
        body.push_str("data: ");
        body.push_str(&event.payload_json);
        body.push_str("\n\n");
    }

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .header(MCP_SESSION_ID_HEADER, session_id)
        .header(MCP_PROTOCOL_VERSION_HEADER, spec::MCP_PROTOCOL_VERSION)
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    response
}

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
    let atlas_error_code = match code {
        -32700 => "parse_error",
        -32600 => "invalid_request",
        -32601 => "method_not_found",
        -32602 => "invalid_params",
        -32603 => "internal_error",
        -32001 => "tool_execution_failed",
        -32002 => "worker_unavailable",
        -32003 => "request_timed_out",
        -32004 => "rate_limited",
        _ => "internal_error",
    };
    (
        status,
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message,
                "data": {
                    "atlas_error_code": atlas_error_code,
                    "atlas_error_code_docs": error_code_docs_ref(atlas_error_code)
                }
            }
        })),
    )
        .into_response()
}

fn protocol_error_response(status: StatusCode, message: String) -> Response {
    let mut response = jsonrpc_error_response(Value::Null, -32600, message);
    *response.status_mut() = status;
    response.headers_mut().insert(
        MCP_PROTOCOL_VERSION_HEADER,
        HeaderValue::from_static(spec::MCP_PROTOCOL_VERSION),
    );
    response
}

fn apply_origin_headers(mut response: Response, origin: Option<&str>) -> Response {
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, DELETE"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(
            "Authorization, Content-Type, MCP-Protocol-Version, Mcp-Session-Id, Last-Event-ID",
        ),
    );
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(origin) = origin
        && let Ok(value) = HeaderValue::from_str(origin)
    {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::to_bytes;
    use axum::extract::State as AxumState;
    use axum::routing::get as axum_get;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde_json::json;
    use std::collections::HashMap;
    use std::time::Duration;

    const TEST_SECRET: &[u8] = b"atlas-mcp-phase5-secret";
    const TEST_SECRET_B64U: &str = "YXRsYXMtbWNwLXBoYXNlNS1zZWNyZXQ";

    fn make_state(_legacy_token: Option<&str>) -> AppState {
        AppState {
            repo_root: Arc::new("repo".to_owned()),
            db_path: Arc::new("db".to_owned()),
            auth_policy: None,
            allowed_origins: Arc::new(HashSet::new()),
            sessions: SessionManager::for_tests(
                Duration::from_secs(60),
                8,
                Duration::from_secs(60),
                Duration::from_millis(1),
            ),
        }
    }

    #[derive(Clone)]
    struct MockAuthState {
        discovery: Arc<String>,
        jwks: Arc<String>,
    }

    async fn make_state_with_auth(origins: &[&str]) -> AppState {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock auth server");
        let addr = listener.local_addr().expect("mock auth addr");
        let base_url = format!("http://{}", addr);
        let discovery = json!({
            "issuer": base_url,
            "jwks_uri": format!("{base_url}/jwks")
        })
        .to_string();
        let jwks = json!({
            "keys": [
                {
                    "kty": "oct",
                    "use": "sig",
                    "kid": "atlas-test-key",
                    "alg": "HS256",
                    "k": TEST_SECRET_B64U
                }
            ]
        })
        .to_string();
        let app = Router::new()
            .route(
                "/.well-known/openid-configuration",
                axum_get(|AxumState(state): AxumState<MockAuthState>| async move {
                    state.discovery.as_str().to_owned()
                }),
            )
            .route(
                "/jwks",
                axum_get(|AxumState(state): AxumState<MockAuthState>| async move {
                    state.jwks.as_str().to_owned()
                }),
            )
            .with_state(MockAuthState {
                discovery: Arc::new(discovery),
                jwks: Arc::new(jwks),
            });
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve mock auth server");
        });

        let policy = ProtectedResourceAuthPolicy::load(auth::ProtectedResourceAuthConfig {
            issuer: base_url,
            discovery_url: None,
            jwks_url: None,
            resource: "https://atlas.test/mcp".to_owned(),
            required_scopes: HashMap::from([(
                auth::ROUTE_FAMILY_MCP.to_owned(),
                vec!["atlas:mcp".to_owned(), "atlas:read".to_owned()],
            )]),
            allowed_origins: origins.iter().map(|value| (*value).to_owned()).collect(),
        })
        .await
        .expect("load auth policy");

        AppState {
            auth_policy: Some(Arc::new(policy)),
            allowed_origins: Arc::new(origins.iter().map(|value| (*value).to_owned()).collect()),
            ..make_state(None)
        }
    }

    fn make_state_with_origins(origins: &[&str]) -> AppState {
        AppState {
            allowed_origins: Arc::new(origins.iter().map(|value| (*value).to_owned()).collect()),
            ..make_state(None)
        }
    }

    fn bearer_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .expect("authorization header"),
        );
        headers
    }

    fn make_token(issuer: &str, scopes: &[&str]) -> String {
        let claims = json!({
            "iss": issuer,
            "sub": "user-123",
            "aud": "https://atlas.test/mcp",
            "exp": 4_102_444_800u64,
            "scope": scopes.join(" "),
        });
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("atlas-test-key".to_owned());
        encode(&header, &claims, &EncodingKey::from_secret(TEST_SECRET)).expect("encode token")
    }

    fn initialize_body(protocol_version: &str, extra_fields: &str) -> Bytes {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"zed\",\"version\":\"1.0.0\"}}{}}}}}",
            protocol_version, extra_fields
        )
        .into_bytes()
        .into()
    }

    async fn read_json_response(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        serde_json::from_slice(&bytes).expect("response json")
    }

    async fn read_body_string(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }

    fn header_value(response: &Response, name: &str) -> String {
        response
            .headers()
            .get(name)
            .expect("header present")
            .to_str()
            .expect("header utf8")
            .to_owned()
    }

    async fn initialize_session(state: AppState) -> (AppState, String) {
        let response = handle_post_mcp(
            State(state.clone()),
            HeaderMap::new(),
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let session_id = header_value(&response, MCP_SESSION_ID_HEADER);
        (state, session_id)
    }

    fn session_headers(session_id: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            HeaderValue::from_static(spec::MCP_PROTOCOL_VERSION),
        );
        headers.insert(
            MCP_SESSION_ID_HEADER,
            HeaderValue::from_str(session_id).expect("session header"),
        );
        headers
    }

    #[tokio::test]
    async fn metadata_endpoint_returns_protected_resource_body_shape() {
        let state = make_state_with_auth(&[]).await;
        let response = handle_protected_resource_metadata(State(state), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = read_json_response(response).await;
        assert_eq!(value["resource"], json!("https://atlas.test/mcp"));
        assert_eq!(
            value["authorization_servers"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(value["bearer_methods_supported"], json!(["header"]));
        assert_eq!(
            value["scopes_supported"],
            json!(["atlas:mcp", "atlas:read"])
        );
    }

    #[tokio::test]
    async fn missing_bearer_token_returns_www_authenticate() {
        let state = make_state_with_auth(&[]).await;
        let response = handle_post_mcp(
            State(state),
            HeaderMap::new(),
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            header_value(&response, header::WWW_AUTHENTICATE.as_str()),
            "Bearer realm=\"atlas-mcp\", resource=\"https://atlas.test/mcp\", error=\"invalid_token\", error_description=\"Bearer token required\", scope=\"atlas:mcp atlas:read\""
        );
        let value = read_json_response(response).await;
        assert_eq!(value["message"], json!("Bearer token required"));
    }

    #[tokio::test]
    async fn invalid_bearer_token_returns_www_authenticate() {
        let state = make_state_with_auth(&[]).await;
        let response = handle_post_mcp(
            State(state),
            bearer_headers("not-a-jwt"),
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            header_value(&response, header::WWW_AUTHENTICATE.as_str())
                .contains("error=\"invalid_token\"")
        );
        let value = read_json_response(response).await;
        assert_eq!(value["message"], json!("invalid bearer token"));
    }

    #[tokio::test]
    async fn insufficient_scope_returns_incremental_scope_challenge() {
        let state = make_state_with_auth(&[]).await;
        let issuer = state
            .auth_policy
            .as_ref()
            .expect("auth policy")
            .issuer()
            .to_owned();
        let response = handle_post_mcp(
            State(state),
            bearer_headers(&make_token(&issuer, &["atlas:mcp"])),
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let challenge = header_value(&response, header::WWW_AUTHENTICATE.as_str());
        assert!(challenge.contains("error=\"insufficient_scope\""));
        assert!(challenge.contains("scope=\"atlas:mcp atlas:read\""));
        assert!(challenge.contains("resource=\"https://atlas.test/mcp\""));
    }

    #[tokio::test]
    async fn forbidden_origin_with_valid_token_is_rejected_independently() {
        let state = make_state_with_auth(&["https://good.test"]).await;
        let issuer = state
            .auth_policy
            .as_ref()
            .expect("auth policy")
            .issuer()
            .to_owned();
        let mut headers = bearer_headers(&make_token(&issuer, &["atlas:mcp", "atlas:read"]));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.test"),
        );
        let response = handle_post_mcp(
            State(state),
            headers,
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
        let value = read_json_response(response).await;
        assert_eq!(
            value["message"],
            json!("Origin is not allowed for this MCP server")
        );
    }

    #[tokio::test]
    async fn http_initialize_uses_shared_builder_and_returns_session_header() {
        let response = handle_post_mcp(
            State(make_state(None)),
            HeaderMap::new(),
            initialize_body(
                spec::MCP_PROTOCOL_VERSION,
                ",\"_meta\":{\"clientTag\":\"abc\"}",
            ),
        )
        .await;
        let value = read_json_response(response).await;
        assert_eq!(
            value["result"],
            spec::negotiate_initialize(Some(&json!({
                "protocolVersion": spec::MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "zed", "version": "1.0.0" },
                "_meta": { "clientTag": "abc" }
            })))
            .expect("shared initialize result")
        );
    }

    #[tokio::test]
    async fn http_initialize_rejects_missing_client_info() {
        let body: Bytes = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{}}}".as_slice().into();
        let response = handle_post_mcp(State(make_state(None)), HeaderMap::new(), body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let value = read_json_response(response).await;
        assert_eq!(value["error"]["code"], json!(-32602));
        assert_eq!(
            value["error"]["message"],
            json!("initialize requires object params.clientInfo")
        );
        assert_eq!(
            value["error"]["data"]["atlas_error_code"],
            json!("invalid_params")
        );
    }

    #[tokio::test]
    async fn http_initialize_rejects_unsupported_protocol_version() {
        let response = handle_post_mcp(
            State(make_state(None)),
            HeaderMap::new(),
            initialize_body("2024-11-05", ""),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let value = read_json_response(response).await;
        assert_eq!(value["error"]["code"], json!(-32602));
        assert_eq!(
            value["error"]["message"],
            json!("unsupported protocol version '2024-11-05'; supported version: 2025-11-25")
        );
    }

    #[tokio::test]
    async fn http_session_creation_reuse_missing_session_and_delete_work() {
        let (state, session_id) = initialize_session(make_state(None)).await;

        let response = handle_post_mcp(
            State(state.clone()),
            session_headers(&session_id),
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}"
                .as_slice()
                .into(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = read_json_response(response).await;
        assert_eq!(
            value["result"],
            crate::tools::tool_list(),
            "http tools/list must serialize shared typed descriptor registry"
        );

        let response = handle_post_mcp(
            State(state.clone()),
            HeaderMap::new(),
            b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/list\",\"params\":{}}"
                .as_slice()
                .into(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = read_json_response(response).await;
        assert_eq!(
            value["error"]["message"],
            json!(format!(
                "missing MCP-Protocol-Version header; expected {}",
                spec::MCP_PROTOCOL_VERSION
            ))
        );

        let response = handle_delete_mcp(State(state.clone()), session_headers(&session_id)).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = handle_post_mcp(
            State(state),
            session_headers(&session_id),
            b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/list\",\"params\":{}}"
                .as_slice()
                .into(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = read_json_response(response).await;
        assert_eq!(
            value["error"]["message"],
            json!("missing or invalid Mcp-Session-Id")
        );
    }

    #[tokio::test]
    async fn resumed_poll_receives_only_missed_events() {
        let (state, session_id) = initialize_session(make_state(None)).await;
        let session = state
            .sessions
            .require_session(&session_id)
            .expect("session");
        let first_id = session.enqueue_event("{\"n\":1}".to_owned());
        session.enqueue_event("{\"n\":2}".to_owned());

        let mut headers = session_headers(&session_id);
        headers.insert(
            LAST_EVENT_ID_HEADER,
            HeaderValue::from_str(&first_id).unwrap(),
        );
        let response = handle_get_mcp(State(state), headers).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = read_body_string(response).await;
        assert!(body.contains("id: "));
        assert!(body.contains("{\"n\":2}"));
        assert!(!body.contains("{\"n\":1}"));
    }

    #[tokio::test]
    async fn sessions_do_not_receive_each_others_events() {
        let (state, session_a) = initialize_session(make_state(None)).await;
        let response = handle_post_mcp(
            State(state.clone()),
            HeaderMap::new(),
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        let session_b = header_value(&response, MCP_SESSION_ID_HEADER);

        let session = state
            .sessions
            .require_session(&session_a)
            .expect("session a");
        session.enqueue_event("{\"owner\":\"a\"}".to_owned());

        let response = handle_get_mcp(State(state), session_headers(&session_b)).await;
        let body = read_body_string(response).await;
        assert!(!body.contains("owner"));
    }

    #[tokio::test]
    async fn server_initiated_disconnect_can_resume() {
        let (state, session_id) = initialize_session(make_state(None)).await;
        let response = handle_get_mcp(State(state.clone()), session_headers(&session_id)).await;
        assert_eq!(response.status(), StatusCode::OK);

        let session = state
            .sessions
            .require_session(&session_id)
            .expect("session");
        let event_id = session.enqueue_event("{\"step\":1}".to_owned());

        let mut headers = session_headers(&session_id);
        headers.insert(
            LAST_EVENT_ID_HEADER,
            HeaderValue::from_str(&(session_id.clone() + ":0")).unwrap(),
        );
        let response = handle_get_mcp(State(state), headers).await;
        let body = read_body_string(response).await;
        assert!(body.contains(&event_id));
        assert!(body.contains("{\"step\":1}"));
    }

    #[tokio::test]
    async fn expired_last_event_id_returns_gone() {
        let mut state = make_state(None);
        state.sessions = SessionManager::for_tests(
            Duration::from_secs(60),
            1,
            Duration::from_secs(60),
            Duration::from_millis(1),
        );
        let (state, session_id) = initialize_session(state).await;
        let session = state
            .sessions
            .require_session(&session_id)
            .expect("session");
        let first = session.enqueue_event("{\"n\":1}".to_owned());
        session.enqueue_event("{\"n\":2}".to_owned());
        session.enqueue_event("{\"n\":3}".to_owned());

        let mut headers = session_headers(&session_id);
        headers.insert(LAST_EVENT_ID_HEADER, HeaderValue::from_str(&first).unwrap());
        let response = handle_get_mcp(State(state), headers).await;
        assert_eq!(response.status(), StatusCode::GONE);
        let value = read_json_response(response).await;
        assert_eq!(
            value["error"]["message"],
            json!("Last-Event-ID is outside the retained resume window")
        );
    }

    #[tokio::test]
    async fn missing_version_header_on_post_initialize_request_is_rejected() {
        let (state, session_id) = initialize_session(make_state(None)).await;
        let mut headers = HeaderMap::new();
        headers.insert(
            MCP_SESSION_ID_HEADER,
            HeaderValue::from_str(&session_id).unwrap(),
        );
        let response = handle_post_mcp(
            State(state),
            headers,
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}"
                .as_slice()
                .into(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = read_json_response(response).await;
        assert_eq!(
            value["error"]["message"],
            json!(format!(
                "missing MCP-Protocol-Version header; expected {}",
                spec::MCP_PROTOCOL_VERSION
            ))
        );
    }

    #[tokio::test]
    async fn mismatched_version_header_is_rejected() {
        let (state, session_id) = initialize_session(make_state(None)).await;
        let mut headers = HeaderMap::new();
        headers.insert(
            MCP_SESSION_ID_HEADER,
            HeaderValue::from_str(&session_id).unwrap(),
        );
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            HeaderValue::from_static("2024-11-05"),
        );
        let response = handle_get_mcp(State(state), headers).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = read_json_response(response).await;
        assert_eq!(
            value["error"]["message"],
            json!("unsupported MCP-Protocol-Version '2024-11-05'; supported version: 2025-11-25")
        );
    }

    #[tokio::test]
    async fn invalid_origin_returns_forbidden() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.test"),
        );
        let response = handle_post_mcp(
            State(make_state_with_origins(&["https://good.test"])),
            headers,
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn jsonrpc_batch_request_rejected_on_http() {
        let response = handle_post_mcp(
            State(make_state(None)),
            HeaderMap::new(),
            b"[]".as_slice().into(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = read_json_response(response).await;
        assert_eq!(
            value["error"]["message"],
            json!("JSON-RPC batch requests are not supported")
        );
    }

    #[tokio::test]
    async fn logging_notifications_are_isolated_per_session() {
        let (state, session_a) = initialize_session(make_state(None)).await;
        let response = handle_post_mcp(
            State(state.clone()),
            HeaderMap::new(),
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        let session_b = header_value(&response, MCP_SESSION_ID_HEADER);

        let _ = handle_post_mcp(
            State(state.clone()),
            session_headers(&session_a),
            b"{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}"
                .as_slice()
                .into(),
        )
        .await;
        let _ = handle_post_mcp(
            State(state.clone()),
            session_headers(&session_b),
            b"{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}"
                .as_slice()
                .into(),
        )
        .await;

        let response = handle_post_mcp(
            State(state.clone()),
            session_headers(&session_a),
            b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"logging/setLevel\",\"params\":{\"level\":\"warning\"}}"
                .as_slice()
                .into(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let response = handle_post_mcp(
            State(state.clone()),
            session_headers(&session_a),
            b"{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"output_format\":\"bogus\"}}}"
                .as_slice()
                .into(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let response = handle_get_mcp(State(state.clone()), session_headers(&session_a)).await;
        let body_a = read_body_string(response).await;
        assert!(body_a.contains("notifications/message"));
        assert!(body_a.contains("tool=query_graph success=false"));

        let response = handle_get_mcp(State(state), session_headers(&session_b)).await;
        let body_b = read_body_string(response).await;
        assert!(!body_b.contains("notifications/message"));
        assert!(!body_b.contains("tool=query_graph success=false"));
    }

    #[tokio::test]
    async fn tool_responses_are_isolated_per_session() {
        let (state, session_a) = initialize_session(make_state(None)).await;
        let response = handle_post_mcp(
            State(state.clone()),
            HeaderMap::new(),
            initialize_body(spec::MCP_PROTOCOL_VERSION, ""),
        )
        .await;
        let session_b = header_value(&response, MCP_SESSION_ID_HEADER);

        let response = handle_post_mcp(
            State(state.clone()),
            session_headers(&session_a),
            b"{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"broker_status\",\"arguments\":{}}}"
                .as_slice()
                .into(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let response = handle_get_mcp(State(state.clone()), session_headers(&session_a)).await;
        let body_a = read_body_string(response).await;
        assert!(body_a.contains("broker_status") || body_a.contains("worker_threads"));

        let response = handle_get_mcp(State(state), session_headers(&session_b)).await;
        let body_b = read_body_string(response).await;
        assert!(!body_b.contains("broker_status"));
        assert!(!body_b.contains("worker_threads"));
    }
}
