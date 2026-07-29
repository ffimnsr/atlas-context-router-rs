use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::output::OutputFormat;
use crate::tool_result::ToolResultBuilder;

const DEFAULT_REQUEST_STATE_TTL: Duration = Duration::from_secs(10 * 60);
const REQUEST_STATE_VERSION: u32 = 1;

static REQUEST_STATE_SECRET: OnceLock<String> = OnceLock::new();
static REQUEST_STATE_NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InputRequiredResult {
    pub(crate) result_type: String,
    pub(crate) input_requests: Vec<InputRequest>,
    pub(crate) request_state: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InputRequest {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) request_type: String,
    pub(crate) message: String,
    pub(crate) requested_schema: Value,
}

pub(crate) type InputResponses = Map<String, Value>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestStateBinding<'a> {
    pub(crate) method: &'a str,
    pub(crate) tool: &'a str,
    pub(crate) arguments: Option<&'a Value>,
    pub(crate) principal: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignedRequestState {
    payload: RequestStatePayload,
    signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestStatePayload {
    version: u32,
    method: String,
    tool: String,
    args_digest: String,
    principal: Option<String>,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    nonce: String,
}

pub(crate) fn build_input_required_tool_result(
    value: &InputRequiredResult,
    output_format: OutputFormat,
) -> Result<Value> {
    let raw = serde_json::to_value(value)?;
    let mut response = ToolResultBuilder::new(output_format).build_value(raw.clone())?;
    let response_object = response
        .as_object_mut()
        .ok_or_else(|| anyhow!("tool input_required result must be an object"))?;
    let raw_object = raw
        .as_object()
        .ok_or_else(|| anyhow!("input_required payload must serialize to object"))?;
    for (key, value) in raw_object {
        response_object.insert(key.clone(), value.clone());
    }
    Ok(response)
}

pub(crate) fn issue_request_state(binding: RequestStateBinding<'_>) -> Result<String> {
    issue_request_state_with_clock(binding, SystemTime::now(), DEFAULT_REQUEST_STATE_TTL)
}

pub(crate) fn validate_request_state(
    request_state: &str,
    binding: RequestStateBinding<'_>,
) -> Result<()> {
    validate_request_state_with_clock(request_state, binding, SystemTime::now())
}

fn issue_request_state_with_clock(
    binding: RequestStateBinding<'_>,
    now: SystemTime,
    ttl: Duration,
) -> Result<String> {
    let issued_at_unix_ms = unix_ms(now)?;
    let expires_at_unix_ms = issued_at_unix_ms.saturating_add(ttl.as_millis() as u64);
    let payload = RequestStatePayload {
        version: REQUEST_STATE_VERSION,
        method: binding.method.to_owned(),
        tool: binding.tool.to_owned(),
        args_digest: args_digest(binding.arguments)?,
        principal: binding.principal.map(str::to_owned),
        issued_at_unix_ms,
        expires_at_unix_ms,
        nonce: format!(
            "{}-{}",
            std::process::id(),
            REQUEST_STATE_NONCE_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
        ),
    };
    let signature = sign_payload(&payload)?;
    Ok(serde_json::to_string(&SignedRequestState {
        payload,
        signature,
    })?)
}

fn validate_request_state_with_clock(
    request_state: &str,
    binding: RequestStateBinding<'_>,
    now: SystemTime,
) -> Result<()> {
    let state: SignedRequestState = serde_json::from_str(request_state)
        .map_err(|error| anyhow!("requestState must be valid signed JSON: {error}"))?;
    if state.payload.version != REQUEST_STATE_VERSION {
        bail!(
            "requestState version mismatch: expected {}, got {}",
            REQUEST_STATE_VERSION,
            state.payload.version
        );
    }
    let expected_signature = sign_payload(&state.payload)?;
    if state.signature != expected_signature {
        bail!("requestState signature mismatch");
    }
    if state.payload.method != binding.method {
        bail!("requestState method mismatch");
    }
    if state.payload.tool != binding.tool {
        bail!("requestState tool mismatch");
    }
    if state.payload.args_digest != args_digest(binding.arguments)? {
        bail!("requestState arguments digest mismatch");
    }
    if state.payload.principal.as_deref() != binding.principal {
        bail!("requestState principal mismatch");
    }
    let now_unix_ms = unix_ms(now)?;
    if now_unix_ms > state.payload.expires_at_unix_ms {
        bail!("requestState expired");
    }
    Ok(())
}

fn sign_payload(payload: &RequestStatePayload) -> Result<String> {
    let payload_bytes = serde_json::to_vec(payload)?;
    Ok(sha256_hex_parts(&[
        request_state_secret().as_bytes(),
        b":",
        &payload_bytes,
    ]))
}

fn request_state_secret() -> &'static str {
    REQUEST_STATE_SECRET.get_or_init(|| {
        std::env::var("ATLAS_MCP_REQUEST_STATE_SECRET").unwrap_or_else(|_| {
            let seed_now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default();
            sha256_hex_parts(&[format!("atlas-mcp:{}:{seed_now}", std::process::id()).as_bytes()])
        })
    })
}

