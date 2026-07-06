use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::http::{HeaderMap, StatusCode, header};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode, decode_header};
use serde::Deserialize;
use serde_json::{Value, json};

pub const ROUTE_FAMILY_MCP: &str = "mcp";
const OIDC_DISCOVERY_PATH: &str = "/.well-known/openid-configuration";
const DEFAULT_AUTH_TIMEOUT_SECS: u64 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedResourceAuthConfig {
    pub issuer: String,
    pub discovery_url: Option<String>,
    pub jwks_url: Option<String>,
    pub resource: String,
    pub required_scopes: HashMap<String, Vec<String>>,
    pub allowed_origins: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ProtectedResourceAuthPolicy {
    issuer: String,
    resource: String,
    required_scopes: HashMap<String, Vec<String>>,
    scopes_supported: Vec<String>,
    allowed_origins: HashSet<String>,
    jwks: JwkSet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedToken {
    pub scopes: BTreeSet<String>,
    pub subject: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthChallenge {
    pub status: StatusCode,
    pub body_error: &'static str,
    pub body_message: String,
    pub error_code: &'static str,
    pub required_scopes: Vec<String>,
    pub www_authenticate: String,
}

#[derive(Debug, Deserialize)]
struct DiscoveryDocument {
    issuer: Option<String>,
    jwks_uri: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct TokenClaims {
    iss: String,
    aud: Option<Value>,
    exp: usize,
    nbf: Option<usize>,
    iat: Option<usize>,
    sub: Option<String>,
    scope: Option<String>,
    scp: Option<Value>,
}

impl ProtectedResourceAuthPolicy {
    pub async fn load(config: ProtectedResourceAuthConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_AUTH_TIMEOUT_SECS))
            .build()
            .context("cannot build HTTP client for MCP auth discovery")?;

        let jwks_url = match config.jwks_url.clone() {
            Some(url) => url,
            None => {
                let discovery_url = config
                    .discovery_url
                    .clone()
                    .unwrap_or_else(|| oidc_discovery_url(&config.issuer));
                let discovery: DiscoveryDocument = client
                    .get(&discovery_url)
                    .send()
                    .await
                    .with_context(|| {
                        format!("cannot fetch OIDC discovery document from {discovery_url}")
                    })?
                    .error_for_status()
                    .with_context(|| format!("OIDC discovery request failed for {discovery_url}"))?
                    .json()
                    .await
                    .with_context(|| {
                        format!("cannot parse OIDC discovery document from {discovery_url}")
                    })?;
                if let Some(discovery_issuer) = discovery.issuer.as_deref()
                    && discovery_issuer != config.issuer
                {
                    anyhow::bail!(
                        "OIDC discovery issuer mismatch: expected {}, got {}",
                        config.issuer,
                        discovery_issuer
                    );
                }
                discovery.jwks_uri.ok_or_else(|| {
                    anyhow::anyhow!(
                        "OIDC discovery document at {} is missing jwks_uri",
                        discovery_url
                    )
                })?
            }
        };

        let jwks: JwkSet = client
            .get(&jwks_url)
            .send()
            .await
            .with_context(|| format!("cannot fetch JWKS from {jwks_url}"))?
            .error_for_status()
            .with_context(|| format!("JWKS request failed for {jwks_url}"))?
            .json()
            .await
            .with_context(|| format!("cannot parse JWKS document from {jwks_url}"))?;
        if jwks.keys.is_empty() {
            anyhow::bail!("JWKS document at {jwks_url} does not contain any keys");
        }

        let mut scopes_supported = config
            .required_scopes
            .values()
            .flat_map(|scopes| scopes.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        scopes_supported.sort();

        Ok(Self {
            issuer: config.issuer,
            resource: config.resource,
            required_scopes: config.required_scopes,
            scopes_supported,
            allowed_origins: config.allowed_origins.into_iter().collect(),
            jwks,
        })
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn resource(&self) -> &str {
        &self.resource
    }

    pub fn allowed_origins(&self) -> &HashSet<String> {
        &self.allowed_origins
    }

    pub fn metadata_json(&self) -> Value {
        json!({
            "resource": self.resource,
            "authorization_servers": [self.issuer],
            "bearer_methods_supported": ["header"],
            "scopes_supported": self.scopes_supported,
        })
    }

    pub fn authorize(
        &self,
        headers: &HeaderMap,
        route_family: &str,
    ) -> std::result::Result<AuthorizedToken, AuthChallenge> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| self.challenge_missing_token(route_family))?;

        let claims = self
            .decode_token(token)
            .map_err(|_| self.challenge_invalid_token(route_family))?;
        let granted_scopes = extract_scopes(&claims.claims);
        let required_scopes = self.required_scopes(route_family);
        let missing_scopes = required_scopes
            .iter()
            .filter(|scope| !granted_scopes.contains(scope.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        if !missing_scopes.is_empty() {
            return Err(self.challenge_insufficient_scope(route_family));
        }

        Ok(AuthorizedToken {
            scopes: granted_scopes,
            subject: claims.claims.sub,
        })
    }

    fn decode_token(&self, token: &str) -> Result<TokenData<TokenClaims>> {
        let header = decode_header(token).context("cannot decode JWT header")?;
        let decoding_key = self.decoding_key_for_header(&header)?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[self.resource.as_str()]);
        validation.required_spec_claims.insert("exp".to_owned());
        validation.required_spec_claims.insert("iss".to_owned());
        validation.leeway = 30;
        let token_data = decode::<TokenClaims>(token, &decoding_key, &validation)
            .context("cannot validate JWT claims or signature")?;
        let _ = (
            &token_data.claims.iss,
            &token_data.claims.aud,
            token_data.claims.exp,
            token_data.claims.nbf,
            token_data.claims.iat,
        );
        Ok(token_data)
    }

    fn decoding_key_for_header(&self, header: &jsonwebtoken::Header) -> Result<DecodingKey> {
        let jwk = match header.kid.as_deref() {
            Some(kid) => self
                .jwks
                .keys
                .iter()
                .find(|key| key.common.key_id.as_deref() == Some(kid))
                .with_context(|| format!("no JWKS key found for kid={kid}"))?,
            None if self.jwks.keys.len() == 1 => &self.jwks.keys[0],
            None => anyhow::bail!("JWT header is missing kid and JWKS contains multiple keys"),
        };
        let key = DecodingKey::from_jwk(jwk).context("cannot build decoding key from JWKS")?;
        let supported_alg = jwk
            .common
            .key_algorithm
            .and_then(parse_algorithm_name)
            .unwrap_or(header.alg);
        if header.alg != supported_alg {
            anyhow::bail!(
                "JWT algorithm mismatch: token uses {:?}, JWKS key expects {:?}",
                header.alg,
                supported_alg
            );
        }
        Ok(key)
    }

    fn required_scopes(&self, route_family: &str) -> Vec<String> {
        self.required_scopes
            .get(route_family)
            .cloned()
            .unwrap_or_default()
    }

    fn challenge_missing_token(&self, route_family: &str) -> AuthChallenge {
        let required_scopes = self.required_scopes(route_family);
        AuthChallenge {
            status: StatusCode::UNAUTHORIZED,
            body_error: "unauthorized",
            body_message: "Bearer token required".to_owned(),
            error_code: "invalid_token",
            www_authenticate: self.www_authenticate(
                "invalid_token",
                "Bearer token required",
                &required_scopes,
            ),
            required_scopes,
        }
    }

    fn challenge_invalid_token(&self, route_family: &str) -> AuthChallenge {
        let required_scopes = self.required_scopes(route_family);
        AuthChallenge {
            status: StatusCode::UNAUTHORIZED,
            body_error: "unauthorized",
            body_message: "invalid bearer token".to_owned(),
            error_code: "invalid_token",
            www_authenticate: self.www_authenticate(
                "invalid_token",
                "invalid bearer token",
                &required_scopes,
            ),
            required_scopes,
        }
    }

    fn challenge_insufficient_scope(&self, route_family: &str) -> AuthChallenge {
        let required_scopes = self.required_scopes(route_family);
        AuthChallenge {
            status: StatusCode::FORBIDDEN,
            body_error: "forbidden",
            body_message: "insufficient scope for this route".to_owned(),
            error_code: "insufficient_scope",
            www_authenticate: self.www_authenticate(
                "insufficient_scope",
                "additional scope consent required",
                &required_scopes,
            ),
            required_scopes,
        }
    }

    fn www_authenticate(
        &self,
        error: &str,
        description: &str,
        required_scopes: &[String],
    ) -> String {
        let mut parts = vec![
            "Bearer realm=\"atlas-mcp\"".to_owned(),
            format!("resource=\"{}\"", escape_header_value(&self.resource)),
            format!("error=\"{}\"", escape_header_value(error)),
            format!("error_description=\"{}\"", escape_header_value(description)),
        ];
        if !required_scopes.is_empty() {
            parts.push(format!(
                "scope=\"{}\"",
                escape_header_value(&required_scopes.join(" "))
            ));
        }
        parts.join(", ")
    }
}

fn oidc_discovery_url(issuer: &str) -> String {
    format!("{}{}", issuer.trim_end_matches('/'), OIDC_DISCOVERY_PATH)
}

fn extract_scopes(claims: &TokenClaims) -> BTreeSet<String> {
    let mut scopes = BTreeSet::new();
    if let Some(scope) = claims.scope.as_deref() {
        for item in scope.split_whitespace() {
            if !item.is_empty() {
                scopes.insert(item.to_owned());
            }
        }
    }
    if let Some(scp) = claims.scp.as_ref() {
        match scp {
            Value::String(value) => {
                for item in value.split_whitespace() {
                    if !item.is_empty() {
                        scopes.insert(item.to_owned());
                    }
                }
            }
            Value::Array(values) => {
                for item in values {
                    if let Some(item) = item.as_str()
                        && !item.is_empty()
                    {
                        scopes.insert(item.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    scopes
}

fn parse_algorithm_name(value: jsonwebtoken::jwk::KeyAlgorithm) -> Option<Algorithm> {
    match value {
        jsonwebtoken::jwk::KeyAlgorithm::HS256 => Some(Algorithm::HS256),
        jsonwebtoken::jwk::KeyAlgorithm::HS384 => Some(Algorithm::HS384),
        jsonwebtoken::jwk::KeyAlgorithm::HS512 => Some(Algorithm::HS512),
        jsonwebtoken::jwk::KeyAlgorithm::RS256 => Some(Algorithm::RS256),
        jsonwebtoken::jwk::KeyAlgorithm::RS384 => Some(Algorithm::RS384),
        jsonwebtoken::jwk::KeyAlgorithm::RS512 => Some(Algorithm::RS512),
        jsonwebtoken::jwk::KeyAlgorithm::PS256 => Some(Algorithm::PS256),
        jsonwebtoken::jwk::KeyAlgorithm::PS384 => Some(Algorithm::PS384),
        jsonwebtoken::jwk::KeyAlgorithm::PS512 => Some(Algorithm::PS512),
        jsonwebtoken::jwk::KeyAlgorithm::ES256 => Some(Algorithm::ES256),
        jsonwebtoken::jwk::KeyAlgorithm::ES384 => Some(Algorithm::ES384),
        jsonwebtoken::jwk::KeyAlgorithm::EdDSA => Some(Algorithm::EdDSA),
        _ => None,
    }
}

fn escape_header_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::State;
    use axum::routing::get;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use std::net::SocketAddr;
    use std::sync::Arc;

    const TEST_SECRET: &[u8] = b"atlas-mcp-phase5-secret";
    const TEST_SECRET_B64U: &str = "YXRsYXMtbWNwLXBoYXNlNS1zZWNyZXQ";

    #[derive(Clone)]
    struct MockAuthState {
        discovery: Arc<String>,
        jwks: Arc<String>,
    }

    fn auth_config(base_url: &str) -> ProtectedResourceAuthConfig {
        ProtectedResourceAuthConfig {
            issuer: base_url.to_owned(),
            discovery_url: None,
            jwks_url: None,
            resource: "https://atlas.test/mcp".to_owned(),
            required_scopes: HashMap::from([(
                ROUTE_FAMILY_MCP.to_owned(),
                vec!["atlas:mcp".to_owned(), "atlas:read".to_owned()],
            )]),
            allowed_origins: vec!["https://app.atlas.test".to_owned()],
        }
    }

    fn jwks_document() -> String {
        json!({
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
        .to_string()
    }

    fn discovery_document(base_url: &str) -> String {
        json!({
            "issuer": base_url,
            "jwks_uri": format!("{base_url}/jwks")
        })
        .to_string()
    }

    async fn spawn_mock_auth_server_with_discovery(discovery: String, jwks: String) -> SocketAddr {
        let state = MockAuthState {
            discovery: Arc::new(discovery),
            jwks: Arc::new(jwks),
        };
        let app =
            Router::new()
                .route(
                    OIDC_DISCOVERY_PATH,
                    get(|State(state): State<MockAuthState>| async move {
                        state.discovery.as_str().to_owned()
                    }),
                )
                .route(
                    "/jwks",
                    get(|State(state): State<MockAuthState>| async move {
                        state.jwks.as_str().to_owned()
                    }),
                )
                .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock auth server");
        let addr = listener.local_addr().expect("mock auth addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve mock auth server");
        });
        addr
    }

    async fn spawn_mock_auth_server() -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock auth server");
        let addr = listener.local_addr().expect("mock auth addr");
        let base_url = format!("http://{}", addr);
        let state = MockAuthState {
            discovery: Arc::new(discovery_document(&base_url)),
            jwks: Arc::new(jwks_document()),
        };
        let app =
            Router::new()
                .route(
                    OIDC_DISCOVERY_PATH,
                    get(|State(state): State<MockAuthState>| async move {
                        state.discovery.as_str().to_owned()
                    }),
                )
                .route(
                    "/jwks",
                    get(|State(state): State<MockAuthState>| async move {
                        state.jwks.as_str().to_owned()
                    }),
                )
                .with_state(state);
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve mock auth server");
        });
        addr
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

    #[tokio::test]
    async fn metadata_body_shape_contains_required_fields() {
        let addr = spawn_mock_auth_server().await;
        let base_url = format!("http://{}", addr);
        let policy = ProtectedResourceAuthPolicy::load(auth_config(&base_url))
            .await
            .expect("load auth policy");
        let metadata = policy.metadata_json();
        assert_eq!(metadata["resource"], json!("https://atlas.test/mcp"));
        assert_eq!(metadata["authorization_servers"], json!([base_url]));
        assert_eq!(metadata["bearer_methods_supported"], json!(["header"]));
        assert_eq!(
            metadata["scopes_supported"],
            json!(["atlas:mcp", "atlas:read"])
        );
    }

    #[tokio::test]
    async fn load_resolves_oidc_discovery_from_issuer() {
        let addr = spawn_mock_auth_server().await;
        let base_url = format!("http://{}", addr);
        let policy = ProtectedResourceAuthPolicy::load(auth_config(&base_url))
            .await
            .expect("load auth policy");
        assert_eq!(policy.issuer(), base_url);
        assert_eq!(policy.resource(), "https://atlas.test/mcp");
        assert!(policy.allowed_origins().contains("https://app.atlas.test"));
    }

    #[tokio::test]
    async fn load_fails_on_invalid_discovery_response() {
        let addr = spawn_mock_auth_server_with_discovery(
            json!({"issuer":"http://bad.test"}).to_string(),
            jwks_document(),
        )
        .await;
        let base_url = format!("http://{}", addr);
        let error = ProtectedResourceAuthPolicy::load(auth_config(&base_url))
            .await
            .expect_err("discovery should fail");
        let message = error.to_string();
        assert!(message.contains("missing jwks_uri") || message.contains("issuer mismatch"));
    }

    #[tokio::test]
    async fn authorize_accepts_valid_token_and_rejects_scope_gaps() {
        let addr = spawn_mock_auth_server().await;
        let base_url = format!("http://{}", addr);
        let policy = ProtectedResourceAuthPolicy::load(auth_config(&base_url))
            .await
            .expect("load auth policy");

        let token = make_token(&base_url, &["atlas:mcp", "atlas:read"]);
        let authorized = policy
            .authorize(&bearer_headers(&token), ROUTE_FAMILY_MCP)
            .expect("token should authorize");
        assert_eq!(authorized.subject.as_deref(), Some("user-123"));
        assert!(authorized.scopes.contains("atlas:mcp"));

        let weak_token = make_token(&base_url, &["atlas:mcp"]);
        let challenge = policy
            .authorize(&bearer_headers(&weak_token), ROUTE_FAMILY_MCP)
            .expect_err("scope gap should challenge");
        assert_eq!(challenge.status, StatusCode::FORBIDDEN);
        assert!(
            challenge
                .www_authenticate
                .contains("scope=\"atlas:mcp atlas:read\"")
        );
        assert!(
            challenge
                .www_authenticate
                .contains("resource=\"https://atlas.test/mcp\"")
        );
    }

    #[tokio::test]
    async fn authorize_rejects_missing_or_invalid_token() {
        let addr = spawn_mock_auth_server().await;
        let base_url = format!("http://{}", addr);
        let policy = ProtectedResourceAuthPolicy::load(auth_config(&base_url))
            .await
            .expect("load auth policy");

        let missing = policy
            .authorize(&HeaderMap::new(), ROUTE_FAMILY_MCP)
            .expect_err("missing header must fail");
        assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
        assert_eq!(missing.body_message, "Bearer token required");

        let invalid = policy
            .authorize(&bearer_headers("not-a-jwt"), ROUTE_FAMILY_MCP)
            .expect_err("invalid token must fail");
        assert_eq!(invalid.status, StatusCode::UNAUTHORIZED);
        assert_eq!(invalid.body_message, "invalid bearer token");
    }
}
