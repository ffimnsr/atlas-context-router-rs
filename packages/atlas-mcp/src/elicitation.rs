use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use rmcp::model::{
    ElicitResult, ElicitationAction, ElicitationSchema, EnumSchema, InputRequiredResult,
    InputResponses,
};
use serde_json::{Map, Value};

use crate::mrtr::{self, RequestStateBinding};
use crate::runtime_context;

const CONFIRMATION_REQUEST_ID: &str = "confirmation";

#[derive(Debug, Clone)]
pub(crate) enum ConfirmationProgress {
    Confirmed,
    Cancelled,
    InputRequired(InputRequiredResult),
}

#[derive(Debug, Clone)]
pub(crate) struct FormElicitation {
    pub message: String,
    pub requested_schema: ElicitationSchema,
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
                ElicitResult {
                    action: ElicitationAction::Accept,
                    content: Some(ref content),
                    ..
                } if content.get("confirmation") == Some(&Value::String("confirm".to_owned())) => {
                    ConfirmationProgress::Confirmed
                }
                _ => ConfirmationProgress::Cancelled,
            })
        }
        (None, None) => Ok(ConfirmationProgress::InputRequired(
            mrtr::build_form_input_required_result(
                CONFIRMATION_REQUEST_ID,
                request.message,
                serde_json::to_value(request.requested_schema)?,
                mrtr::issue_request_state(binding)?,
            )?,
        )),
        (Some(_), None) => bail!("requestState retry requires params.inputResponses"),
        (None, Some(_)) => bail!("params.inputResponses require requestState"),
    }
}

pub(crate) fn confirm_age_based_purge() -> Result<ConfirmationProgress> {
    create_form(FormElicitation {
        message: "Confirm purge of saved context across all sessions older than keep_days. This cannot be undone.".to_owned(),
        requested_schema: confirmation_schema()?,
    })
}

fn confirmation_schema() -> Result<ElicitationSchema> {
    ElicitationSchema::builder()
        .required_enum_schema(
            CONFIRMATION_REQUEST_ID,
            EnumSchema::builder(vec!["confirm".to_owned(), "cancel".to_owned()])
                .title("Confirmation")
                .enum_titles(vec![
                    "Purge saved context".to_owned(),
                    "Do not purge".to_owned(),
                ])
                .map_err(|error| anyhow!(error))?
                .with_default("cancel")
                .map_err(|error| anyhow!(error))?
                .build(),
        )
        .build()
        .map_err(|error| anyhow!(error))
}

fn parse_form_response(
    requested_schema: &ElicitationSchema,
    responses: &Value,
) -> Result<ElicitResult> {
    let responses: InputResponses = serde_json::from_value(responses.clone())
        .map_err(|error| anyhow!("params.inputResponses must be an object: {error}"))?;
    let response = responses.get(CONFIRMATION_REQUEST_ID).ok_or_else(|| {
        anyhow!("params.inputResponses missing '{CONFIRMATION_REQUEST_ID}' response")
    })?;
    let response: ElicitResult = serde_json::from_value(response.clone()).map_err(|error| {
        anyhow!(
            "input response '{CONFIRMATION_REQUEST_ID}' must be valid elicitation result: {error}"
        )
    })?;
    let validated = match response.action {
        ElicitationAction::Accept => Some(validate_form_content(
            requested_schema,
            response.content.as_ref(),
        )?),
        ElicitationAction::Cancel | ElicitationAction::Decline => None,
        _ => None,
    };
    let mut result = ElicitResult::new(response.action);
    if let Some(content) = validated {
        result = result.with_content(Value::Object(content));
    }
    if let Some(meta) = response.meta {
        result = result.with_meta(meta);
    }
    Ok(result)
}

fn validate_form_content(
    schema: &ElicitationSchema,
    submitted: Option<&Value>,
) -> Result<Map<String, Value>> {
    let schema = serde_json::to_value(schema)?;
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
    let submitted = match submitted {
        Some(Value::Object(object)) => object.clone(),
        Some(_) => bail!("elicitation response content must be an object"),
        None => Map::new(),
    };
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
    Ok(Value::String(selected))
}

fn validate_array_field(name: &str, field: &Map<String, Value>, value: Value) -> Result<Value> {
    let items = value
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("field '{name}' must be array"))?;
    if let Some(item_schema) = field.get("items") {
        for item in &items {
            let _ = validate_field_value(name, item_schema, item.clone())?;
        }
    }
    Ok(Value::Array(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_schema_uses_rmcp_builder_and_preserves_contract() {
        let schema = confirmation_schema().expect("confirmation schema");
        let raw = serde_json::to_value(schema).expect("schema json");
        assert_eq!(raw["type"], serde_json::json!("object"));
        assert_eq!(raw["required"], serde_json::json!(["confirmation"]));
        assert_eq!(
            raw["properties"]["confirmation"]["title"],
            serde_json::json!("Confirmation")
        );
        assert_eq!(
            raw["properties"]["confirmation"]["default"],
            serde_json::json!("cancel")
        );
        assert_eq!(
            raw["properties"]["confirmation"]["oneOf"],
            serde_json::json!([
                {"const": "confirm", "title": "Purge saved context"},
                {"const": "cancel", "title": "Do not purge"}
            ])
        );
    }

    #[test]
    fn invalid_response_content_is_rejected() {
        let schema: ElicitationSchema = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "confirmation": {"type": "string"}
            },
            "required": ["confirmation"]
        }))
        .expect("typed schema");
        let error = parse_form_response(
            &schema,
            &serde_json::json!({
                "confirmation": {
                    "action": "accept",
                    "content": "bad"
                }
            }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("content must be an object"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let schema: ElicitationSchema = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "confirmation": {"type": "string"}
            },
            "required": ["confirmation"]
        }))
        .expect("typed schema");
        let error = parse_form_response(
            &schema,
            &serde_json::json!({
                "confirmation": {
                    "action": "accept",
                    "content": {
                        "confirmation": "confirm",
                        "extra": true
                    }
                }
            }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field 'extra'"));
    }

    #[test]
    fn default_cancel_behavior_is_supported() {
        let schema: ElicitationSchema = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "confirmation": {
                    "type": "string",
                    "default": "cancel"
                }
            },
            "required": ["confirmation"]
        }))
        .expect("typed schema");
        let result = parse_form_response(
            &schema,
            &serde_json::json!({
                "confirmation": {
                    "action": "accept",
                    "content": {}
                }
            }),
        )
        .unwrap();
        assert_eq!(result.action, ElicitationAction::Accept);
        assert_eq!(
            result.content,
            Some(serde_json::json!({"confirmation": "cancel"}))
        );
    }
}
