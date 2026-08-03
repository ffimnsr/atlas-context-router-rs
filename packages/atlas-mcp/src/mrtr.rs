use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use rmcp::model::{
    ElicitRequest, ElicitRequestParams, ElicitationSchema, InputRequest, InputRequiredResult,
    RequestStateCodec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::output::OutputFormat;
use crate::tool_result::ToolResultBuilder;

const DEFAULT_REQUEST_STATE_TTL: Duration = Duration::from_secs(10 * 60);
const REQUEST_STATE_VERSION: u32 = 1;

static REQUEST_STATE_SECRET: OnceLock<Vec<u8>> = OnceLock::new();
static REQUEST_STATE_NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestStateBinding<'a> {
    pub(crate) method: &'a str,
    pub(crate) tool: &'a str,
    pub(crate) arguments: Option<&'a Value>,
    pub(crate) principal: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestStatePayload {
    version: u32,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    nonce: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestStateAssociatedData {
    method: String,
    tool: String,
    args_digest: String,
    principal: Option<String>,
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

pub(crate) fn build_form_input_required_result(
    request_id: impl Into<String>,
    message: impl Into<String>,
    requested_schema: Value,
    request_state: impl Into<String>,
) -> Result<InputRequiredResult> {
    let request = InputRequest::Elicitation(ElicitRequest::new(
        ElicitRequestParams::FormElicitationParams {
            meta: None,
            message: message.into(),
            requested_schema: serde_json::from_value::<ElicitationSchema>(requested_schema)?,
        },
    ));
    let input_requests = BTreeMap::from([(request_id.into(), request)]);
    Ok(InputRequiredResult::new(
        Some(input_requests),
        Some(request_state.into()),
    ))
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
        issued_at_unix_ms,
        expires_at_unix_ms,
        nonce: format!(
            "{}-{}",
            std::process::id(),
            REQUEST_STATE_NONCE_COUNTER.fetch_add(1, Ordering::Relaxed) + 1
        ),
    };
    let associated_data = request_state_associated_data_bytes(binding)?;
    request_state_codec()
        .seal_json_with(
            &payload,
            &rmcp::model::SealOptions::new().associated_data(&associated_data),
        )
        .map_err(map_request_state_codec_error)
}

fn validate_request_state_with_clock(
    request_state: &str,
    binding: RequestStateBinding<'_>,
    now: SystemTime,
) -> Result<()> {
    let associated_data = request_state_associated_data_bytes(binding)?;
    let payload: RequestStatePayload = request_state_codec()
        .open_json_with(request_state, &associated_data)
        .map_err(map_request_state_codec_error)?;
    if payload.version != REQUEST_STATE_VERSION {
        bail!(
            "requestState version mismatch: expected {}, got {}",
            REQUEST_STATE_VERSION,
            payload.version
        );
    }
    let now_unix_ms = unix_ms(now)?;
    if now_unix_ms > payload.expires_at_unix_ms {
        bail!("requestState expired");
    }
    Ok(())
}

fn request_state_codec() -> RequestStateCodec {
    RequestStateCodec::new(request_state_secret().clone())
}

fn request_state_secret() -> &'static Vec<u8> {
    REQUEST_STATE_SECRET.get_or_init(|| {
        std::env::var("ATLAS_MCP_REQUEST_STATE_SECRET")
            .map(|value| value.into_bytes())
            .unwrap_or_else(|_| {
                let seed_now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|value| value.as_nanos())
                    .unwrap_or_default();
                sha256_bytes(&[format!("atlas-mcp:{}:{seed_now}", std::process::id()).as_bytes()])
            })
    })
}

fn request_state_associated_data_bytes(binding: RequestStateBinding<'_>) -> Result<Vec<u8>> {
    let payload = RequestStateAssociatedData {
        method: binding.method.to_owned(),
        tool: binding.tool.to_owned(),
        args_digest: args_digest(binding.arguments)?,
        principal: binding.principal.map(str::to_owned),
    };
    Ok(serde_json::to_vec(&payload)?)
}

fn map_request_state_codec_error(error: rmcp::model::RequestStateError) -> anyhow::Error {
    match error {
        rmcp::model::RequestStateError::IntegrityCheckFailed => {
            anyhow!("requestState signature mismatch")
        }
        rmcp::model::RequestStateError::Expired => anyhow!("requestState expired"),
        other => anyhow!("requestState must be valid sealed JSON: {other}"),
    }
}

