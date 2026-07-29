//! Streamable HTTP transport for atlas-mcp.
//!
//! Enabled with `http-transport` Cargo feature.
//!
//! Routes:
//! - `POST /mcp` — stateless JSON-RPC request ingress with JSON or one-shot SSE response
//! - `GET /health` — unauthenticated liveness probe
//! - `GET /.well-known/oauth-protected-resource` — OAuth protected-resource metadata

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result, anyhow};
use atlas_core::error_code_docs_ref;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

use crate::auth::{self, ProtectedResourceAuthPolicy};
use crate::spec;
use crate::tools;
use crate::tools::health::mark_server_started;
use crate::{completion, resources};

const DEFAULT_BIND: &str = "127.0.0.1:7070";
const MCP_PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";
const MCP_METHOD_HEADER: &str = "Mcp-Method";
const MCP_NAME_HEADER: &str = "Mcp-Name";
const MCP_PARAM_HEADER_PREFIX: &str = "mcp-param-";
#[cfg(test)]
const MCP_SESSION_ID_HEADER: &str = "Mcp-Session-Id";
#[cfg(test)]
const LAST_EVENT_ID_HEADER: &str = "Last-Event-ID";
const PROTECTED_RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";
const BASE64_SENTINELS: [&str; 2] = ["base64:", "b64:"];
const SUBSCRIPTION_ID_META_KEY: &str = "io.modelcontextprotocol/subscriptionId";
const SUBSCRIPTION_TYPE_TOOLS_LIST_CHANGED: &str = "notifications/tools/list_changed";
const SUBSCRIPTION_TYPE_PROMPTS_LIST_CHANGED: &str = "notifications/prompts/list_changed";
const SUBSCRIPTION_TYPE_RESOURCES_LIST_CHANGED: &str = "notifications/resources/list_changed";
const SUBSCRIPTION_TYPE_RESOURCE_UPDATED: &str = "notifications/resources/updated";

#[derive(Clone)]
struct AppState {
    repo_root: Arc<String>,
    db_path: Arc<String>,
    auth_policy: Option<Arc<ProtectedResourceAuthPolicy>>,
    allowed_origins: Arc<HashSet<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseMode {
    Json,
    Sse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthorizedRequest {
    subject: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body_text: String,
    pub json_body: Option<Value>,
}

#[derive(Clone)]
pub struct HttpTestHarness {
    state: AppState,
    test_auth_issuer: Option<String>,
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

impl HttpTestHarness {
    pub fn new(repo_root: &str, db_path: &str) -> Self {
        Self {
            state: AppState {
                repo_root: Arc::new(repo_root.to_owned()),
                db_path: Arc::new(db_path.to_owned()),
                auth_policy: None,
                allowed_origins: Arc::new(HashSet::new()),
            },
            test_auth_issuer: None,
        }
    }

    pub fn new_with_test_auth(
        repo_root: &str,
        db_path: &str,
        allowed_origins: &[&str],
    ) -> Result<Self> {
        let (issuer, policy) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("cannot build tokio runtime for HTTP auth test harness")?
            .block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .context("bind mock auth server")?;
                let addr = listener.local_addr().context("mock auth addr")?;
                let issuer = format!("http://{addr}");
                let discovery = serde_json::json!({
                    "issuer": issuer,
                    "jwks_uri": format!("{issuer}/jwks")
                })
                .to_string();
                let jwks = serde_json::json!({
                    "keys": [
                        {
                            "kty": "oct",
                            "use": "sig",
                            "kid": "atlas-test-key",
                            "alg": "HS256",
                            "k": "YXRsYXMtbWNwLXBoYXNlNS1zZWNyZXQ"
                        }
                    ]
                })
                .to_string();
                let app = Router::new()
                    .route(
                        "/.well-known/openid-configuration",
                        get(move || {
                            let discovery = discovery.clone();
                            async move { discovery }
                        }),
                    )
                    .route(
                        "/jwks",
                        get(move || {
                            let jwks = jwks.clone();
                            async move { jwks }
                        }),
                    );
                tokio::spawn(async move {
                    let _ = axum::serve(listener, app).await;
                });
                let policy = ProtectedResourceAuthPolicy::load(auth::ProtectedResourceAuthConfig {
                    issuer: issuer.clone(),
                    discovery_url: None,
                    jwks_url: None,
                    resource: "https://atlas.test/mcp".to_owned(),
                    required_scopes: HashMap::from([(
                        auth::ROUTE_FAMILY_MCP.to_owned(),
                        vec!["atlas:mcp".to_owned(), "atlas:read".to_owned()],
                    )]),
                    allowed_origins: allowed_origins
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect(),
                })
                .await?;
                Ok::<_, anyhow::Error>((issuer, policy))
            })?;

        Ok(Self {
            state: AppState {
                repo_root: Arc::new(repo_root.to_owned()),
                db_path: Arc::new(db_path.to_owned()),
                auth_policy: Some(Arc::new(policy)),
                allowed_origins: Arc::new(
                    allowed_origins
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect(),
                ),
            },
            test_auth_issuer: Some(issuer),
        })
    }

    pub fn post_jsonrpc(&self, headers: &[(&str, &str)], body: &Value) -> Result<TestHttpResponse> {
        let mut headers = make_test_header_map(headers)?;
        maybe_insert_test_mirror_headers(&mut headers, body)?;
        let body = Bytes::from(serde_json::to_vec(body)?);
        let response = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("cannot build tokio runtime for HTTP test harness")?
            .block_on(handle_post_mcp(State(self.state.clone()), headers, body));
        response_to_test_output(response)
    }

    pub fn get_metadata(&self, headers: &[(&str, &str)]) -> Result<TestHttpResponse> {
        let headers = make_test_header_map(headers)?;
        let response = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("cannot build tokio runtime for HTTP test harness")?
            .block_on(handle_protected_resource_metadata(
                State(self.state.clone()),
                headers,
            ));
        response_to_test_output(response)
    }

    pub fn make_test_bearer_token(&self, scopes: &[&str]) -> Option<String> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

        const TEST_SECRET: &[u8] = b"atlas-mcp-phase5-secret";

        let issuer = self.test_auth_issuer.as_ref()?;
        let claims = serde_json::json!({
            "iss": issuer,
            "sub": "user-123",
            "aud": "https://atlas.test/mcp",
            "exp": 4_102_444_800u64,
            "scope": scopes.join(" "),
        });
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("atlas-test-key".to_owned());
        encode(&header, &claims, &EncodingKey::from_secret(TEST_SECRET)).ok()
    }
}

