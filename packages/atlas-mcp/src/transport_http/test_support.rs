//! Response capture for the in-process streamable HTTP test harness.
//!
//! The body must be drained inside the same Tokio runtime that served the
//! request. Dropping that runtime first tears down the SSE producer tasks and
//! truncates any stream that outlives the response headers.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use axum::response::Response;
use serde_json::Value;

use super::TestHttpResponse;

pub(super) async fn response_to_test_output(response: Response) -> Result<TestHttpResponse> {
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .context("cannot read HTTP test response body")?;
    let body_text =
        String::from_utf8(bytes.to_vec()).context("HTTP test response body must be utf-8")?;
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