fn args_digest(arguments: Option<&Value>) -> Result<String> {
    let canonical = canonicalize_value(arguments.unwrap_or(&Value::Null));
    Ok(hex_encode(&sha256_bytes(&[&serde_json::to_vec(
        &canonical,
    )?])))
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

fn sha256_bytes(parts: &[&[u8]]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().to_vec()
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
    use rmcp::model::ElicitRequestParams;
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

        let result = build_form_input_required_result(
            "confirmation",
            "confirm",
            json!({
                "type": "object",
                "properties": {
                    "confirmation": {"type": "string"}
                }
            }),
            request_state,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&result).unwrap()["resultType"],
            json!("input_required")
        );
    }

    #[test]
    fn input_required_result_preserves_official_shape() {
        let result = build_form_input_required_result(
            "confirmation",
            "confirm",
            json!({
                "type": "object",
                "properties": {
                    "confirmation": {"type": "string"}
                }
            }),
            "sealed",
        )
        .unwrap();
        let requests = result.input_requests.expect("input requests");
        let request = requests.get("confirmation").expect("confirmation request");
        match request {
            InputRequest::Elicitation(request) => match &request.params {
                ElicitRequestParams::FormElicitationParams {
                    message,
                    requested_schema,
                    ..
                } => {
                    assert_eq!(message, "confirm");
                    assert_eq!(
                        serde_json::to_value(requested_schema).unwrap()["type"],
                        json!("object")
                    );
                }
                other => panic!("expected form elicitation, got {other:?}"),
            },
            other => panic!("expected elicitation input request, got {other:?}"),
        }
    }

    #[test]
    fn input_response_cancel_shape_is_supported() {
        let responses: rmcp::model::InputResponses = serde_json::from_value(json!({
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
        let mut parts = request_state
            .split('.')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(parts.len(), 3);
        let tag = parts[2].clone();
        let mut chars = tag.chars().collect::<Vec<_>>();
        chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
        parts[2] = chars.into_iter().collect();
        let tampered = parts.join(".");
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

    #[test]
    fn request_state_rejects_mismatched_arguments() {
        let arguments = json!({"keep_days": 30});
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let request_state =
            issue_request_state_with_clock(binding(&arguments, None), now, Duration::from_secs(60))
                .unwrap();
        let mismatch = json!({"keep_days": 60});
        let error =
            validate_request_state_with_clock(&request_state, binding(&mismatch, None), now)
                .unwrap_err();
        assert!(error.to_string().contains("signature mismatch"));
    }

    #[test]
    fn request_state_rejects_mismatched_principal() {
        let arguments = json!({"keep_days": 30});
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let request_state = issue_request_state_with_clock(
            binding(&arguments, Some("user-a")),
            now,
            Duration::from_secs(60),
        )
        .unwrap();
        let error = validate_request_state_with_clock(
            &request_state,
            binding(&arguments, Some("user-b")),
            now,
        )
        .unwrap_err();
        assert!(error.to_string().contains("signature mismatch"));
    }

    #[test]
    fn request_state_rejects_mismatched_tool() {
        let arguments = json!({"keep_days": 30});
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let request_state =
            issue_request_state_with_clock(binding(&arguments, None), now, Duration::from_secs(60))
                .unwrap();
        let error = validate_request_state_with_clock(
            &request_state,
            RequestStateBinding {
                method: "tools/call",
                tool: "other_tool",
                arguments: Some(&arguments),
                principal: None,
            },
            now,
        )
        .unwrap_err();
        assert!(error.to_string().contains("signature mismatch"));
    }

    #[test]
    fn request_state_rejects_mismatched_method() {
        let arguments = json!({"keep_days": 30});
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let request_state =
            issue_request_state_with_clock(binding(&arguments, None), now, Duration::from_secs(60))
                .unwrap();
        let error = validate_request_state_with_clock(
            &request_state,
            RequestStateBinding {
                method: "prompts/get",
                tool: "purge_saved_context",
                arguments: Some(&arguments),
                principal: None,
            },
            now,
        )
        .unwrap_err();
        assert!(error.to_string().contains("signature mismatch"));
    }
}