fn make_test_header_map(headers: &[(&str, &str)]) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            axum::http::header::HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(value)?,
        );
    }
    Ok(map)
}

fn maybe_insert_test_mirror_headers(headers: &mut HeaderMap, body: &Value) -> Result<()> {
    if !headers.contains_key(header::ACCEPT) {
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    }
    if !headers.contains_key(MCP_PROTOCOL_VERSION_HEADER) {
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            HeaderValue::from_static(spec::MCP_PROTOCOL_VERSION),
        );
    }
    let Some(method) = body.get("method").and_then(Value::as_str) else {
        return Ok(());
    };
    if !headers.contains_key(MCP_METHOD_HEADER) {
        headers.insert(MCP_METHOD_HEADER, HeaderValue::from_str(method)?);
    }
    if !headers.contains_key(MCP_NAME_HEADER)
        && let Some(name) = body
            .get("params")
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
    {
        headers.insert(MCP_NAME_HEADER, HeaderValue::from_str(name)?);
    }
    Ok(())
}

fn response_to_test_output(response: Response) -> Result<TestHttpResponse> {
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            Ok((
                name.as_str().to_owned(),
                value
                    .to_str()
                    .context("HTTP test header must be utf-8")?
                    .to_owned(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let body_text = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot build tokio runtime for HTTP test body read")?
        .block_on(async move {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .context("cannot read HTTP test response body")?;
            String::from_utf8(bytes.to_vec()).context("HTTP test response body must be utf-8")
        })?;
    let json_body = serde_json::from_str(&body_text).ok();
    Ok(TestHttpResponse {
        status,
        headers,
        body_text,
        json_body,
    })
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handle_health))
        .route(
            PROTECTED_RESOURCE_METADATA_PATH,
            get(handle_protected_resource_metadata),
        )
        .route("/mcp", post(handle_post_mcp))
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
    let authorized = match authorize_request(&state, &headers) {
        Ok(authorized) => authorized,
        Err(response) => return *response,
    };
    let origin = match validate_origin(&state, &headers) {
        Ok(origin) => origin,
        Err(response) => return *response,
    };
    let response_mode = match negotiate_response_mode(&headers) {
        Ok(mode) => mode,
        Err(response) => return apply_origin_headers(*response, origin.as_deref()),
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

    let protocol_version = match required_header_value(&headers, MCP_PROTOCOL_VERSION_HEADER) {
        Ok(value) => value,
        Err(response) => return apply_origin_headers(*response, origin.as_deref()),
    };
    if protocol_version != spec::MCP_PROTOCOL_VERSION {
        return apply_origin_headers(
            jsonrpc_error_with_data_response(
                id.clone(),
                -32022,
                format!(
                    "unsupported MCP-Protocol-Version '{protocol_version}'; supported version: {}",
                    spec::MCP_PROTOCOL_VERSION
                ),
                Some(serde_json::json!({
                    "supportedVersions": spec::supported_protocol_versions_value(),
                })),
            ),
            origin.as_deref(),
        );
    }
    if let Err(response) = validate_header_mirrors(&headers, &id, &method, params.as_ref()) {
        return apply_origin_headers(*response, origin.as_deref());
    }

    let response = match method.as_str() {
        "initialize" => match spec::parse_initialize_request(params.as_ref()) {
            Ok(request) => {
                if request.protocol_version != protocol_version {
                    jsonrpc_error_with_data_response(
                        id,
                        -32020,
                        format!(
                            "header/body mismatch for MCP-Protocol-Version: header='{protocol_version}' body='{}'",
                            request.protocol_version
                        ),
                        Some(serde_json::json!({
                            "atlas_error_code": "header_mismatch",
                            "header": MCP_PROTOCOL_VERSION_HEADER,
                            "headerValue": protocol_version,
                            "bodyField": "params.protocolVersion",
                            "bodyValue": request.protocol_version,
                        })),
                    )
                } else {
                    match spec::ensure_supported_protocol_version(&request.protocol_version) {
                        Ok(()) => jsonrpc_ok_response(
                            id,
                            serde_json::to_value(spec::initialize_result(&request))
                                .expect("initialize result serialization"),
                        ),
                        Err(error) => jsonrpc_error_with_data_response(
                            id,
                            -32022,
                            error.to_string(),
                            Some(serde_json::json!({
                                "supportedVersions": spec::supported_protocol_versions_value(),
                            })),
                        ),
                    }
                }
            }
            Err(error) => jsonrpc_error_response(id, -32602, error.to_string()),
        },
        "server/discover" => jsonrpc_ok_response(id, spec::discover_result()),
        "subscriptions/listen" => dispatch_subscriptions_listen(id, params, response_mode),
        "initialized" | "notifications/initialized" => StatusCode::NO_CONTENT.into_response(),
        m if m.starts_with("notifications/") => StatusCode::NO_CONTENT.into_response(),
        "tools/list" => respond_with_mode(
            response_mode,
            jsonrpc_envelope_result(id, tools::tool_list()),
        ),
        "resources/list" => match resources::resources_list(params.as_ref()) {
            Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
            Err(error) => respond_with_mode(
                response_mode,
                jsonrpc_envelope_error(id, -32602, error.to_string(), None),
            ),
        },
        "resources/templates/list" => match resources::resources_templates_list(params.as_ref()) {
            Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
            Err(error) => respond_with_mode(
                response_mode,
                jsonrpc_envelope_error(id, -32602, error.to_string(), None),
            ),
        },
        "resources/read" => {
            match resources::resources_read(params.as_ref(), &state.repo_root, &state.db_path) {
                Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
                Err(error) => respond_with_mode(
                    response_mode,
                    jsonrpc_envelope_error(id, -32602, error.to_string(), None),
                ),
            }
        }
        "completion/complete" => {
            match completion::complete(params.as_ref(), &state.repo_root, &state.db_path) {
                Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
                Err(error) => respond_with_mode(
                    response_mode,
                    jsonrpc_envelope_error(id, -32602, error.to_string(), None),
                ),
            }
        }
        "tools/call" => {
            dispatch_tool_call(state, id, params, response_mode, authorized.subject).await
        }
        "tasks/list" => match crate::tasks::tasks_list(
            params.as_ref(),
            &state.repo_root,
            crate::output::OutputFormat::Json,
        ) {
            Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
            Err(error) => respond_with_mode(response_mode, jsonrpc_envelope_task_error(id, error)),
        },
        "tasks/get" => match crate::tasks::tasks_get(
            params.as_ref(),
            &state.repo_root,
            crate::output::OutputFormat::Json,
        ) {
            Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
            Err(error) => respond_with_mode(response_mode, jsonrpc_envelope_task_error(id, error)),
        },
        "tasks/result" => match crate::tasks::tasks_result(
            params.as_ref(),
            &state.repo_root,
            crate::output::OutputFormat::Json,
        ) {
            Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
            Err(error) => respond_with_mode(response_mode, jsonrpc_envelope_task_error(id, error)),
        },
        "tasks/cancel" => match crate::tasks::tasks_cancel(
            params.as_ref(),
            &state.repo_root,
            crate::output::OutputFormat::Json,
        ) {
            Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
            Err(error) => respond_with_mode(response_mode, jsonrpc_envelope_task_error(id, error)),
        },
        "prompts/list" => respond_with_mode(
            response_mode,
            jsonrpc_envelope_result(id, crate::prompts::prompt_list()),
        ),
        "prompts/get" => {
            let name = match params
                .as_ref()
                .and_then(|value| value.get("name"))
                .and_then(|value| value.as_str())
            {
                Some(name) => name.to_owned(),
                None => {
                    return apply_origin_headers(
                        respond_with_mode(
                            response_mode,
                            jsonrpc_envelope_error(
                                id,
                                -32602,
                                "missing prompt name".to_owned(),
                                None,
                            ),
                        ),
                        origin.as_deref(),
                    );
                }
            };
            let prompt_args = params
                .as_ref()
                .and_then(|value| value.get("arguments"))
                .cloned();
            match crate::prompts::prompt_get(&name, prompt_args.as_ref()) {
                Ok(result) => respond_with_mode(response_mode, jsonrpc_envelope_result(id, result)),
                Err(error) => respond_with_mode(
                    response_mode,
                    jsonrpc_envelope_error(id, -32603, error.to_string(), None),
                ),
            }
        }
        other => respond_with_mode(
            response_mode,
            jsonrpc_envelope_error(id, -32601, format!("method not found: {other}"), None),
        ),
    };
    apply_origin_headers(response, origin.as_deref())
}

async fn dispatch_tool_call(
    state: AppState,
    id: Value,
    params: Option<Value>,
    response_mode: ResponseMode,
    principal: Option<String>,
) -> Response {
    let request = crate::transport::RequestLogContext {
        request_id: match &id {
            Value::String(value) => value.clone(),
            _ => id.to_string(),
        },
        method: "tools/call".to_owned(),
        tool_name: params
            .as_ref()
            .and_then(|value| value.get("name"))
            .and_then(|value| value.as_str())
            .map(str::to_owned),
    };
    let Some(params) = params else {
        crate::transport::log_protocol_error_observation(
            "http",
            &request,
            "invalid_params",
            "missing tools/call params object",
        );
        return respond_with_mode(
            response_mode,
            jsonrpc_envelope_error(
                id,
                -32602,
                "missing tools/call params object".to_owned(),
                None,
            ),
        );
    };
    let (name, args, progress_token) = {
        let Some(params_object) = params.as_object() else {
            crate::transport::log_protocol_error_observation(
                "http",
                &request,
                "invalid_params",
                "tools/call params must be an object",
            );
            return respond_with_mode(
                response_mode,
                jsonrpc_envelope_error(
                    id,
                    -32602,
                    "tools/call params must be an object".to_owned(),
                    None,
                ),
            );
        };
        let Some(name) = params_object.get("name").and_then(|value| value.as_str()) else {
            crate::transport::log_protocol_error_observation(
                "http",
                &request,
                "invalid_params",
                "missing tool name",
            );
            return respond_with_mode(
                response_mode,
                jsonrpc_envelope_error(id, -32602, "missing tool name".to_owned(), None),
            );
        };
        if !crate::tools::is_known_tool_name(name) {
            crate::transport::log_protocol_error_observation(
                "http",
                &request,
                "method_not_found",
                &format!("unknown tool: {name}"),
            );
            return respond_with_mode(
                response_mode,
                jsonrpc_envelope_error(id, -32601, format!("unknown tool: {name}"), None),
            );
        }
        let args = match params_object.get("arguments") {
            None | Some(Value::Null) => None,
            Some(value) if value.is_object() => {
                Some(crate::transport::repo_selection::strip_repo_selector_fields(value.clone()))
            }
            Some(_) => {
                crate::transport::log_protocol_error_observation(
                    "http",
                    &request,
                    "invalid_params",
                    "tools/call arguments must be an object when provided",
                );
                return respond_with_mode(
                    response_mode,
                    jsonrpc_envelope_error(
                        id,
                        -32602,
                        "tools/call arguments must be an object when provided".to_owned(),
                        None,
                    ),
                );
            }
        };
        let progress_token = params_object
            .get("_meta")
            .and_then(|meta| meta.get("progressToken"))
            .or_else(|| params_object.get("progressToken"))
            .cloned();
        (name.to_owned(), args, progress_token)
    };
    let repo_root = Arc::clone(&state.repo_root);
    let db_path = Arc::clone(&state.db_path);
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_worker = Arc::clone(&cancel_flag);
    let interaction_capabilities = crate::transport::parse_client_interaction_capabilities(
        params
            .get("_meta")
            .and_then(|meta| meta.get(crate::spec::META_CLIENT_CAPABILITIES))
            .unwrap_or(&Value::Null),
    );
    let request_id_for_context = match &id {
        Value::String(value) => value.clone(),
        _ => id.to_string(),
    };
    let sse_events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sse_events_for_worker = Arc::clone(&sse_events);

    let result = tokio::task::spawn_blocking(move || {
        let request_context = crate::runtime_context::RequestContext::new(
            Arc::new({
                let sse_events = Arc::clone(&sse_events_for_worker);
                move |params| {
                    if response_mode == ResponseMode::Sse {
                        sse_events
                            .lock()
                            .expect("http sse event lock poisoned")
                            .push(serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/tasks/status",
                                "params": params,
                            }));
                    }
                    Ok(())
                }
            }),
            interaction_capabilities.clone(),
            "http",
            None,
            principal.clone(),
            request_id_for_context.clone(),
            "tools/call",
            Some(params.clone()),
        );
        crate::runtime_context::install(request_context);
        crate::tasks::install_tool_call_request_params(Some(&params));
        crate::progress::install(
            move |message, percentage| {
                if response_mode == ResponseMode::Sse {
                    let mut params = serde_json::json!({
                        "token": progress_token,
                        "value": {
                            "kind": "report",
                            "message": message,
                        }
                    });
                    if let Some(pct) = percentage {
                        params["value"]["percentage"] = serde_json::json!(pct);
                    }
                    sse_events_for_worker
                        .lock()
                        .expect("http sse event lock poisoned")
                        .push(serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "$/progress",
                            "params": params,
                        }));
                }
            },
            cancel_flag_worker,
        );
        let call_result = crate::tasks::execute_tool_call(&name, args, &repo_root, &db_path);
        crate::progress::uninstall();
        crate::tasks::uninstall_tool_call_request_params();
        crate::runtime_context::uninstall();
        call_result.map_err(|error| error.to_string())
    })
    .await;

    match result {
        Ok(Ok(tool_result)) => {
            let is_tool_error = tool_result.get("isError").and_then(Value::as_bool) == Some(true);
            if is_tool_error {
                crate::transport::log_tool_execution_error_observation(
                    "http",
                    &request,
                    &tool_result,
                );
            }
            if response_mode == ResponseMode::Sse {
                let mut events = take_sse_events(&sse_events);
                events.push(jsonrpc_envelope_result(id, tool_result));
                sse_json_response(events)
            } else {
                jsonrpc_ok_response(id, tool_result)
            }
        }
        Ok(Err(error_message)) => {
            if response_mode == ResponseMode::Sse {
                let mut events = take_sse_events(&sse_events);
                events.push(jsonrpc_envelope_error(id, -32001, error_message, None));
                sse_json_response(events)
            } else {
                jsonrpc_error_response(id, -32001, error_message)
            }
        }
        Err(join_error) => {
            let message = format!("worker panicked: {join_error}");
            if response_mode == ResponseMode::Sse {
                let mut events = take_sse_events(&sse_events);
                events.push(jsonrpc_envelope_error(id, -32603, message, None));
                sse_json_response(events)
            } else {
                jsonrpc_error_response(id, -32603, message)
            }
        }
    }
}

