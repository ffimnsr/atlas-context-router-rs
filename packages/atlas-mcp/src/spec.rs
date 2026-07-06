use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value};

pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
pub const MCP_SERVER_NAME: &str = "atlas";

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
pub struct ExperimentalCapabilities {
    pub progress_notifications: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

pub fn initialize_capabilities() -> InitializeCapabilities {
    InitializeCapabilities {
        tools: EmptyCapability::default(),
        prompts: PromptCapabilities {
            list_changed: false,
        },
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

pub fn ensure_supported_protocol_version(protocol_version: &str) -> Result<()> {
    if protocol_version == MCP_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(anyhow!(
            "unsupported protocol version '{protocol_version}'; supported version: {MCP_PROTOCOL_VERSION}"
        ))
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
            "unsupported protocol version '2024-11-05'; supported version: 2025-11-25"
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
}
