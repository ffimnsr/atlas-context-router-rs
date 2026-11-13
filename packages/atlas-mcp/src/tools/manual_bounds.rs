//! Payload limiting for `man` / `tool_help` manual documents.
//!
//! Manual documents expand every input and output schema field, so a single
//! call can emit several thousand tokens. This module keeps that bounded:
//!
//! * compact default: drops optional examples/notes unless `compact: false`;
//! * `max_bytes` ceiling enforced on the serialized document;
//! * synthesized field descriptions instead of a placeholder string.
use serde_json::Value;

use super::manual::ToolManualDocument;

/// Default serialized ceiling for one manual payload.
pub(super) const DEFAULT_MANUAL_MAX_BYTES: usize = 16 * 1024;

const COMPACT_EXAMPLE_LIMIT: usize = 3;
const COMPACT_ERROR_CASE_LIMIT: usize = 3;
const COMPACT_FIELD_LIMIT: usize = 40;
const SHORT_DESCRIPTION_CHARS: usize = 220;

/// Caller-controlled manual rendering limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ManualLimits {
    pub max_bytes: usize,
    pub compact: bool,
}

impl Default for ManualLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MANUAL_MAX_BYTES,
            compact: true,
        }
    }
}

impl ManualLimits {
    pub(super) fn from_args(args: Option<&Value>) -> Self {
        let max_bytes = args
            .and_then(|value| value.get("max_bytes"))
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MANUAL_MAX_BYTES);
        let compact = args
            .and_then(|value| value.get("compact"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        Self { max_bytes, compact }
    }
}

/// Apply compact mode and the byte ceiling to a manual document in place.
///
/// Reductions are deterministic and ordered so the most valuable sections
/// (description, input args, contract, usage) survive longest.
pub(super) fn apply_manual_limits(document: &mut ToolManualDocument, limits: ManualLimits) {
    let mut payload_truncated = false;

    if limits.compact {
        if document.usage.few_shot_prompts.len() > COMPACT_EXAMPLE_LIMIT {
            document
                .usage
                .few_shot_prompts
                .truncate(COMPACT_EXAMPLE_LIMIT);
            payload_truncated = true;
        }
        if document.usage.target_tool_call_examples.len() > COMPACT_EXAMPLE_LIMIT {
            document
                .usage
                .target_tool_call_examples
                .truncate(COMPACT_EXAMPLE_LIMIT);
            payload_truncated = true;
        }
        if document.error_cases.len() > COMPACT_ERROR_CASE_LIMIT {
            document.error_cases.truncate(COMPACT_ERROR_CASE_LIMIT);
            payload_truncated = true;
        }
        if document.output_response.response_fields.len() > COMPACT_FIELD_LIMIT {
            document
                .output_response
                .response_fields
                .truncate(COMPACT_FIELD_LIMIT);
            payload_truncated = true;
        }
    }

    while serialized_len(document) > limits.max_bytes {
        let mut changed = false;
        changed |= halve(&mut document.output_response.response_fields);
        if changed {
            payload_truncated = true;
            continue;
        }
        changed |= trim_to(
            &mut document.output_response.metadata_fields,
            COMPACT_ERROR_CASE_LIMIT,
        );
        if changed {
            payload_truncated = true;
            continue;
        }
        changed |= trim_to(&mut document.usage.target_tool_call_examples, 1);
        if changed {
            payload_truncated = true;
            continue;
        }
        changed |= clear(&mut document.usage.tool_usage);
        if changed {
            payload_truncated = true;
            continue;
        }
        changed |= clear(&mut document.input_contract.notes);
        if changed {
            payload_truncated = true;
            continue;
        }
        changed |= clear(&mut document.output_response.error_payload_fields);
        if changed {
            payload_truncated = true;
            continue;
        }
        changed |= clear(&mut document.usage.few_shot_prompts);
        if changed {
            payload_truncated = true;
            continue;
        }
        // Last resort: shorten the long free-text fields.
        if document.description.chars().count() > SHORT_DESCRIPTION_CHARS {
            document.description = truncate_chars(&document.description, SHORT_DESCRIPTION_CHARS);
            document.truncation.description_truncated = true;
            payload_truncated = true;
            continue;
        }
        if document.tool_structure.purpose.chars().count() > SHORT_DESCRIPTION_CHARS {
            document.tool_structure.purpose =
                truncate_chars(&document.tool_structure.purpose, SHORT_DESCRIPTION_CHARS);
            payload_truncated = true;
            continue;
        }
        if document.output_response.response_shape.chars().count() > SHORT_DESCRIPTION_CHARS {
            document.output_response.response_shape = truncate_chars(
                &document.output_response.response_shape,
                SHORT_DESCRIPTION_CHARS,
            );
            payload_truncated = true;
            continue;
        }
        break;
    }

    document.truncation.payload_truncated |= payload_truncated;
}

fn serialized_len(document: &ToolManualDocument) -> usize {
    serde_json::to_vec(document)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn halve<T>(items: &mut Vec<T>) -> bool {
    if items.is_empty() {
        return false;
    }
    let keep = items.len() / 2;
    items.truncate(keep);
    true
}

fn trim_to<T>(items: &mut Vec<T>, keep: usize) -> bool {
    if items.len() <= keep {
        return false;
    }
    items.truncate(keep);
    true
}

fn clear<T>(items: &mut Vec<T>) -> bool {
    if items.is_empty() {
        return false;
    }
    items.clear();
    true
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out = text.chars().take(max).collect::<String>();
    out.push('…');
    out
}

/// Synthesize a useful field description when the schema omits one.
///
/// The fallback describes the declared JSON shape (and nested object fields)
/// so `man` output never repeats a placeholder line.
pub(super) fn schema_field_description(name: &str, field: &Value) -> String {
    if let Some(description) = field
        .get("description")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return truncate_chars(description, SHORT_DESCRIPTION_CHARS);
    }

    match field.get("type").and_then(Value::as_str) {
        Some("object") => {
            let fields = property_names(field);
            if fields.is_empty() {
                format!("Object payload returned for `{name}`.")
            } else {
                format!(
                    "Object payload for `{name}` with fields: {}.",
                    fields.join(", ")
                )
            }
        }
        Some("array") => {
            let items = field.get("items").unwrap_or(&Value::Null);
            match items.get("type").and_then(Value::as_str) {
                Some("object") => {
                    let fields = property_names(items);
                    if fields.is_empty() {
                        format!("Array of objects returned for `{name}`.")
                    } else {
                        format!(
                            "Array of objects for `{name}`; item fields: {}.",
                            fields.join(", ")
                        )
                    }
                }
                Some(kind) => format!("Array of {kind} values returned for `{name}`."),
                None => format!("Array returned for `{name}`."),
            }
        }
        Some("boolean") => format!("Boolean flag `{name}` reported by the tool."),
        Some("integer") => format!("Integer value `{name}` reported by the tool."),
        Some("number") => format!("Numeric value `{name}` reported by the tool."),
        Some("string") => format!("String value `{name}` reported by the tool."),
        Some("null") => format!("Nullable value `{name}`; normally null."),
        Some(kind) => format!("`{name}` value of type {kind} reported by the tool."),
        None => format!("`{name}` value reported by the tool."),
    }
}

fn property_names(schema: &Value) -> Vec<&str> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_field_description_uses_declared_description() {
        let field = json!({ "type": "string", "description": "Repository-relative path." });
        assert_eq!(
            schema_field_description("file", &field),
            "Repository-relative path."
        );
    }

    #[test]
    fn schema_field_description_synthesizes_object_fields() {
        let field = json!({
            "type": "object",
            "properties": { "summary": {}, "findings": {} }
        });
        let description = schema_field_description("report", &field);
        assert!(description.contains("summary"));
        assert!(description.contains("findings"));
    }

    #[test]
    fn schema_field_description_synthesizes_array_item_fields() {
        let field = json!({
            "type": "array",
            "items": { "type": "object", "properties": { "qn": {}, "kind": {} } }
        });
        let description = schema_field_description("matches", &field);
        assert!(description.contains("qn"));
        assert!(description.contains("kind"));
    }

    #[test]
    fn limits_default_to_compact_with_byte_ceiling() {
        let limits = ManualLimits::from_args(None);
        assert!(limits.compact);
        assert_eq!(limits.max_bytes, DEFAULT_MANUAL_MAX_BYTES);
    }

    #[test]
    fn limits_honor_caller_overrides() {
        let args = json!({ "compact": false, "max_bytes": 2048 });
        let limits = ManualLimits::from_args(Some(&args));
        assert!(!limits.compact);
        assert_eq!(limits.max_bytes, 2048);
    }
}