fn take_sse_events(events: &Arc<Mutex<Vec<Value>>>) -> Vec<Value> {
    std::mem::take(&mut *events.lock().expect("http sse event lock poisoned"))
}

fn negotiate_response_mode(
    headers: &HeaderMap,
) -> std::result::Result<ResponseMode, Box<Response>> {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            Box::new(protocol_error_response(
                StatusCode::NOT_ACCEPTABLE,
                "missing required Accept header".to_owned(),
            ))
        })?;
    if accept_allows(&accept, "application/json") || accept_allows(&accept, "*/*") {
        return Ok(ResponseMode::Json);
    }
    if accept_allows(&accept, "text/event-stream") {
        return Ok(ResponseMode::Sse);
    }
    Err(Box::new(protocol_error_response(
        StatusCode::NOT_ACCEPTABLE,
        "Accept must allow application/json or text/event-stream".to_owned(),
    )))
}

fn accept_allows(accept: &str, expected: &str) -> bool {
    accept
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .any(|value| value.split(';').next().map(str::trim) == Some(expected))
}

fn validate_header_mirrors(
    headers: &HeaderMap,
    id: &Value,
    method: &str,
    params: Option<&Value>,
) -> std::result::Result<(), Box<Response>> {
    let header_method = required_header_value(headers, MCP_METHOD_HEADER)?;
    if header_method != method {
        return Err(Box::new(header_mismatch_response(
            id.clone(),
            MCP_METHOD_HEADER,
            &header_method,
            "method",
            method,
        )));
    }

    if method_requires_name_header(method) {
        let body_name = params
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Box::new(jsonrpc_error_response(
                    id.clone(),
                    -32020,
                    format!("missing {MCP_NAME_HEADER} header or body params.name"),
                ))
            })?;
        let header_name = decode_mirrored_header_value(
            headers
                .get(MCP_NAME_HEADER)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    Box::new(jsonrpc_error_response(
                        id.clone(),
                        -32020,
                        format!("missing required {MCP_NAME_HEADER} header"),
                    ))
                })?,
        )
        .map_err(|error| {
            Box::new(jsonrpc_error_response(
                id.clone(),
                -32602,
                error.to_string(),
            ))
        })?;
        if header_name != body_name {
            return Err(Box::new(header_mismatch_response(
                id.clone(),
                MCP_NAME_HEADER,
                &header_name,
                "params.name",
                body_name,
            )));
        }
    }

    let Some(params) = params.and_then(Value::as_object) else {
        return Ok(());
    };
    for (header_name, header_value) in headers {
        let header_name = header_name.as_str().to_ascii_lowercase();
        let Some(param_name) = header_name.strip_prefix(MCP_PARAM_HEADER_PREFIX) else {
            continue;
        };
        let header_value = header_value.to_str().map_err(|_| {
            Box::new(jsonrpc_error_response(
                id.clone(),
                -32602,
                format!("invalid utf-8 in header {header_name}"),
            ))
        })?;
        let decoded = decode_mirrored_header_value(header_value).map_err(|error| {
            Box::new(jsonrpc_error_response(
                id.clone(),
                -32602,
                error.to_string(),
            ))
        })?;
        let Some(body_value) = params.get(param_name) else {
            return Err(Box::new(jsonrpc_error_response(
                id.clone(),
                -32020,
                format!("header/body mismatch for {header_name}: body params.{param_name} missing"),
            )));
        };
        let body_rendered = json_value_for_header_compare(body_value);
        if decoded != body_rendered {
            return Err(Box::new(header_mismatch_response(
                id.clone(),
                &header_name,
                &decoded,
                &format!("params.{param_name}"),
                &body_rendered,
            )));
        }
    }
    Ok(())
}

