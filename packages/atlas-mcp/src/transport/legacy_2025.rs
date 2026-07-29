use anyhow::Result;
use serde_json::Value;

use crate::spec;

use super::types::ConnectionState;

pub(crate) const LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";

pub(crate) fn legacy_initialize_response(params: Option<&Value>) -> Option<Result<Value>> {
    let request = match spec::parse_initialize_request(params) {
        Ok(request) if request.protocol_version == LEGACY_PROTOCOL_VERSION => request,
        Ok(_) => return None,
        Err(error) => return Some(Err(error)),
    };
    Some(serde_json::to_value(spec::initialize_result(&request)).map_err(Into::into))
}

pub(crate) fn note_initialize_request(
    connection_state: &mut ConnectionState,
    params: Option<&Value>,
) {
    let Some(params) = params.and_then(Value::as_object) else {
        return;
    };
    let protocol_version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if protocol_version == LEGACY_PROTOCOL_VERSION {
        connection_state.client_capabilities = params
            .get("capabilities")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
    } else if protocol_version == spec::MCP_PROTOCOL_VERSION {
        connection_state.client_capabilities = Value::Null;
    }
}

pub(crate) fn allows_missing_request_meta(connection_state: &ConnectionState) -> bool {
    connection_state.client_capabilities.is_object()
}

pub(crate) fn is_legacy_initialized_notification(method: &str) -> bool {
    matches!(method, "initialized" | "notifications/initialized")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        LEGACY_PROTOCOL_VERSION, allows_missing_request_meta, is_legacy_initialized_notification,
        legacy_initialize_response, note_initialize_request,
    };
    use crate::spec;
    use crate::transport::types::connection_state;

    #[test]
    fn legacy_initialize_response_reuses_modern_result_shape() {
        let response = legacy_initialize_response(Some(&json!({
            "protocolVersion": LEGACY_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "zed", "version": "1.0.0"}
        })))
        .expect("legacy initialize handled")
        .expect("legacy initialize result");
        assert_eq!(
            response["protocolVersion"],
            json!(spec::MCP_PROTOCOL_VERSION)
        );
    }

    #[test]
    fn legacy_initialize_enables_missing_meta_fallback() {
        let mut state = connection_state(None, None, false);
        note_initialize_request(
            &mut state,
            Some(&json!({
                "protocolVersion": LEGACY_PROTOCOL_VERSION,
                "capabilities": {"roots": {"listChanged": true}}
            })),
        );
        assert!(allows_missing_request_meta(&state));
    }

    #[test]
    fn modern_initialize_does_not_enable_missing_meta_fallback() {
        let mut state = connection_state(None, None, false);
        note_initialize_request(
            &mut state,
            Some(&json!({
                "protocolVersion": spec::MCP_PROTOCOL_VERSION,
                "capabilities": {"roots": {"listChanged": true}}
            })),
        );
        assert!(!allows_missing_request_meta(&state));
    }

    #[test]
    fn legacy_initialized_notification_names_are_recognized() {
        assert!(is_legacy_initialized_notification("initialized"));
        assert!(is_legacy_initialized_notification(
            "notifications/initialized"
        ));
        assert!(!is_legacy_initialized_notification("notifications/message"));
    }
}