fn args_digest(arguments: Option<&Value>) -> Result<String> {
    let canonical = canonicalize_value(arguments.unwrap_or(&Value::Null));
    Ok(sha256_hex_parts(&[&serde_json::to_vec(&canonical)?]))
}

fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            let mut normalized = Map::new();
            for (key, value) in entries {
                normalized.insert(key.clone(), canonicalize_value(value));
            }
            Value::Object(normalized)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_value).collect()),
        _ => value.clone(),
    }
}

fn unix_ms(now: SystemTime) -> Result<u64> {
    Ok(now
        .duration_since(UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock before UNIX_EPOCH: {error}"))?
        .as_millis() as u64)
}

fn sha256_hex_parts(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding<'a>(arguments: &'a Value, principal: Option<&'a str>) -> RequestStateBinding<'a> {
        RequestStateBinding {
            method: "tools/call",
            tool: "purge_saved_context",
            arguments: Some(arguments),
            principal,
        }
    }

    #[test]
    fn request_state_round_trip_accept_shape() {
        let arguments = json!({"keep_days": 30, "purge_bridge_files": true});
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let request_state = issue_request_state_with_clock(
            binding(&arguments, Some("session-123")),
            now,
            Duration::from_secs(60),
        )
        .unwrap();
        validate_request_state_with_clock(
            &request_state,
            binding(&arguments, Some("session-123")),
            now + Duration::from_secs(30),
        )
        .unwrap();
        let result = InputRequiredResult {
            result_type: "input_required".to_owned(),
            input_requests: vec![InputRequest {
                id: "confirmation".to_owned(),
                request_type: "form".to_owned(),
                message: "confirm".to_owned(),
                requested_schema: json!({"type": "object"}),
            }],
            request_state,
        };
        assert_eq!(result.result_type, "input_required");
    }

    #[test]
    fn input_response_cancel_shape_is_supported() {
        let responses: InputResponses = serde_json::from_value(json!({
            "confirmation": {
                "action": "cancel"
            }
        }))
        .unwrap();
        assert_eq!(responses["confirmation"]["action"], json!("cancel"));
    }

    #[test]
    fn request_state_rejects_tampering() {
        let arguments = json!({"keep_days": 30});
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let request_state =
            issue_request_state_with_clock(binding(&arguments, None), now, Duration::from_secs(60))
                .unwrap();
        let mut envelope: SignedRequestState = serde_json::from_str(&request_state).unwrap();
        envelope.payload.tool = "other_tool".to_owned();
        let tampered = serde_json::to_string(&envelope).unwrap();
        let error = validate_request_state_with_clock(&tampered, binding(&arguments, None), now)
            .unwrap_err();
        assert!(error.to_string().contains("signature mismatch"));
    }

    #[test]
    fn request_state_rejects_expired_retries() {
        let arguments = json!({"keep_days": 30});
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let request_state =
            issue_request_state_with_clock(binding(&arguments, None), now, Duration::from_secs(1))
                .unwrap();
        let error = validate_request_state_with_clock(
            &request_state,
            binding(&arguments, None),
            now + Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(error.to_string().contains("expired"));
    }
}
