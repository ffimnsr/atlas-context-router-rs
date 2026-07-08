//! MCP method dispatch routing.

use serde::{Deserialize, Serialize};

use super::jsonrpc::{
    DispatchError, JsonRpcErrorKind, classify_prompt_error, classify_resource_error,
    classify_task_api_error, classify_tool_call_dispatch_error,
};
use crate::{completion, prompts, resources, spec, tools};

/// Dispatch an MCP method to the appropriate handler.
pub(crate) fn dispatch(
    method: &str,
    params: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> std::result::Result<serde_json::Value, DispatchError> {
    match method {
        "initialize" => spec::negotiate_initialize(params)
            .map_err(|error| DispatchError::new(JsonRpcErrorKind::InvalidParams, error)),

        "tools/list" => Ok(tools::tool_list()),
        "resources/list" => resources::resources_list(params)
            .map_err(|error| DispatchError::new(JsonRpcErrorKind::InvalidParams, error)),
        "resources/templates/list" => resources::resources_templates_list(params)
            .map_err(|error| DispatchError::new(JsonRpcErrorKind::InvalidParams, error)),
        "resources/read" => {
            resources::resources_read(params, repo_root, db_path).map_err(classify_resource_error)
        }
        "completion/complete" => completion::complete(params, repo_root, db_path)
            .map_err(|error| DispatchError::new(JsonRpcErrorKind::InvalidParams, error)),

        "prompts/list" => Ok(prompts::prompt_list()),

        "prompts/get" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| {
                    DispatchError::new(
                        JsonRpcErrorKind::InvalidParams,
                        anyhow::anyhow!("missing prompt name"),
                    )
                })?;
            let args = params.and_then(|p| p.get("arguments"));
            prompts::prompt_get(name, args).map_err(classify_prompt_error)
        }

        "tools/call" => {
            let params = params.ok_or_else(|| {
                DispatchError::new(
                    JsonRpcErrorKind::InvalidParams,
                    anyhow::anyhow!("missing tools/call params object"),
                )
            })?;
            let params_object = params.as_object().ok_or_else(|| {
                DispatchError::new(
                    JsonRpcErrorKind::InvalidParams,
                    anyhow::anyhow!("tools/call params must be an object"),
                )
            })?;
            let name = params_object
                .get("name")
                .and_then(|n| n.as_str())
                .ok_or_else(|| {
                    DispatchError::new(
                        JsonRpcErrorKind::InvalidParams,
                        anyhow::anyhow!("missing tool name"),
                    )
                })?;
            if !crate::tools::is_known_tool_name(name) {
                return Err(DispatchError::new(
                    JsonRpcErrorKind::MethodNotFound,
                    anyhow::anyhow!("unknown tool: {name}"),
                ));
            }
            let args = match params_object.get("arguments") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) if value.is_object() => Some(value.clone()),
                Some(_) => {
                    return Err(DispatchError::new(
                        JsonRpcErrorKind::InvalidParams,
                        anyhow::anyhow!("tools/call arguments must be an object when provided"),
                    ));
                }
            };
            crate::tasks::execute_tool_call(name, args, repo_root, db_path)
                .map_err(classify_tool_call_dispatch_error)
        }

        "tasks/list" => {
            crate::tasks::tasks_list(params, repo_root, crate::output::OutputFormat::Json)
                .map_err(classify_task_api_error)
        }
        "tasks/get" => {
            crate::tasks::tasks_get(params, repo_root, crate::output::OutputFormat::Json)
                .map_err(classify_task_api_error)
        }
        "tasks/result" => {
            crate::tasks::tasks_result(params, repo_root, crate::output::OutputFormat::Json)
                .map_err(classify_task_api_error)
        }
        "tasks/cancel" => {
            crate::tasks::tasks_cancel(params, repo_root, crate::output::OutputFormat::Json)
                .map_err(classify_task_api_error)
        }

        other => Err(DispatchError::new(
            JsonRpcErrorKind::MethodNotFound,
            anyhow::anyhow!("method not found: {other}"),
        )),
    }
}

// ---------------------------------------------------------------------------
// Daemon handshake types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct DaemonHandshakeRequest {
    pub(crate) protocol_version: String,
    pub(crate) repo_root: String,
    pub(crate) db_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct DaemonHandshakeResponse {
    pub(crate) ok: bool,
    pub(crate) protocol_version: String,
    pub(crate) repo_root: String,
    pub(crate) db_path: String,
    pub(crate) error: Option<String>,
}

impl DaemonHandshakeResponse {
    pub(crate) fn ok(protocol_version: &str, repo_root: &str, db_path: &str) -> Self {
        Self {
            ok: true,
            protocol_version: protocol_version.to_owned(),
            repo_root: repo_root.to_owned(),
            db_path: db_path.to_owned(),
            error: None,
        }
    }

    pub(crate) fn err(
        protocol_version: &str,
        repo_root: &str,
        db_path: &str,
        error: String,
    ) -> Self {
        Self {
            ok: false,
            protocol_version: protocol_version.to_owned(),
            repo_root: repo_root.to_owned(),
            db_path: db_path.to_owned(),
            error: Some(error),
        }
    }
}
