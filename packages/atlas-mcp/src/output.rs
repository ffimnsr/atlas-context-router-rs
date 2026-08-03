//! MCP output rendering with deterministic JSON.

use anyhow::Result;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RenderedPayload {
    pub actual_format: OutputFormat,
    pub text: String,
}

pub fn render_value(value: &Value) -> Result<RenderedPayload> {
    Ok(RenderedPayload {
        actual_format: OutputFormat::Json,
        text: serde_json::to_string_pretty(&normalize_json(value))?,
    })
}

fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(boolean) => Value::Bool(*boolean),
        Value::Number(number) => Value::Number(number.clone()),
        Value::String(text) => Value::String(text.clone()),
        Value::Array(items) => Value::Array(items.iter().map(normalize_json).collect()),
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));

            let normalized = entries
                .into_iter()
                .map(|(key, value)| (key.clone(), normalize_json(value)))
                .collect();
            Value::Object(normalized)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_json_is_deterministic_for_object_key_order() {
        let payload = json!({ "b": 2, "a": 1 });

        let rendered = render_value(&payload).expect("render json");

        assert_eq!(rendered.actual_format, OutputFormat::Json);
        assert_eq!(rendered.text, "{\n  \"a\": 1,\n  \"b\": 2\n}");
    }

    #[test]
    fn render_json_preserves_nested_arrays_and_scalars() {
        let payload = json!({
            "entries": [
                { "kind": "simple", "value": 42 },
                { "kind": "nested", "items": [1, 2, 3] },
                true,
                null,
            ]
        });

        let rendered = render_value(&payload).expect("render json");
        let reparsed: Value = serde_json::from_str(&rendered.text).expect("reparse rendered json");

        assert_eq!(reparsed, payload);
    }
}
