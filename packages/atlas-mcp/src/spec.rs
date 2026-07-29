use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value, json};

pub const MCP_PROTOCOL_VERSION: &str = "2026-07-28";
pub const MCP_SERVER_NAME: &str = "atlas";

pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
pub const META_LOG_LEVEL: &str = "io.modelcontextprotocol/logLevel";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InitializeCapabilities {
    pub tools: EmptyCapability,
    pub prompts: PromptCapabilities,
    pub resources: ResourceCapabilities,
    pub completions: EmptyCapability,
    pub logging: EmptyCapability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<TasksCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<ExperimentalCapabilities>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct EmptyCapability {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    pub list_changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCapabilities {
    pub subscribe: bool,
    pub list_changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalCapabilities {
    pub progress_notifications: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TasksCapabilities {
    pub list: EmptyCapability,
    pub cancel: EmptyCapability,
    pub requests: TaskRequestCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TaskRequestCapabilities {
    pub tools: TaskToolCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TaskToolCapabilities {
    pub call: EmptyCapability,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InitializeRequest {
    pub protocol_version: String,
    pub capabilities: Value,
    pub client_info: ClientInfo,
    pub meta: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequestMeta {
    pub protocol_version: String,
    pub client_capabilities: Value,
    pub client_info: Option<ClientInfo>,
    pub log_level: Option<String>,
    pub meta: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestMetaErrorKind {
    MissingMeta,
    InvalidMeta,
    UnsupportedProtocolVersion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestMetaError {
    kind: RequestMetaErrorKind,
    message: String,
}

impl RequestMetaError {
    pub fn kind(&self) -> &RequestMetaErrorKind {
        &self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for RequestMetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RequestMetaError {}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: InitializeCapabilities,
    pub server_info: ServerInfo,
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

pub fn server_info() -> ServerInfo {
    ServerInfo {
        name: MCP_SERVER_NAME.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        description: env!("CARGO_PKG_DESCRIPTION").to_owned(),
    }
}

pub fn server_info_meta_value() -> Value {
    serde_json::to_value(server_info()).expect("server info serialization")
}

pub fn supported_protocol_versions_value() -> Value {
    json!([MCP_PROTOCOL_VERSION])
}

pub fn initialize_capabilities() -> InitializeCapabilities {
    InitializeCapabilities {
        tools: EmptyCapability::default(),
        prompts: PromptCapabilities {
            list_changed: false,
        },
        resources: ResourceCapabilities {
            subscribe: false,
            list_changed: false,
        },
        completions: EmptyCapability::default(),
        logging: EmptyCapability::default(),
        tasks: Some(TasksCapabilities {
            list: EmptyCapability::default(),
            cancel: EmptyCapability::default(),
            requests: TaskRequestCapabilities {
                tools: TaskToolCapabilities {
                    call: EmptyCapability::default(),
                },
            },
        }),
        experimental: Some(ExperimentalCapabilities {
            progress_notifications: true,
        }),
    }
}

pub fn parse_initialize_request(params: Option<&Value>) -> Result<InitializeRequest> {
    let params = params
        .ok_or_else(|| anyhow!("initialize requires params object"))?
        .as_object()
        .ok_or_else(|| anyhow!("initialize requires params object"))?;

    let protocol_version = required_string_field(
        params,
        "protocolVersion",
        "initialize requires string params.protocolVersion",
    )?;
    let capabilities = required_object_value(
        params,
        "capabilities",
        "initialize requires object params.capabilities",
    )?;
    let client_info = required_object_field(
        params,
        "clientInfo",
        "initialize requires object params.clientInfo",
    )?;
    let client_name = required_string_field(
        client_info,
        "name",
        "initialize requires string params.clientInfo.name",
    )?;
    let client_version = required_string_field(
        client_info,
        "version",
        "initialize requires string params.clientInfo.version",
    )?;

    Ok(InitializeRequest {
        protocol_version,
        capabilities,
        client_info: ClientInfo {
            name: client_name,
            version: client_version,
        },
        meta: params.get("_meta").cloned(),
    })
}

pub fn parse_request_meta(
    method: &str,
    params: Option<&Value>,
) -> std::result::Result<RequestMeta, RequestMetaError> {
    let params = params
        .and_then(Value::as_object)
        .ok_or_else(|| missing_meta_error(method))?;
    let meta = params
        .get("_meta")
        .and_then(Value::as_object)
        .ok_or_else(|| missing_meta_error(method))?;

    let protocol_version = meta
        .get(META_PROTOCOL_VERSION)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_meta_error(method, META_PROTOCOL_VERSION, "string"))?
        .to_owned();
    ensure_supported_request_protocol_version(method, &protocol_version)?;

    let client_capabilities = meta
        .get(META_CLIENT_CAPABILITIES)
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| invalid_meta_error(method, META_CLIENT_CAPABILITIES, "object"))?;

    let client_info = match meta.get(META_CLIENT_INFO) {
        None | Some(Value::Null) => None,
        Some(value) => Some(parse_client_info_value(value, method, META_CLIENT_INFO)?),
    };
    let log_level = match meta.get(META_LOG_LEVEL) {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| invalid_meta_error(method, META_LOG_LEVEL, "string"))?
                .to_owned(),
        ),
    };

    Ok(RequestMeta {
        protocol_version,
        client_capabilities,
        client_info,
        log_level,
        meta: Value::Object(meta.clone()),
    })
}

pub fn negotiate_initialize(params: Option<&Value>) -> Result<Value> {
    let request = parse_initialize_request(params)?;
    ensure_supported_protocol_version(&request.protocol_version)?;
    serde_json::to_value(initialize_result(&request)).map_err(Into::into)
}

pub fn initialize_result(request: &InitializeRequest) -> InitializeResult {
    InitializeResult {
        protocol_version: MCP_PROTOCOL_VERSION.to_owned(),
        capabilities: initialize_capabilities(),
        server_info: server_info(),
        meta: request.meta.clone(),
    }
}

pub fn complete_result(mut result: Value) -> Value {
    let meta = shared_result_meta();
    match result {
        Value::Object(ref mut object) => {
            object.insert(
                "resultType".to_owned(),
                Value::String("complete".to_owned()),
            );
            merge_result_meta(object, meta);
            result
        }
        other => json!({
            "resultType": "complete",
            "value": other,
            "_meta": meta,
        }),
    }
}

pub fn ensure_supported_protocol_version(protocol_version: &str) -> Result<()> {
    if protocol_version == MCP_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(anyhow!(
            "unsupported protocol version '{protocol_version}'; supported version: {MCP_PROTOCOL_VERSION}"
        ))
    }
}

fn ensure_supported_request_protocol_version(
    method: &str,
    protocol_version: &str,
) -> std::result::Result<(), RequestMetaError> {
    if protocol_version == MCP_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(RequestMetaError {
            kind: RequestMetaErrorKind::UnsupportedProtocolVersion,
            message: format!(
                "{method} requested unsupported protocol version '{protocol_version}'; supported version: {MCP_PROTOCOL_VERSION}"
            ),
        })
    }
}

fn missing_meta_error(method: &str) -> RequestMetaError {
    RequestMetaError {
        kind: RequestMetaErrorKind::MissingMeta,
        message: format!(
            "{method} requires params._meta with {META_PROTOCOL_VERSION} and {META_CLIENT_CAPABILITIES}"
        ),
    }
}

fn invalid_meta_error(method: &str, key: &str, expected: &str) -> RequestMetaError {
    RequestMetaError {
        kind: RequestMetaErrorKind::InvalidMeta,
        message: format!("{method} requires params._meta.{key} as {expected}"),
    }
}

fn parse_client_info_value(
    value: &Value,
    method: &str,
    field_name: &str,
) -> std::result::Result<ClientInfo, RequestMetaError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid_meta_error(method, field_name, "object"))?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_meta_error(method, field_name, "object with string name/version"))?;
    let version = object
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_meta_error(method, field_name, "object with string name/version"))?;
    Ok(ClientInfo {
        name: name.to_owned(),
        version: version.to_owned(),
    })
}

fn shared_result_meta() -> Map<String, Value> {
    Map::from_iter([(META_SERVER_INFO.to_owned(), server_info_meta_value())])
}

fn merge_result_meta(result: &mut Map<String, Value>, shared_meta: Map<String, Value>) {
    match result.get_mut("_meta") {
        Some(Value::Object(existing)) => {
            for (key, value) in shared_meta {
                existing.insert(key, value);
            }
        }
        _ => {
            result.insert("_meta".to_owned(), Value::Object(shared_meta));
        }
    }
}

fn required_string_field(
    object: &Map<String, Value>,
    key: &str,
    message: &'static str,
) -> Result<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!(message))
}

