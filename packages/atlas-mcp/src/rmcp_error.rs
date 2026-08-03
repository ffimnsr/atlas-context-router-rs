#![allow(dead_code)]

use std::borrow::Cow;

use anyhow::Error;
use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;
use serde_json::{Value, json};

pub(crate) type RmcpResult<T> = Result<T, Error>;

pub(crate) fn internal_error(error: Error) -> McpError {
    internal_error_message(error.to_string())
}

pub(crate) fn internal_error_message(message: impl Into<Cow<'static, str>>) -> McpError {
    McpError::internal_error(
        message,
        Some(json!({
            "atlas_error_code": "internal_error",
        })),
    )
}

pub(crate) fn invalid_params(
    message: impl Into<Cow<'static, str>>,
    data: Option<Value>,
) -> McpError {
    McpError::invalid_params(
        message,
        Some(merge_atlas_error_code(data, "invalid_params")),
    )
}

pub(crate) fn method_not_found(message: impl Into<Cow<'static, str>>) -> McpError {
    McpError::new(
        ErrorCode::METHOD_NOT_FOUND,
        message,
        Some(json!({
            "atlas_error_code": "method_not_found",
        })),
    )
}

fn merge_atlas_error_code(data: Option<Value>, atlas_error_code: &str) -> Value {
    match data {
        Some(Value::Object(mut object)) => {
            object.insert(
                "atlas_error_code".to_owned(),
                Value::String(atlas_error_code.to_owned()),
            );
            Value::Object(object)
        }
        Some(other) => json!({
            "atlas_error_code": atlas_error_code,
            "details": other,
        }),
        None => json!({
            "atlas_error_code": atlas_error_code,
        }),
    }
}
