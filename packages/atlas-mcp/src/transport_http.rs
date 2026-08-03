//! Streamable HTTP transport for atlas-mcp.
//!
//! Enabled with `http-transport` Cargo feature.
//!
//! Routes:
//! - `POST /mcp` — MCP ingress handled by official `rmcp` streamable HTTP service
//! - `GET /health` — unauthenticated liveness probe
//! - `GET /.well-known/oauth-protected-resource` — OAuth protected-resource metadata

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, session::local::LocalSessionManager, tower::StreamableHttpService,
};
use serde_json::Value;

use crate::auth::{self, ProtectedResourceAuthPolicy};
use crate::rmcp_server::{AtlasRmcpServer, AuthenticatedPrincipal};
use crate::spec;
use crate::tools::health::mark_server_started;
use crate::transport::ServerOptions;

const DEFAULT_BIND: &str = "127.0.0.1:7070";
const MCP_PROTOCOL_VERSION_HEADER: &str = "MCP-Protocol-Version";
const MCP_METHOD_HEADER: &str = "Mcp-Method";
const MCP_NAME_HEADER: &str = "Mcp-Name";
#[cfg(test)]
const MCP_SESSION_ID_HEADER: &str = "Mcp-Session-Id";
const PROTECTED_RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

#[derive(Clone)]
struct AppState {
    repo_root: Arc<String>,
    db_path: Arc<String>,
    server_options: ServerOptions,
    auth_policy: Option<Arc<ProtectedResourceAuthPolicy>>,
    allowed_origins: Arc<HashSet<String>>,
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
    run_http_server_with_options(repo_root, db_path, ServerOptions::default())
}

pub fn run_http_server_with_options(
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
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
            let app = build_router(app_state(repo_root, db_path, options).await?);
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
                server_options: ServerOptions::default(),
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
                server_options: ServerOptions::default(),
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
        let body = augment_test_request_body(body.clone());
        maybe_insert_test_request_headers(&mut headers, &body)?;
        let request = axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri("/mcp")
            .body(axum::body::Body::from(serde_json::to_vec(&body)?))
            .expect("http test request");
        let (mut parts, body) = request.into_parts();
        parts.headers = headers;
        let request = axum::http::Request::from_parts(parts, body);
        let response = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("cannot build tokio runtime for HTTP test harness")?
            .block_on(async move {
                use tower::util::ServiceExt;
                build_router(self.state.clone())
                    .oneshot(request)
                    .await
                    .expect("router response")
            });
        response_to_test_output(response)
    }

    pub fn get_metadata(&self, headers: &[(&str, &str)]) -> Result<TestHttpResponse> {
        let headers = make_test_header_map(headers)?;
        let request = axum::http::Request::builder()
            .method(axum::http::Method::GET)
            .uri(PROTECTED_RESOURCE_METADATA_PATH)
            .body(axum::body::Body::empty())
            .expect("metadata request");
        let (mut parts, body) = request.into_parts();
        parts.headers = headers;
        let request = axum::http::Request::from_parts(parts, body);
        let response = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("cannot build tokio runtime for HTTP test harness")?
            .block_on(async move {
                use tower::util::ServiceExt;
                build_router(self.state.clone())
                    .oneshot(request)
                    .await
                    .expect("router response")
            });
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

async fn app_state(repo_root: &str, db_path: &str, options: ServerOptions) -> Result<AppState> {
    let auth_policy = match options.http_auth.clone() {
        Some(config) => Some(Arc::new(ProtectedResourceAuthPolicy::load(config).await?)),
        None => None,
    };
    let allowed_origins = Arc::new(
        auth_policy
            .as_ref()
            .map(|policy| policy.allowed_origins().iter().cloned().collect())
            .unwrap_or_default(),
    );
    Ok(AppState {
        repo_root: Arc::new(repo_root.to_owned()),
        db_path: Arc::new(db_path.to_owned()),
        server_options: options,
        auth_policy,
        allowed_origins,
    })
}

fn build_router(state: AppState) -> Router {
    let mcp_service = build_mcp_service(&state);
    let mcp_router =
        Router::new()
            .route_service("/mcp", mcp_service)
            .layer(middleware::from_fn_with_state(
                state.clone(),
                mcp_request_middleware,
            ));

    Router::new()
        .route("/health", get(handle_health))
        .route(
            PROTECTED_RESOURCE_METADATA_PATH,
            get(handle_protected_resource_metadata),
        )
        .merge(mcp_router)
        .with_state(state)
}

fn build_mcp_service(
    state: &AppState,
) -> StreamableHttpService<AtlasRmcpServer, LocalSessionManager> {
    let repo_root = state.repo_root.as_ref().clone();
    let db_path = state.db_path.as_ref().clone();
    let options = state.server_options.clone();
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None);
    StreamableHttpService::new(
        move || {
            Ok(AtlasRmcpServer::new(
                repo_root.clone(),
                db_path.clone(),
                options.clone(),
            ))
        },
        Default::default(),
        config,
    )
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

async fn mcp_request_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let origin = match validate_origin(&state, request.headers()) {
        Ok(origin) => origin,
        Err(response) => return *response,
    };

    if request.method() == axum::http::Method::POST {
        let authorized = match authorize_request(&state, request.headers()) {
            Ok(authorized) => authorized,
            Err(response) => return apply_origin_headers(*response, origin.as_deref()),
        };
        if let Some(subject) = authorized.subject {
            request
                .extensions_mut()
                .insert(AuthenticatedPrincipal(subject));
        }
    }

    let response = next.run(request).await;
    apply_origin_headers(response, origin.as_deref())
}