fn required_object_field<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    message: &'static str,
) -> Result<&'a Map<String, Value>> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!(message))
}

fn required_object_value(
    object: &Map<String, Value>,
    key: &str,
    message: &'static str,
) -> Result<Value> {
    object
        .get(key)
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| anyhow!(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn initialize_requires_required_fields() {
        let error = parse_initialize_request(Some(&json!({}))).unwrap_err();
        assert_eq!(
            error.to_string(),
            "initialize requires string params.protocolVersion"
        );
    }

    #[test]
    fn initialize_rejects_unsupported_protocol_version() {
        let error = negotiate_initialize(Some(&json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "zed", "version": "1.0.0" }
        })))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "unsupported protocol version '2024-11-05'; supported version: 2026-07-28"
        );
    }

    #[test]
    fn initialize_result_includes_meta_and_description() {
        let result = negotiate_initialize(Some(&json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "zed", "version": "1.0.0" },
            "_meta": { "clientTag": "abc" }
        })))
        .unwrap();

        assert_eq!(result["protocolVersion"], json!(MCP_PROTOCOL_VERSION));
        assert_eq!(result["serverInfo"]["name"], json!(MCP_SERVER_NAME));
        assert_eq!(
            result["serverInfo"]["version"],
            json!(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            result["serverInfo"]["description"],
            json!(env!("CARGO_PKG_DESCRIPTION"))
        );
        assert_eq!(result["_meta"]["clientTag"], json!("abc"));
    }

    #[test]
    fn request_meta_requires_meta_object() {
        let error = parse_request_meta("tools/list", Some(&json!({}))).unwrap_err();
        assert_eq!(error.kind(), &RequestMetaErrorKind::MissingMeta);
        assert_eq!(
            error.to_string(),
            format!(
                "tools/list requires params._meta with {META_PROTOCOL_VERSION} and {META_CLIENT_CAPABILITIES}"
            )
        );
    }

    #[test]
    fn request_meta_rejects_unsupported_protocol_version() {
        let error = parse_request_meta(
            "tools/list",
            Some(&json!({
                "_meta": {
                    META_PROTOCOL_VERSION: "2025-11-25",
                    META_CLIENT_CAPABILITIES: {},
                }
            })),
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            &RequestMetaErrorKind::UnsupportedProtocolVersion
        );
    }

    #[test]
    fn complete_result_includes_result_type_and_server_info_meta() {
        let result = complete_result(json!({ "tools": [] }));
        assert_eq!(result["resultType"], json!("complete"));
        assert_eq!(
            result["_meta"][META_SERVER_INFO]["name"],
            json!(MCP_SERVER_NAME)
        );
    }
}