fn method_requires_name_header(method: &str) -> bool {
    matches!(method, "tools/call" | "prompts/get")
}

fn required_header_value(
    headers: &HeaderMap,
    name: &str,
) -> std::result::Result<String, Box<Response>> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            Box::new(protocol_error_response(
                StatusCode::BAD_REQUEST,
                format!("missing required {name} header"),
            ))
        })
}

fn decode_mirrored_header_value(value: &str) -> Result<String> {
    for sentinel in BASE64_SENTINELS {
        if let Some(encoded) = value.strip_prefix(sentinel) {
            let bytes = decode_base64(encoded)?;
            return String::from_utf8(bytes)
                .context("Base64 sentinel decoded non-utf8 header value");
        }
    }
    Ok(value.to_owned())
}

fn decode_base64(input: &str) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for ch in input.chars().filter(|ch| !ch.is_whitespace()) {
        if ch == '=' {
            break;
        }
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => 26 + (ch as u32 - 'a' as u32),
            '0'..='9' => 52 + (ch as u32 - '0' as u32),
            '+' | '-' => 62,
            '/' | '_' => 63,
            _ => return Err(anyhow!("invalid Base64 sentinel character '{ch}'")),
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        while bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Ok(output)
}

fn json_value_for_header_compare(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => "null".to_owned(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn header_mismatch_response(
    id: Value,
    header_name: &str,
    header_value: &str,
    body_field: &str,
    body_value: &str,
) -> Response {
    jsonrpc_error_with_data_response(
        id,
        -32020,
        format!(
            "header/body mismatch for {header_name}: header='{header_value}' body='{body_value}'"
        ),
        Some(serde_json::json!({
            "header": header_name,
            "headerValue": header_value,
            "bodyField": body_field,
            "bodyValue": body_value,
        })),
    )
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

fn authorize_request(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<AuthorizedRequest, Box<Response>> {
    let Some(policy) = state.auth_policy.as_ref() else {
        return Ok(AuthorizedRequest { subject: None });
    };
    match policy.authorize(headers, auth::ROUTE_FAMILY_MCP) {
        Ok(token) => Ok(AuthorizedRequest {
            subject: token.subject,
        }),
        Err(challenge) => Err(Box::new(auth_challenge_response(challenge))),
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

fn respond_with_mode(mode: ResponseMode, envelope: Value) -> Response {
    match mode {
        ResponseMode::Json => json_response_from_envelope(envelope),
        ResponseMode::Sse => sse_json_response(vec![envelope]),
    }
}

fn dispatch_subscriptions_listen(
    id: Value,
    params: Option<Value>,
    response_mode: ResponseMode,
) -> Response {
    if response_mode != ResponseMode::Sse {
        return protocol_error_response(
            StatusCode::NOT_ACCEPTABLE,
            "subscriptions/listen requires Accept: text/event-stream".to_owned(),
        );
    }

    let subscription_id = format!("sub-{}", request_id_fragment(&id));
    let selected = match parse_subscription_types(params.as_ref()) {
        Ok(selected) => selected,
        Err(error) => return jsonrpc_error_response(id, -32602, error.to_string()),
    };

    let mut result = serde_json::json!({
        "subscriptionId": subscription_id,
        "selectedNotificationTypes": selected.iter().cloned().collect::<Vec<_>>(),
    });
    result = spec::complete_result(result);
    let mut events = vec![jsonrpc_envelope_result(id, result)];
    for event in bootstrap_subscription_notifications(&selected, &subscription_id) {
        events.push(event);
    }
    sse_json_response(events)
}

fn request_id_fragment(id: &Value) -> String {
    match id {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => "listen".to_owned(),
    }
}

fn parse_subscription_types(params: Option<&Value>) -> Result<BTreeSet<String>> {
    let requested = params.and_then(|value| {
        value
            .get("notificationTypes")
            .or_else(|| value.get("types"))
    });
    let mut selected = BTreeSet::new();
    match requested {
        None | Some(Value::Null) => {
            for value in supported_subscription_types() {
                selected.insert(value.to_owned());
            }
        }
        Some(Value::Array(values)) => {
            for value in values {
                let raw = value.as_str().ok_or_else(|| {
                    anyhow!("subscriptions/listen notificationTypes entries must be strings")
                })?;
                for expanded in expand_subscription_type(raw)? {
                    selected.insert(expanded.to_owned());
                }
            }
        }
        Some(_) => {
            return Err(anyhow!(
                "subscriptions/listen notificationTypes must be an array of strings"
            ));
        }
    }
    Ok(selected)
}

fn supported_subscription_types() -> [&'static str; 4] {
    [
        SUBSCRIPTION_TYPE_PROMPTS_LIST_CHANGED,
        SUBSCRIPTION_TYPE_RESOURCES_LIST_CHANGED,
        SUBSCRIPTION_TYPE_RESOURCE_UPDATED,
        SUBSCRIPTION_TYPE_TOOLS_LIST_CHANGED,
    ]
}

fn expand_subscription_type(raw: &str) -> Result<Vec<&'static str>> {
    match raw {
        "tools" | SUBSCRIPTION_TYPE_TOOLS_LIST_CHANGED => {
            Ok(vec![SUBSCRIPTION_TYPE_TOOLS_LIST_CHANGED])
        }
        "prompts" | SUBSCRIPTION_TYPE_PROMPTS_LIST_CHANGED => {
            Ok(vec![SUBSCRIPTION_TYPE_PROMPTS_LIST_CHANGED])
        }
        "resources" => Ok(vec![
            SUBSCRIPTION_TYPE_RESOURCES_LIST_CHANGED,
            SUBSCRIPTION_TYPE_RESOURCE_UPDATED,
        ]),
        SUBSCRIPTION_TYPE_RESOURCES_LIST_CHANGED => {
            Ok(vec![SUBSCRIPTION_TYPE_RESOURCES_LIST_CHANGED])
        }
        "resource_subscriptions" | SUBSCRIPTION_TYPE_RESOURCE_UPDATED => {
            Ok(vec![SUBSCRIPTION_TYPE_RESOURCE_UPDATED])
        }
        other => Err(anyhow!(
            "unsupported subscriptions/listen notification type: {other}"
        )),
    }
}

fn bootstrap_subscription_notifications(
    selected: &BTreeSet<String>,
    subscription_id: &str,
) -> Vec<Value> {
    let mut events = Vec::new();
    for kind in selected {
        let params = match kind.as_str() {
            SUBSCRIPTION_TYPE_TOOLS_LIST_CHANGED => serde_json::json!({
                "reason": "subscription_opened",
                "toolCount": tools::tool_list()["tools"].as_array().map(|items| items.len()).unwrap_or_default(),
            }),
            SUBSCRIPTION_TYPE_PROMPTS_LIST_CHANGED => serde_json::json!({
                "reason": "subscription_opened",
                "promptCount": crate::prompts::prompt_list()["prompts"].as_array().map(|items| items.len()).unwrap_or_default(),
            }),
            SUBSCRIPTION_TYPE_RESOURCES_LIST_CHANGED => serde_json::json!({
                "reason": "subscription_opened",
                "resourceCount": resources::resource_descriptors().len(),
                "resourceTemplateCount": resources::resource_template_descriptors().len(),
            }),
            SUBSCRIPTION_TYPE_RESOURCE_UPDATED => serde_json::json!({
                "uri": "atlas://docs/index",
                "reason": "subscription_opened",
            }),
            _ => continue,
        };
        events.push(subscription_notification(kind, params, subscription_id));
    }
    events
}

fn subscription_notification(method: &str, params: Value, subscription_id: &str) -> Value {
    let mut params = params;
    if let Some(object) = params.as_object_mut() {
        object.insert(
            "_meta".to_owned(),
            serde_json::json!({ SUBSCRIPTION_ID_META_KEY: subscription_id }),
        );
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

fn sse_json_response(events: Vec<Value>) -> Response {
    let mut body = String::new();
    for event in events {
        body.push_str("event: message\n");
        body.push_str("data: ");
        body.push_str(&serde_json::to_string(&event).expect("SSE event serialization"));
        body.push_str("\n\n");
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(MCP_PROTOCOL_VERSION_HEADER, spec::MCP_PROTOCOL_VERSION)
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn json_response_from_envelope(envelope: Value) -> Response {
    let status = http_status_for_envelope(&envelope);
    let mut response = Json(envelope).into_response();
    *response.status_mut() = status;
    response.headers_mut().insert(
        MCP_PROTOCOL_VERSION_HEADER,
        HeaderValue::from_static(spec::MCP_PROTOCOL_VERSION),
    );
    response
}

fn http_status_for_envelope(envelope: &Value) -> StatusCode {
    let Some(code) = envelope
        .get("error")
        .and_then(|value| value.get("code"))
        .and_then(Value::as_i64)
    else {
        return StatusCode::OK;
    };
    match code as i32 {
        -32700 | -32600 | -32602 | -32020 | -32022 => StatusCode::BAD_REQUEST,
        -32601 | -32010 => StatusCode::NOT_FOUND,
        -32013..=-32011 => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn jsonrpc_envelope_result(id: Value, result: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": spec::complete_result(result),
    })
}

fn jsonrpc_envelope_error(
    id: Value,
    code: i32,
    message: String,
    extra_data: Option<Value>,
) -> Value {
    let atlas_error_code = match code {
        -32700 => "parse_error",
        -32600 => "invalid_request",
        -32601 => "method_not_found",
        -32602 => "invalid_params",
        -32603 => "internal_error",
        -32001 => "tool_execution_failed",
        -32010 => "task_not_found",
        -32011 => "task_not_ready",
        -32012 => "task_cancelled",
        -32013 => "task_failed",
        -32020 => "header_mismatch",
        -32022 => "unsupported_protocol_version",
        _ => "internal_error",
    };
    let mut data = serde_json::json!({
        "atlas_error_code": atlas_error_code,
        "atlas_error_code_docs": error_code_docs_ref(atlas_error_code)
    });
    if let Some(extra) = extra_data
        && let (Some(base), Some(extra)) = (data.as_object_mut(), extra.as_object())
    {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": data,
        }
    })
}

fn jsonrpc_envelope_task_error(id: Value, error: crate::tasks::TaskApiError) -> Value {
    let code = match error.kind() {
        crate::tasks::TaskApiErrorKind::InvalidParams => -32602,
        crate::tasks::TaskApiErrorKind::NotFound => -32010,
        crate::tasks::TaskApiErrorKind::NotReady => -32011,
        crate::tasks::TaskApiErrorKind::Cancelled => -32012,
        crate::tasks::TaskApiErrorKind::Failed => -32013,
        crate::tasks::TaskApiErrorKind::Internal => -32603,
    };
    jsonrpc_envelope_error(id, code, error.message(), None)
}

fn jsonrpc_raw_ok_response(id: Value, result: Value) -> Response {
    json_response_from_envelope(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
}

fn jsonrpc_ok_response(id: Value, result: Value) -> Response {
    jsonrpc_raw_ok_response(id, spec::complete_result(result))
}

fn jsonrpc_error_response(id: Value, code: i32, message: String) -> Response {
    jsonrpc_error_with_data_response(id, code, message, None)
}

fn jsonrpc_error_with_data_response(
    id: Value,
    code: i32,
    message: String,
    extra_data: Option<Value>,
) -> Response {
    json_response_from_envelope(jsonrpc_envelope_error(id, code, message, extra_data))
}

fn protocol_error_response(status: StatusCode, message: String) -> Response {
    let mut response = jsonrpc_error_response(Value::Null, -32600, message);
    *response.status_mut() = status;
    response
}

fn apply_origin_headers(mut response: Response, origin: Option<&str>) -> Response {
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(
            "Authorization, Content-Type, Accept, MCP-Protocol-Version, Mcp-Method, Mcp-Name, Mcp-Param-*, Mcp-Session-Id, Last-Event-ID",
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
    use axum::body::to_bytes;
    use serde_json::json;
    use std::collections::HashMap;

    const TEST_SECRET_B64U: &str = "YXRsYXMtbWNwLXBoYXNlNS1zZWNyZXQ";

    fn make_state() -> AppState {
        AppState {
            repo_root: Arc::new("repo".to_owned()),
            db_path: Arc::new("db".to_owned()),
            auth_policy: None,
            allowed_origins: Arc::new(HashSet::new()),
        }
    }

    async fn make_state_with_auth(origins: &[&str]) -> AppState {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock auth server");
        let addr = listener.local_addr().expect("mock auth addr");
        let base_url = format!("http://{addr}");
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
                get(move || {
                    let discovery = discovery.clone();
                    async move { discovery }
                }),
            )
            .route(
                "/jwks",
                get(move || {
                    let jwks = jwks.clone();
                    async move { jwks }
                }),
            );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
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
            ..make_state()
        }
    }

    fn base_headers(method: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            HeaderValue::from_static(spec::MCP_PROTOCOL_VERSION),
        );
        headers.insert(
            MCP_METHOD_HEADER,
            HeaderValue::from_str(method).expect("method header"),
        );
        headers
    }

    fn sse_headers(method: &str) -> HeaderMap {
        let mut headers = base_headers(method);
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
        headers
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

    async fn request_router(method: axum::http::Method, path: &str) -> Response {
        use tower::util::ServiceExt;
        let app = build_router(make_state());
        app.oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .body(axum::body::Body::empty())
                .expect("request builder"),
        )
        .await
        .expect("router response")
    }

    #[tokio::test]
    async fn metadata_endpoint_returns_protected_resource_body_shape() {
        let state = make_state_with_auth(&[]).await;
        let response = handle_protected_resource_metadata(State(state), HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = read_json_response(response).await;
        assert_eq!(value["resource"], json!("https://atlas.test/mcp"));
        assert_eq!(value["bearer_methods_supported"], json!(["header"]));
    }

    #[tokio::test]
    async fn missing_bearer_token_returns_www_authenticate() {
        let state = make_state_with_auth(&[]).await;
        let response = handle_post_mcp(
            State(state),
            base_headers("initialize"),
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"zed","version":"1.0.0"}}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get(header::WWW_AUTHENTICATE).is_some());
    }

    #[tokio::test]
    async fn http_initialize_returns_no_session_headers() {
        let response = handle_post_mcp(
            State(make_state()),
            base_headers("initialize"),
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"zed","version":"1.0.0"}}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
        let value = read_json_response(response).await;
        assert_eq!(
            value["result"]["protocolVersion"],
            json!(spec::MCP_PROTOCOL_VERSION)
        );
    }

    #[tokio::test]
    async fn post_requires_accept_header_support() {
        let mut headers = HeaderMap::new();
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            HeaderValue::from_static(spec::MCP_PROTOCOL_VERSION),
        );
        headers.insert(MCP_METHOD_HEADER, HeaderValue::from_static("tools/list"));
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
        let value = read_json_response(response).await;
        assert_eq!(value["error"]["code"], json!(-32600));
    }

    #[tokio::test]
    async fn post_requires_protocol_and_method_headers() {
        let response = handle_post_mcp(
            State(make_state()),
            HeaderMap::new(),
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);

        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = read_json_response(response).await;
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("MCP-Protocol-Version")
        );
    }

    #[tokio::test]
    async fn header_mismatch_uses_code_minus_32020() {
        let headers = base_headers("tools/list");
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(
                br#"{"jsonrpc":"2.0","id":1,"method":"resources/list","params":{}}"#,
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = read_json_response(response).await;
        assert_eq!(value["error"]["code"], json!(-32020));
        assert_eq!(
            value["error"]["data"]["atlas_error_code"],
            json!("header_mismatch")
        );
    }

    #[tokio::test]
    async fn tools_call_requires_matching_name_header() {
        let mut headers = base_headers("tools/call");
        headers.insert(MCP_NAME_HEADER, HeaderValue::from_static("wrong_tool"));
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"broker_status","arguments":{}}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = read_json_response(response).await;
        assert_eq!(value["error"]["code"], json!(-32020));
    }

    #[tokio::test]
    async fn base64_sentinel_name_header_is_decoded() {
        let mut headers = base_headers("prompts/get");
        headers.insert(
            MCP_NAME_HEADER,
            HeaderValue::from_static("base64:cmV2aWV3X2NoYW5nZQ=="),
        );
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"prompts/get","params":{"name":"review_change","arguments":{"files":"src/lib.rs"}}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_methods_return_404_plus_jsonrpc_method_not_found() {
        let headers = base_headers("bogus/method");
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"bogus/method","params":{}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let value = read_json_response(response).await;
        assert_eq!(value["error"]["code"], json!(-32601));
    }

    #[tokio::test]
    async fn get_and_delete_mcp_routes_are_removed() {
        assert_eq!(
            request_router(axum::http::Method::GET, "/mcp")
                .await
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            request_router(axum::http::Method::DELETE, "/mcp")
                .await
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }

    #[tokio::test]
    async fn session_and_resume_headers_are_ignored_on_post() {
        let mut headers = base_headers("tools/list");
        headers.insert(
            MCP_SESSION_ID_HEADER,
            HeaderValue::from_static("stale-session"),
        );
        headers.insert(
            LAST_EVENT_ID_HEADER,
            HeaderValue::from_static("stale-event"),
        );
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
    }

    #[tokio::test]
    async fn post_can_return_non_resumable_sse() {
        let headers = sse_headers("tools/list");
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );
        assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
        let body = read_body_string(response).await;
        assert!(body.contains("event: message"));
        assert!(body.contains("\"result\""));
        assert!(!body.contains("id: "));
    }

    #[tokio::test]
    async fn subscriptions_listen_requires_sse_accept() {
        let headers = base_headers("subscriptions/listen");
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":1,"method":"subscriptions/listen","params":{"notificationTypes":["tools"]}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_ACCEPTABLE);
        let value = read_json_response(response).await;
        assert_eq!(value["error"]["code"], json!(-32600));
    }

    #[tokio::test]
    async fn subscriptions_listen_filters_notifications_and_tags_subscription_id() {
        let headers = sse_headers("subscriptions/listen");
        let response = handle_post_mcp(
            State(make_state()),
            headers,
            Bytes::from_static(br#"{"jsonrpc":"2.0","id":7,"method":"subscriptions/listen","params":{"notificationTypes":["tools","resource_subscriptions"]}}"#),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = read_body_string(response).await;
        assert!(body.contains("\"subscriptionId\":\"sub-7\""));
        assert!(body.contains(SUBSCRIPTION_TYPE_TOOLS_LIST_CHANGED));
        assert!(body.contains(SUBSCRIPTION_TYPE_RESOURCE_UPDATED));
        assert!(!body.contains(SUBSCRIPTION_TYPE_PROMPTS_LIST_CHANGED));
        assert!(!body.contains(SUBSCRIPTION_TYPE_RESOURCES_LIST_CHANGED));
        assert!(body.contains(SUBSCRIPTION_ID_META_KEY));
        assert!(!body.contains("$/progress"));
        assert!(!body.contains("notifications/tasks/status"));
    }

    #[tokio::test]
    async fn removed_resource_subscription_methods_return_method_not_found() {
        for method in ["resources/subscribe", "resources/unsubscribe"] {
            let headers = base_headers(method);
            let body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": {"uri": "atlas://docs/index"}
            });
            let response = handle_post_mcp(
                State(make_state()),
                headers,
                Bytes::from(serde_json::to_vec(&body).expect("serialize body")),
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            let value = read_json_response(response).await;
            assert_eq!(value["error"]["code"], json!(-32601));
        }
    }
}