fn make_test_header_map(headers: &[(&str, &str)]) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            axum::http::header::HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(value)?,
        );
    }
    if !map.contains_key(header::HOST) {
        map.insert(header::HOST, HeaderValue::from_static("localhost"));
    }
    Ok(map)
}

fn default_request_meta_value() -> Value {
    serde_json::json!({
        spec::META_PROTOCOL_VERSION: spec::MCP_PROTOCOL_VERSION,
        spec::META_CLIENT_CAPABILITIES: {
            "elicitation": { "form": {}, "url": {} }
        },
        spec::META_CLIENT_INFO: { "name": "zed", "version": "1.0.0" }
    })
}

fn augment_test_request_body(mut body: Value) -> Value {
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if method == "initialize" {
        if let Some(params) = body.get_mut("params").and_then(Value::as_object_mut) {
            params.remove("_meta");
        }
        return body;
    }
    if matches!(
        method,
        "server/discover" | "initialized" | "notifications/initialized"
    ) || method.starts_with("notifications/")
    {
        return body;
    }
    if let Some(params) = body.get_mut("params").and_then(Value::as_object_mut) {
        params
            .entry("_meta".to_owned())
            .or_insert_with(default_request_meta_value);
    }
    body
}

fn maybe_insert_test_request_headers(headers: &mut HeaderMap, body: &Value) -> Result<()> {
    if !headers.contains_key(header::ACCEPT) {
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
    }
    if !headers.contains_key(header::CONTENT_TYPE) {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
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
    if !headers.contains_key(MCP_NAME_HEADER) {
        let mirrored_name = body
            .get("params")
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .or_else(|| {
                body.get("params")
                    .and_then(|value| value.get("uri"))
                    .and_then(Value::as_str)
            });
        if let Some(name) = mirrored_name {
            headers.insert(MCP_NAME_HEADER, HeaderValue::from_str(name)?);
        }
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
    let json_body = serde_json::from_str(&body_text).ok().or_else(|| {
        let messages = body_text
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .collect::<Vec<_>>();
        match messages.len() {
            0 => None,
            1 => messages.into_iter().next(),
            _ => messages.into_iter().last(),
        }
    });
    Ok(TestHttpResponse {
        status,
        headers,
        body_text,
        json_body,
    })
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

fn apply_origin_headers(mut response: Response, origin: Option<&str>) -> Response {
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST"),
    );
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(
            "Authorization, Content-Type, Accept, MCP-Protocol-Version, Mcp-Method, Mcp-Name, Mcp-Param-*",
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
    use serde_json::json;
    use std::collections::HashMap;

    const TEST_SECRET_B64U: &str = "YXRsYXMtbWNwLXBoYXNlNS1zZWNyZXQ";

    fn make_state() -> AppState {
        AppState {
            repo_root: Arc::new("repo".to_owned()),
            db_path: Arc::new("db".to_owned()),
            server_options: ServerOptions::default(),
            auth_policy: None,
            allowed_origins: Arc::new(HashSet::new()),
        }
    }

    #[test]
    fn cors_headers_do_not_advertise_removed_session_resume_headers() {
        let response =
            apply_origin_headers(StatusCode::OK.into_response(), Some("https://client.test"));
        let allowed_headers = response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|value| value.to_str().ok())
            .expect("allow headers");

        assert!(allowed_headers.contains("MCP-Protocol-Version"));
        assert!(!allowed_headers.contains("Mcp-Session-Id"));
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

    async fn request_router(
        state: AppState,
        method: axum::http::Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<Value>,
    ) -> Response {
        use tower::util::ServiceExt;

        let mut request = axum::http::Request::builder().method(method).uri(path);
        let mut has_host = false;
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("host") {
                has_host = true;
            }
            request = request.header(*name, *value);
        }
        if !has_host {
            request = request.header(header::HOST, "localhost");
        }
        let body = body
            .map(augment_test_request_body)
            .map(|value| serde_json::to_vec(&value).expect("serialize request body"))
            .unwrap_or_default();
        build_router(state)
            .oneshot(
                request
                    .body(axum::body::Body::from(body))
                    .expect("request builder"),
            )
            .await
            .expect("router response")
    }

    async fn read_json_response(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        serde_json::from_slice(&bytes).expect("response json")
    }

    async fn read_body_string(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }

    #[tokio::test]
    async fn metadata_endpoint_returns_protected_resource_body_shape() {
        let state = make_state_with_auth(&[]).await;
        let response = request_router(
            state,
            axum::http::Method::GET,
            PROTECTED_RESOURCE_METADATA_PATH,
            &[],
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = read_json_response(response).await;
        assert_eq!(value["resource"], json!("https://atlas.test/mcp"));
        assert_eq!(value["bearer_methods_supported"], json!(["header"]));
    }

    #[tokio::test]
    async fn missing_bearer_token_returns_www_authenticate() {
        let state = make_state_with_auth(&[]).await;
        let response = request_router(
            state,
            axum::http::Method::POST,
            "/mcp",
            &[
                ("Accept", "application/json, text/event-stream"),
                ("Content-Type", "application/json"),
                ("MCP-Protocol-Version", spec::MCP_PROTOCOL_VERSION),
                ("Mcp-Method", "initialize"),
            ],
            Some(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": spec::MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "zed", "version": "1.0.0" }
                }
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get(header::WWW_AUTHENTICATE).is_some());
    }

    #[tokio::test]
    async fn health_route_stays_unauthenticated() {
        let state = make_state_with_auth(&[]).await;
        let response = request_router(state, axum::http::Method::GET, "/health", &[], None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let value = read_json_response(response).await;
        assert_eq!(value["ok"], json!(true));
    }

    #[tokio::test]
    async fn http_initialize_returns_no_session_headers() {
        let response = request_router(
            make_state(),
            axum::http::Method::POST,
            "/mcp",
            &[
                ("Accept", "application/json, text/event-stream"),
                ("Content-Type", "application/json"),
                ("MCP-Protocol-Version", spec::MCP_PROTOCOL_VERSION),
                ("Mcp-Method", "initialize"),
            ],
            Some(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": spec::MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "zed", "version": "1.0.0" }
                }
            })),
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
    async fn mcp_post_json_response_uses_official_json_shape() {
        let response = request_router(
            make_state(),
            axum::http::Method::POST,
            "/mcp",
            &[
                ("Accept", "application/json, text/event-stream"),
                ("Content-Type", "application/json"),
                ("MCP-Protocol-Version", spec::MCP_PROTOCOL_VERSION),
                ("Mcp-Method", "tools/list"),
            ],
            Some(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(MCP_SESSION_ID_HEADER).is_none());
        assert!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .contains("application/json")
        );
        let value = read_json_response(response).await;
        assert!(value["result"]["tools"].is_array());
    }

    #[tokio::test]
    async fn mcp_post_can_fall_back_to_sse_for_subscription_stream() {
        let response = request_router(
            make_state(),
            axum::http::Method::POST,
            "/mcp",
            &[
                ("Accept", "application/json, text/event-stream"),
                ("Content-Type", "application/json"),
                ("MCP-Protocol-Version", spec::MCP_PROTOCOL_VERSION),
                ("Mcp-Method", "tools/call"),
                ("Mcp-Name", "__test_sleep"),
            ],
            Some(json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": {
                    "name": "__test_sleep",
                    "arguments": {
                        "sleep_ms": 20,
                        "chunk_ms": 10,
                        "report_progress": true
                    },
                    "_meta": {
                        spec::META_PROTOCOL_VERSION: spec::MCP_PROTOCOL_VERSION,
                        spec::META_CLIENT_CAPABILITIES: {},
                        spec::META_CLIENT_INFO: { "name": "zed", "version": "1.0.0" },
                        "progressToken": "progress-test-1"
                    }
                }
            })),
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
        let body = read_body_string(response).await;
        assert!(body.contains("data:"));
        assert!(body.contains("notifications/progress"));
        assert!(body.contains("\"result\""));
    }

    #[tokio::test]
    async fn allowed_origin_is_enforced_before_mcp_dispatch() {
        let state = make_state_with_auth(&["https://allowed.test"]).await;
        let response = request_router(
            state,
            axum::http::Method::POST,
            "/mcp",
            &[
                ("Origin", "https://blocked.test"),
                ("Accept", "application/json, text/event-stream"),
                ("Content-Type", "application/json"),
                ("MCP-Protocol-Version", spec::MCP_PROTOCOL_VERSION),
                ("Mcp-Method", "tools/list"),
            ],
            Some(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let value = read_json_response(response).await;
        assert_eq!(value["error"], json!("forbidden"));
    }

    #[tokio::test]
    async fn metadata_endpoint_applies_origin_headers_for_allowed_origin() {
        let state = make_state_with_auth(&["https://client.test"]).await;
        let response = request_router(
            state,
            axum::http::Method::GET,
            PROTECTED_RESOURCE_METADATA_PATH,
            &[("Origin", "https://client.test")],
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .and_then(|value| value.to_str().ok()),
            Some("https://client.test")
        );
    }

    #[tokio::test]
    async fn get_and_delete_mcp_routes_are_removed() {
        assert_eq!(
            request_router(make_state(), axum::http::Method::GET, "/mcp", &[], None)
                .await
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            request_router(make_state(), axum::http::Method::DELETE, "/mcp", &[], None)
                .await
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}
