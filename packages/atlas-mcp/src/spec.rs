use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value, json};

pub const MCP_PROTOCOL_VERSION: &str = "2026-07-28";
pub const MCP_PREVIOUS_PROTOCOL_VERSION: &str = "2025-11-25";
pub const MCP_SERVER_NAME: &str = "atlas";

pub const MCP_SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &[MCP_PROTOCOL_VERSION, MCP_PREVIOUS_PROTOCOL_VERSION];

pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
pub const META_LOG_LEVEL: &str = "io.modelcontextprotocol/logLevel";
pub const CACHE_SCOPE_PUBLIC: &str = "public";
pub const CACHE_SCOPE_PRIVATE: &str = "private";
pub const DISCOVER_CACHE_TTL_MS: u64 = 300_000;
pub const DISCOVER_CACHE_SCOPE: &str = CACHE_SCOPE_PUBLIC;
pub const DISCOVER_INSTRUCTIONS: &str = "Use server/discover for capability negotiation. After discovery, include params._meta with protocol version and client capabilities.";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<ExtensionsCapabilities>,
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
pub struct ExtensionsCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<TasksExtensionCapabilities>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TasksExtensionCapabilities {
    pub get: EmptyCapability,
    pub update: EmptyCapability,
    pub tool_call_handles: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
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

pub fn server_info_meta_value() -> Value {
    serde_json::to_value(server_info()).expect("server info serialization")
}

pub fn supported_protocol_versions_value() -> Value {
    json!(MCP_SUPPORTED_PROTOCOL_VERSIONS)
}

pub fn supported_protocol_versions_display() -> String {
    MCP_SUPPORTED_PROTOCOL_VERSIONS.join(", ")
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
        extensions: Some(ExtensionsCapabilities {
            tasks: Some(TasksExtensionCapabilities {
                get: EmptyCapability::default(),
                update: EmptyCapability::default(),
                tool_call_handles: true,
            }),
        }),
        experimental: Some(ExperimentalCapabilities {
            progress_notifications: true,
        }),
    }
}

pub fn discover_result() -> Value {
    let mut result = json!({
        "supportedVersions": supported_protocol_versions_value(),
        "capabilities": initialize_capabilities(),
        "serverInfo": server_info(),
        "instructions": DISCOVER_INSTRUCTIONS,
    });
    annotate_cacheable_result(&mut result, DISCOVER_CACHE_TTL_MS, DISCOVER_CACHE_SCOPE);
    result
}

pub fn annotate_cacheable_result(result: &mut Value, ttl_ms: u64, cache_scope: &str) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    object
        .entry("resultType".to_owned())
        .or_insert_with(|| Value::String("complete".to_owned()));
    object.insert("ttlMs".to_owned(), json!(ttl_ms));
    object.insert("cacheScope".to_owned(), json!(cache_scope));
}

pub fn complete_result(mut result: Value) -> Value {
    let meta = shared_result_meta();
    match result {
        Value::Object(ref mut object) => {
            object
                .entry("resultType".to_owned())
                .or_insert_with(|| Value::String("complete".to_owned()));
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
    if MCP_SUPPORTED_PROTOCOL_VERSIONS.contains(&protocol_version) {
        Ok(())
    } else {
        Err(anyhow!(
            "unsupported protocol version '{protocol_version}'; supported versions: {}",
            supported_protocol_versions_display()
        ))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ensure_supported_protocol_version_rejects_unsupported_version() {
        let error = ensure_supported_protocol_version("2024-11-05").unwrap_err();
        assert_eq!(
            error.to_string(),
            "unsupported protocol version '2024-11-05'; supported versions: 2026-07-28, 2025-11-25"
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

    #[test]
    fn discover_result_includes_required_fields_and_initialize_capabilities() {
        let result = discover_result();

        assert_eq!(
            result["supportedVersions"],
            supported_protocol_versions_value()
        );
        assert_eq!(
            result["capabilities"],
            serde_json::to_value(initialize_capabilities()).expect("capabilities")
        );
        assert_eq!(result["serverInfo"], server_info_meta_value());
        assert_eq!(result["instructions"], json!(DISCOVER_INSTRUCTIONS));
        assert_eq!(result["resultType"], json!("complete"));
        assert_eq!(result["ttlMs"], json!(DISCOVER_CACHE_TTL_MS));
        assert_eq!(result["cacheScope"], json!(DISCOVER_CACHE_SCOPE));
    }
}
