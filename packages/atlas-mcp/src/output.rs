//! MCP output rendering with deterministic JSON and optional TOON encoding.
//!
//! Supported TOON subset in Atlas:
//! - objects, arrays, strings, numbers, booleans, and null
//! - inline primitive arrays when encoder selects them
//! - tabular arrays of primitive-only objects when encoder selects them
//! - expanded nested or mixed arrays when encoder selects them
//!
//! Deliberate deviations:
//! - Atlas sorts object keys before encoding for deterministic output.
//! - Atlas falls back to JSON when TOON encoding fails, round-trip validation
//!   fails, or root output would be empty.

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::Value;
use toon_format::{decode_default, encode_default};

const OUTPUT_FORMAT_ENV: &str = "ATLAS_MCP_OUTPUT_FORMAT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Toon,
}

impl OutputFormat {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "toon" => Ok(Self::Toon),
            other => Err(anyhow!(
                "unsupported output_format '{other}'; expected 'json' or 'toon'"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toon => "toon",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Toon => "text/x-toon",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct RenderedPayload {
    pub requested_format: OutputFormat,
    pub actual_format: OutputFormat,
    pub text: String,
    pub fallback_reason: Option<String>,
}

pub fn resolve_output_format(
    args: Option<&Value>,
    default_format: OutputFormat,
) -> Result<OutputFormat> {
    if let Some(raw) = args
        .and_then(|value| value.get("output_format"))
        .and_then(|value| value.as_str())
    {
        return OutputFormat::parse(raw);
    }

    if let Ok(raw) = std::env::var(OUTPUT_FORMAT_ENV)
        && !raw.trim().is_empty()
    {
        return OutputFormat::parse(&raw);
    }

    Ok(default_format)
}

pub fn render_serializable<T: Serialize>(
    value: &T,
    requested_format: OutputFormat,
) -> Result<RenderedPayload> {
    let json = serde_json::to_value(value)?;
    render_value(&json, requested_format)
}

pub fn render_value(value: &Value, requested_format: OutputFormat) -> Result<RenderedPayload> {
    let normalized = normalize_json(value);
    match requested_format {
        OutputFormat::Json => render_json(normalized, requested_format, None),
        OutputFormat::Toon => match render_toon(&normalized) {
            Ok(text) => Ok(RenderedPayload {
                requested_format,
                actual_format: OutputFormat::Toon,
                text,
                fallback_reason: None,
            }),
            Err(error) => render_json(normalized, requested_format, Some(error.to_string())),
        },
    }
}

fn render_json(
    normalized: Value,
    requested_format: OutputFormat,
    fallback_reason: Option<String>,
) -> Result<RenderedPayload> {
    Ok(RenderedPayload {
        requested_format,
        actual_format: OutputFormat::Json,
        text: serde_json::to_string_pretty(&normalized)?,
        fallback_reason,
    })
}

fn render_toon(normalized: &Value) -> Result<String> {
    if matches!(normalized, Value::Object(map) if map.is_empty()) {
        return Err(anyhow!("root empty object would produce empty TOON output"));
    }

    let text = encode_default(normalized).map_err(|error| anyhow!(error.to_string()))?;
    if text.trim().is_empty() {
        return Err(anyhow!("TOON encoder produced empty output"));
    }

    let decoded: Value = decode_default(&text).map_err(|error| anyhow!(error.to_string()))?;
    if normalize_json(&decoded) != *normalized {
        return Err(anyhow!("TOON round-trip validation failed"));
    }

    Ok(text)
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
    use std::sync::Mutex;
    use tiktoken_rs::cl100k_base;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn parse_output_format_accepts_json_and_toon() {
        assert_eq!(
            OutputFormat::parse("json").expect("json"),
            OutputFormat::Json
        );
        assert_eq!(
            OutputFormat::parse("TOON").expect("toon"),
            OutputFormat::Toon
        );
    }

    #[test]
    fn resolve_output_format_prefers_argument_over_env() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        // SAFETY: test-scoped env mutation.
        unsafe {
            std::env::set_var(OUTPUT_FORMAT_ENV, "json");
        }
        let args = json!({ "output_format": "toon" });
        let resolved =
            resolve_output_format(Some(&args), OutputFormat::Json).expect("resolve output format");
        assert_eq!(resolved, OutputFormat::Toon);
        // SAFETY: test-scoped env mutation cleanup.
        unsafe {
            std::env::remove_var(OUTPUT_FORMAT_ENV);
        }
    }

    #[test]
    fn resolve_output_format_uses_default_when_no_override_exists() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        // SAFETY: test-scoped env cleanup.
        unsafe {
            std::env::remove_var(OUTPUT_FORMAT_ENV);
        }
        let resolved = resolve_output_format(None, OutputFormat::Toon).expect("resolve output");
        assert_eq!(resolved, OutputFormat::Toon);
    }

    #[test]
    fn resolve_output_format_prefers_env_over_default() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        // SAFETY: test-scoped env mutation.
        unsafe {
            std::env::set_var(OUTPUT_FORMAT_ENV, "json");
        }
        let resolved = resolve_output_format(None, OutputFormat::Toon).expect("resolve output");
        assert_eq!(resolved, OutputFormat::Json);
        // SAFETY: test-scoped env mutation cleanup.
        unsafe {
            std::env::remove_var(OUTPUT_FORMAT_ENV);
        }
    }

