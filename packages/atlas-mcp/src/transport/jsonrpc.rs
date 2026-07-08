//! JSON-RPC 2.0 protocol error types and response builders.

use atlas_core::{error_code_docs_ref, user_facing_error_message};

use crate::tasks;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const JSONRPC_PARSE_ERROR: i32 = -32700;
const JSONRPC_INVALID_REQUEST: i32 = -32600;
const JSONRPC_METHOD_NOT_FOUND: i32 = -32601;
const JSONRPC_INVALID_PARAMS: i32 = -32602;
const JSONRPC_INTERNAL_ERROR: i32 = -32603;
pub(crate) const JSONRPC_WORKER_UNAVAILABLE: i32 = -32002;
pub(crate) const JSONRPC_RATE_LIMITED: i32 = -32004;
const JSONRPC_TASK_NOT_FOUND: i32 = -32010;
const JSONRPC_TASK_NOT_READY: i32 = -32011;
const JSONRPC_TASK_CANCELLED: i32 = -32012;
const JSONRPC_TASK_FAILED: i32 = -32013;

// ---------------------------------------------------------------------------
// JsonRpcErrorKind
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsonRpcErrorKind {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,

    WorkerUnavailable,
    RateLimited,
    TaskNotFound,
    TaskNotReady,
    TaskCancelled,
    TaskFailed,
}

impl JsonRpcErrorKind {
    pub(crate) fn code(self) -> i32 {
        match self {
            Self::ParseError => JSONRPC_PARSE_ERROR,
            Self::InvalidRequest => JSONRPC_INVALID_REQUEST,
            Self::MethodNotFound => JSONRPC_METHOD_NOT_FOUND,
            Self::InvalidParams => JSONRPC_INVALID_PARAMS,
            Self::InternalError => JSONRPC_INTERNAL_ERROR,
            Self::WorkerUnavailable => JSONRPC_WORKER_UNAVAILABLE,
            Self::RateLimited => JSONRPC_RATE_LIMITED,
            Self::TaskNotFound => JSONRPC_TASK_NOT_FOUND,
            Self::TaskNotReady => JSONRPC_TASK_NOT_READY,
            Self::TaskCancelled => JSONRPC_TASK_CANCELLED,
            Self::TaskFailed => JSONRPC_TASK_FAILED,
        }
    }

    pub(crate) fn atlas_error_code(self) -> &'static str {
        match self {
            Self::ParseError => "parse_error",
            Self::InvalidRequest => "invalid_request",
            Self::MethodNotFound => "method_not_found",
            Self::InvalidParams => "invalid_params",
            Self::InternalError => "internal_error",
            Self::WorkerUnavailable => "worker_unavailable",
            Self::RateLimited => "rate_limited",
            Self::TaskNotFound => "task_not_found",
            Self::TaskNotReady => "task_not_ready",
            Self::TaskCancelled => "task_cancelled",
            Self::TaskFailed => "task_failed",
        }
    }
}

// ---------------------------------------------------------------------------
// DispatchError
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct DispatchError {
    pub(crate) kind: JsonRpcErrorKind,
    pub(crate) source: anyhow::Error,
}

impl DispatchError {
    pub(crate) fn new(kind: JsonRpcErrorKind, source: anyhow::Error) -> Self {
        Self { kind, source }
    }

    pub(crate) fn message(&self) -> String {
        match self.kind {
            JsonRpcErrorKind::InternalError => user_visible_error_message(&self.source),
            _ => self.source.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Error classifiers
// ---------------------------------------------------------------------------

pub(crate) fn classify_prompt_error(error: anyhow::Error) -> DispatchError {
    let detail = error.to_string();
    let kind = if detail.starts_with("unknown prompt:") || is_invalid_params_message(&detail) {
        JsonRpcErrorKind::InvalidParams
    } else {
        JsonRpcErrorKind::InternalError
    };
    DispatchError::new(kind, error)
}

pub(crate) fn classify_tool_call_dispatch_error(error: anyhow::Error) -> DispatchError {
    DispatchError::new(JsonRpcErrorKind::InternalError, error)
}

pub(crate) fn classify_resource_error(error: anyhow::Error) -> DispatchError {
    DispatchError::new(JsonRpcErrorKind::InvalidParams, error)
}

pub(crate) fn classify_task_api_error(error: tasks::TaskApiError) -> DispatchError {
    let kind = match error.kind() {
        tasks::TaskApiErrorKind::InvalidParams => JsonRpcErrorKind::InvalidParams,
        tasks::TaskApiErrorKind::NotFound => JsonRpcErrorKind::TaskNotFound,
        tasks::TaskApiErrorKind::NotReady => JsonRpcErrorKind::TaskNotReady,
        tasks::TaskApiErrorKind::Cancelled => JsonRpcErrorKind::TaskCancelled,
        tasks::TaskApiErrorKind::Failed => JsonRpcErrorKind::TaskFailed,
        tasks::TaskApiErrorKind::Internal => JsonRpcErrorKind::InternalError,
    };
    DispatchError::new(kind, error.into_anyhow())
}

pub(crate) fn is_invalid_params_message(detail: &str) -> bool {
    detail.starts_with("missing ")
        || detail.starts_with("argument '")
        || detail.contains("missing required argument:")
        || detail.contains("requires non-empty")
        || detail.contains("must be ")
        || detail.contains("invalid regex pattern")
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 response builders
// ---------------------------------------------------------------------------

pub(crate) fn jsonrpc_ok(id: serde_json::Value, result: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

pub(crate) fn jsonrpc_notification(method: &str, params: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

fn user_visible_error_message(error: &anyhow::Error) -> String {
    user_facing_error_message(&error.to_string(), &format!("{error:#}"))
}

pub(crate) fn jsonrpc_error(
    id: serde_json::Value,
    kind: JsonRpcErrorKind,
    message: String,
) -> String {
    let atlas_error_code = kind.atlas_error_code();
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": kind.code(),
            "message": message,
            "data": {
                "atlas_error_code": atlas_error_code,
                "atlas_error_code_docs": error_code_docs_ref(atlas_error_code)
            }
        }
    })
    .to_string()
}

pub(crate) fn jsonrpc_dispatch_error(id: serde_json::Value, error: &DispatchError) -> String {
    jsonrpc_error(id, error.kind, error.message())
}
