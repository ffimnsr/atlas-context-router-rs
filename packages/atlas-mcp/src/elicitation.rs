use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};

use crate::runtime_context;

const DEFAULT_ELICIT_TIMEOUT: Duration = Duration::from_secs(120);

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

#[derive(Debug, Clone)]
pub(crate) struct FormElicitation {
    pub message: String,
    pub requested_schema: Value,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct UrlElicitation {
    pub elicitation_id: String,
    pub message: String,
    pub url: String,
}

pub(crate) fn create_form(request: FormElicitation) -> Result<ElicitationResponse> {
    let client = runtime_context::current()?;
    if !client.capabilities.supports_elicitation_form {
        bail!("client does not advertise elicitation.form capability");
    }
    let params = json!({
        "mode": "form",
        "message": request.message,
        "requestedSchema": request.requested_schema,
    });
    let raw = client.request("elicitation/create", params.clone(), DEFAULT_ELICIT_TIMEOUT)?;
    parse_and_validate_response(&params, &raw)
}

#[allow(dead_code)]
pub(crate) fn create_url(request: UrlElicitation) -> Result<ElicitationResponse> {
    let client = runtime_context::current()?;
    if !client.capabilities.supports_elicitation_url {
        bail!("client does not advertise elicitation.url capability");
    }
    let params = json!({
        "mode": "url",
        "elicitationId": request.elicitation_id,
        "message": request.message,
        "url": request.url,
    });
    let raw = client.request("elicitation/create", params.clone(), DEFAULT_ELICIT_TIMEOUT)?;
    parse_and_validate_response(&params, &raw)
}

pub(crate) fn confirm_age_based_purge() -> Result<bool> {
    let response = create_form(FormElicitation {
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
    })?;
    Ok(matches!(
        response,
        ElicitationResponse {
            action: ElicitationAction::Accept,
            content: Some(ref content),
        } if content.get("confirmation") == Some(&Value::String("confirm".to_owned()))
    ))
}

fn parse_and_validate_response(
    request_params: &Value,
    response: &Value,
) -> Result<ElicitationResponse> {
    let object = response
        .as_object()
        .ok_or_else(|| anyhow!("elicitation response must be an object"))?;
    let action = match object.get("action").and_then(Value::as_str) {
        Some("accept") => ElicitationAction::Accept,
        Some("cancel") => ElicitationAction::Cancel,
        Some("decline") => ElicitationAction::Decline,
        Some(other) => bail!("unsupported elicitation action '{other}'"),
        None => bail!("elicitation response missing action"),
    };
    let content = object.get("content").and_then(Value::as_object).cloned();
    if request_params.get("mode").and_then(Value::as_str) == Some("url") {
        if content.is_some() {
            bail!("URL elicitation response must not include content");
        }
        return Ok(ElicitationResponse {
            action,
            content: None,
        });
    }
    let schema = request_params
        .get("requestedSchema")
        .ok_or_else(|| anyhow!("form elicitation missing requestedSchema"))?;
    let validated = match action {
        ElicitationAction::Accept => Some(validate_form_content(schema, content.as_ref())?),
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
    let values = value
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("field '{name}' must be an array"))?;
    if let Some(min_items) = field.get("minItems").and_then(Value::as_u64)
        && (values.len() as u64) < min_items
    {
        bail!("field '{name}' must contain at least {min_items} items");
    }
    if let Some(max_items) = field.get("maxItems").and_then(Value::as_u64)
        && (values.len() as u64) > max_items
    {
        bail!("field '{name}' must contain at most {max_items} items");
    }
    let items = field
        .get("items")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("field '{name}' multi-select schema missing items"))?;
    let allowed = extract_multi_select_options(items)?;
    let mut normalized = Vec::with_capacity(values.len());
    for item in values {
        let selected = item
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("field '{name}' values must be strings"))?;
        if !allowed.contains(selected.as_str()) {
            bail!("field '{name}' contains unsupported enum value '{selected}'");
        }
        normalized.push(Value::String(selected));
    }
    Ok(Value::Array(normalized))
}

fn extract_multi_select_options(items: &Map<String, Value>) -> Result<BTreeSet<&str>> {
    if let Some(values) = items.get("enum").and_then(Value::as_array) {
        return Ok(values.iter().filter_map(Value::as_str).collect());
    }
    if let Some(values) = items.get("anyOf").and_then(Value::as_array) {
        return Ok(values
            .iter()
            .filter_map(|item| item.get("const"))
            .filter_map(Value::as_str)
            .collect());
    }
    bail!("multi-select enum schema missing enum or anyOf options")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_validation_applies_defaults_and_titled_single_select() {
        let schema = json!({
            "type": "object",
            "properties": {
                "confirmation": {
                    "type": "string",
                    "oneOf": [
                        {"const": "confirm", "title": "Confirm"},
                        {"const": "cancel", "title": "Cancel"}
                    ],
                    "default": "confirm"
                },
                "reason": {
                    "type": "string",
                    "default": "cleanup"
                }
            },
            "required": ["confirmation"]
        });
        let validated = validate_form_content(&schema, Some(&Map::new())).unwrap();
        assert_eq!(
            validated.get("confirmation"),
            Some(&Value::String("confirm".to_owned()))
        );
        assert_eq!(
            validated.get("reason"),
            Some(&Value::String("cleanup".to_owned()))
        );
    }

    #[test]
    fn form_validation_supports_multi_select_any_of() {
        let schema = json!({
            "type": "object",
            "properties": {
                "choices": {
                    "type": "array",
                    "items": {
                        "anyOf": [
                            {"const": "a", "title": "A"},
                            {"const": "b", "title": "B"}
                        ]
                    }
                }
            }
        });
        let mut submitted = Map::new();
        submitted.insert("choices".to_owned(), json!(["a", "b"]));
        let validated = validate_form_content(&schema, Some(&submitted)).unwrap();
        assert_eq!(validated.get("choices"), Some(&json!(["a", "b"])));
    }

    #[test]
    fn url_responses_reject_content() {
        let error = parse_and_validate_response(
            &json!({"mode": "url", "elicitationId": "abc", "message": "m", "url": "https://example.com"}),
            &json!({"action": "accept", "content": {"bad": true}}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("must not include content"));
    }
}