    #[test]
    fn render_toon_supports_uniform_rows_with_tabular_output() {
        let payload = json!({
            "users": [
                { "id": 1, "name": "Alice", "active": true },
                { "id": 2, "name": "Bob", "active": false }
            ]
        });

        let rendered = render_value(&payload, OutputFormat::Toon).expect("render toon");

        assert_eq!(rendered.actual_format, OutputFormat::Toon);
        assert!(rendered.text.contains("users[2]{active,id,name}:"));
    }

    #[test]
    fn render_toon_keeps_primitive_arrays_inline() {
        let payload = json!({ "tags": ["reading", "gaming", "coding"] });

        let rendered = render_value(&payload, OutputFormat::Toon).expect("render toon");

        assert_eq!(rendered.text, "tags[3]: reading,gaming,coding");
    }

    #[test]
    fn render_toon_uses_expanded_form_for_mixed_nested_arrays() {
        let payload = json!({
            "entries": [
                { "kind": "simple", "value": 42 },
                { "kind": "nested", "items": [1, 2, 3] }
            ]
        });

        let rendered = render_value(&payload, OutputFormat::Toon).expect("render toon");

        assert_eq!(rendered.actual_format, OutputFormat::Toon);
        assert!(rendered.text.contains("entries[2]:"));
        assert!(rendered.text.contains("kind: simple"));
        assert!(rendered.text.contains("items[3]: 1,2,3"));
    }

    #[test]
    fn render_toon_preserves_canonical_numbers() {
        let payload = json!({ "value": 1.5000 });

        let rendered = render_value(&payload, OutputFormat::Toon).expect("render toon");

        assert!(rendered.text.contains("value: 1.5"));
    }

    #[test]
    fn render_toon_supports_booleans_and_nulls() {
        let payload = json!({ "active": true, "disabled": false, "missing": null });

        let rendered = render_value(&payload, OutputFormat::Toon).expect("render toon");

        assert!(rendered.text.contains("active: true"));
        assert!(rendered.text.contains("disabled: false"));
        assert!(rendered.text.contains("missing: null"));
    }

    #[test]
    fn render_toon_quotes_strings_that_conflict_with_delimiters() {
        let payload = json!({ "tags": ["a,b", "plain"] });

        let rendered = render_value(&payload, OutputFormat::Toon).expect("render toon");

        assert!(rendered.text.contains("\"a,b\""));
    }

    #[test]
    fn render_toon_falls_back_to_json_for_empty_root_object() {
        let payload = json!({});

        let rendered = render_value(&payload, OutputFormat::Toon).expect("render payload");

        assert_eq!(rendered.actual_format, OutputFormat::Json);
        assert_eq!(rendered.requested_format, OutputFormat::Toon);
        assert!(
            rendered
                .fallback_reason
                .expect("fallback reason")
                .contains("empty TOON output")
        );
    }

    #[test]
    fn render_json_is_deterministic_for_object_key_order() {
        let payload = json!({ "b": 2, "a": 1 });

        let rendered = render_value(&payload, OutputFormat::Json).expect("render json");

        assert_eq!(rendered.text, "{\n  \"a\": 1,\n  \"b\": 2\n}");
    }

    #[test]
    fn representative_context_payload_uses_fewer_tokens_than_json() {
        let payload = json!({
            "intent": "review",
            "node_count": 3,
            "nodes": [
                {
                    "reason": "direct_target",
                    "distance": 0,
                    "qn": "src/api.rs::fn::handle_request",
                    "kind": "function",
                    "file": "src/api.rs",
                    "line": 12,
                    "lang": "rust"
                },
                {
                    "reason": "caller",
                    "distance": 1,
                    "qn": "src/router.rs::fn::route",
                    "kind": "function",
                    "file": "src/router.rs",
                    "line": 8,
                    "lang": "rust"
                },
                {
                    "reason": "test",
                    "distance": 1,
                    "qn": "tests/api.rs::fn::handle_request_test",
                    "kind": "test",
                    "file": "tests/api.rs",
                    "line": 4,
                    "lang": "rust"
                }
            ],
            "edge_count": 2,
            "edges": [
                {
                    "reason": "caller",
                    "from": "src/router.rs::fn::route",
                    "to": "src/api.rs::fn::handle_request",
                    "kind": "calls"
                },
                {
                    "reason": "test",
                    "from": "tests/api.rs::fn::handle_request_test",
                    "to": "src/api.rs::fn::handle_request",
                    "kind": "tests"
                }
            ],
            "file_count": 2,
            "files": [
                { "path": "src/api.rs", "reason": "direct_target", "line_ranges": [[12, 24]] },
                { "path": "tests/api.rs", "reason": "test", "line_ranges": [[1, 16]] }
            ],
            "truncated": false,
            "nodes_dropped": 0,
            "edges_dropped": 0
        });

        let json_rendered = render_value(&payload, OutputFormat::Json).expect("render json");
        let toon_rendered = render_value(&payload, OutputFormat::Toon).expect("render toon");
        let bpe = cl100k_base().expect("cl100k_base");
        let json_tokens = bpe.encode_with_special_tokens(&json_rendered.text).len();
        let toon_tokens = bpe.encode_with_special_tokens(&toon_rendered.text).len();

        assert_eq!(toon_rendered.actual_format, OutputFormat::Toon);
        assert!(toon_rendered.text.len() < json_rendered.text.len());
        assert!(toon_tokens < json_tokens);
    }
}
