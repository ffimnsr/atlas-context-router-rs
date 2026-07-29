use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::mrtr::{self, InputRequest, InputRequiredResult, InputResponses, RequestStateBinding};
use crate::runtime_context;

const CONFIRMATION_REQUEST_ID: &str = "confirmation";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ElicitationAction {
    Accept,
    Cancel,
    Decline,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElicitationResponse {
    pub action: ElicitationAction,
    pub content: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConfirmationProgress {
    Confirmed,
    Cancelled,
    InputRequired(InputRequiredResult),
}

#[derive(Debug, Clone)]
pub(crate) struct FormElicitation {
    pub message: String,
    pub requested_schema: Value,
}

fn create_form(request: FormElicitation) -> Result<ConfirmationProgress> {
    let context = runtime_context::current()?;
    if !context.capabilities.supports_elicitation_form {
        bail!("client does not advertise elicitation.form capability");
    }
    if context.request_method != "tools/call" {
        bail!(
            "MRTR input_required flow only supports tools/call retries; got '{}'",
            context.request_method
        );
    }
    let request_params = context
        .request_params
        .as_ref()
        .ok_or_else(|| anyhow!("active MCP request context missing tools/call params"))?;
    let tool_name = request_params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tools/call params missing name for MRTR elicitation"))?;
    let arguments = request_params.get("arguments");
    let binding = RequestStateBinding {
        method: "tools/call",
        tool: tool_name,
        arguments,
        principal: context.authenticated_principal.as_deref(),
    };
    let request_state = request_params.get("requestState").and_then(Value::as_str);
    let input_responses = request_params.get("inputResponses");

    match (request_state, input_responses) {
        (Some(state), Some(responses)) => {
            mrtr::validate_request_state(state, binding)?;
            let response = parse_form_response(&request.requested_schema, responses)?;
            Ok(match response {
                ElicitationResponse {
                    action: ElicitationAction::Accept,
                    content: Some(ref content),
                } if content.get("confirmation") == Some(&Value::String("confirm".to_owned())) => {
                    ConfirmationProgress::Confirmed
                }
                _ => ConfirmationProgress::Cancelled,
            })
        }
        (None, None) => Ok(ConfirmationProgress::InputRequired(InputRequiredResult {
            result_type: "input_required".to_owned(),
            input_requests: vec![InputRequest {
                id: CONFIRMATION_REQUEST_ID.to_owned(),
                request_type: "form".to_owned(),
                message: request.message,
                requested_schema: request.requested_schema,
            }],
            request_state: mrtr::issue_request_state(binding)?,
        })),
        (Some(_), None) => bail!("requestState retry requires params.inputResponses"),
        (None, Some(_)) => bail!("params.inputResponses require requestState"),
    }
}

pub(crate) fn confirm_age_based_purge() -> Result<ConfirmationProgress> {
    create_form(FormElicitation {
        message: "Confirm purge of saved context across all sessions older than keep_days. This cannot be undone.".to_owned(),
        requested_schema: json!({
            "type": "object",
            "properties": {
                "confirmation": {
                    "type": "string",
                    "title": "Confirmation",
                    "oneOf": [
                        { "const": "confirm", "title": "Purge saved context" },
                        { "const": "cancel", "title": "Do not purge" }
                    ],
                    "default": "cancel"
                }
            },
            "required": ["confirmation"]
        }),
    })
}

fn parse_form_response(requested_schema: &Value, responses: &Value) -> Result<ElicitationResponse> {
    let responses: InputResponses = serde_json::from_value(responses.clone())
        .map_err(|error| anyhow!("params.inputResponses must be an object: {error}"))?;
    let response = responses.get(CONFIRMATION_REQUEST_ID).ok_or_else(|| {
        anyhow!("params.inputResponses missing '{CONFIRMATION_REQUEST_ID}' response")
    })?;
    let object = response
        .as_object()
        .ok_or_else(|| anyhow!("input response '{CONFIRMATION_REQUEST_ID}' must be an object"))?;
    let action = match object.get("action").and_then(Value::as_str) {
        Some("accept") => ElicitationAction::Accept,
        Some("cancel") => ElicitationAction::Cancel,
        Some("decline") => ElicitationAction::Decline,
        Some(other) => bail!("unsupported elicitation action '{other}'"),
        None => bail!("elicitation response missing action"),
    };
    let content = object.get("content").and_then(Value::as_object).cloned();
    let validated = match action {
        ElicitationAction::Accept => {
            Some(validate_form_content(requested_schema, content.as_ref())?)
        }
        ElicitationAction::Cancel | ElicitationAction::Decline => None,
    };
    Ok(ElicitationResponse {
        action,
        content: validated,
    })
}

fn validate_form_content(
    schema: &Value,
    submitted: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>> {
    let schema = schema
        .as_object()
        .ok_or_else(|| anyhow!("requestedSchema must be an object"))?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        bail!("requestedSchema.type must be 'object'");
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("requestedSchema.properties must be an object"))?;
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let submitted = submitted.cloned().unwrap_or_default();
    let mut validated = Map::new();

    for (name, definition) in properties {
        let value = submitted
            .get(name)
            .cloned()
            .or_else(|| definition.get("default").cloned());
        match value {
            Some(value) => {
                validated.insert(name.clone(), validate_field_value(name, definition, value)?);
            }
            None if required.contains(name) => {
                bail!("elicitation response missing required field '{name}'")
            }
            None => {}
        }
    }

    for key in submitted.keys() {
        if !properties.contains_key(key) {
            bail!("elicitation response contains unknown field '{key}'");
        }
    }

    Ok(validated)
}

fn validate_field_value(name: &str, definition: &Value, value: Value) -> Result<Value> {
    let field = definition
        .as_object()
        .ok_or_else(|| anyhow!("schema definition for '{name}' must be an object"))?;
    match field.get("type").and_then(Value::as_str) {
        Some("string") => validate_string_field(name, field, value),
        Some("boolean") => {
            if value.is_boolean() {
                Ok(value)
            } else {
                bail!("field '{name}' must be boolean")
            }
        }
        Some("integer") | Some("number") => {
            if value.is_i64() || value.is_u64() || value.is_f64() {
                Ok(value)
            } else {
                bail!("field '{name}' must be numeric")
            }
        }
        Some("array") => validate_array_field(name, field, value),
        Some(other) => bail!("unsupported schema type '{other}' for field '{name}'"),
        None => bail!("schema definition for '{name}' missing type"),
    }
}

fn validate_string_field(name: &str, field: &Map<String, Value>, value: Value) -> Result<Value> {
    let selected = value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("field '{name}' must be string"))?;
    if let Some(options) = field.get("enum").and_then(Value::as_array) {
        let allowed = options
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        if !allowed.contains(selected.as_str()) {
            bail!("field '{name}' contains unsupported enum value '{selected}'");
        }
        return Ok(Value::String(selected));
    }
    if let Some(options) = field.get("oneOf").and_then(Value::as_array) {
        let allowed = options
            .iter()
            .filter_map(|item| item.get("const"))
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>();
        if !allowed.contains(selected.as_str()) {
            bail!("field '{name}' contains unsupported enum value '{selected}'");
        }
        return Ok(Value::String(selected));
    }
    if let Some(options) = field.get("enumNames") {
        let _ = options;
    }
    Ok(Value::String(selected))
}

fn validate_array_field(name: &str, field: &Map<String, Value>, value: Value) -> Result<Value> {
    let items = value
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("field '{name}' must be array"))?;
    if let Some(item_schema) = field.get("items") {
        let validated = items
            .into_iter()
            .map(|item| validate_field_value(name, item_schema, item))
            .collect::<Result<Vec<_>>>()?;
        return Ok(Value::Array(validated));
    }
    Ok(Value::Array(items))
}
