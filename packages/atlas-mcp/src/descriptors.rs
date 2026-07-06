use anyhow::{Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub(crate) const JSON_SCHEMA_2020_12_URI: &str = "https://json-schema.org/draft/2020-12/schema";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolRegistry {
    pub(crate) tools: Vec<ToolDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolDescriptor {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    #[serde(rename = "inputSchema")]
    pub(crate) input_schema: Value,
    #[serde(rename = "outputSchema")]
    pub(crate) output_schema: Value,
    pub(crate) annotations: ToolAnnotations,
    pub(crate) icons: Vec<IconDescriptor>,
    #[serde(rename = "_meta")]
    pub(crate) meta: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptRegistry {
    pub(crate) prompts: Vec<PromptDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptDescriptor {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) arguments: Vec<PromptArgumentDescriptor>,
    pub(crate) icons: Vec<IconDescriptor>,
    #[serde(rename = "_meta")]
    pub(crate) meta: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptArgumentDescriptor {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) required: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResourceDescriptor {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) icons: Vec<IconDescriptor>,
    #[serde(rename = "_meta")]
    pub(crate) meta: Value,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResourceTemplateDescriptor {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) icons: Vec<IconDescriptor>,
    #[serde(rename = "_meta")]
    pub(crate) meta: Value,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CompletionDescriptor {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) icons: Vec<IconDescriptor>,
    #[serde(rename = "_meta")]
    pub(crate) meta: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolAnnotations {
    #[serde(rename = "readOnlyHint")]
    pub(crate) read_only_hint: bool,
    #[serde(rename = "stateChangingHint")]
    pub(crate) state_changing_hint: bool,
    #[serde(rename = "destructiveHint")]
    pub(crate) destructive_hint: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IconDescriptor {
    pub(crate) kind: String,
    pub(crate) label: String,
    pub(crate) value: String,
}

impl IconDescriptor {
    pub(crate) fn emoji(label: &str, value: &str) -> Self {
        Self {
            kind: "emoji".to_owned(),
            label: label.to_owned(),
            value: value.to_owned(),
        }
    }
}

pub(crate) fn validate_descriptor_name(name: &str) -> Result<()> {
    static DESCRIPTOR_NAME_RE: OnceLock<Regex> = OnceLock::new();
    let pattern = DESCRIPTOR_NAME_RE.get_or_init(|| {
        Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$").expect("valid descriptor regex")
    });
    if pattern.is_match(name) {
        Ok(())
    } else {
        bail!("descriptor name '{name}' violates MCP naming guidance")
    }
}

pub(crate) fn human_title(name: &str) -> String {
    name.split(['_', '-', '.'])
        .filter(|part| !part.is_empty())
        .map(title_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_word(word: &str) -> String {
    match word {
        "mcp" => "MCP".to_owned(),
        "rpc" => "RPC".to_owned(),
        "sql" => "SQL".to_owned(),
        "db" => "DB".to_owned(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_ascii_lowercase()
                }
                None => String::new(),
            }
        }
    }
}

pub(crate) fn ensure_schema_2020_12(mut schema: Value) -> Value {
    let Value::Object(ref mut object) = schema else {
        return schema;
    };
    object.insert(
        "$schema".to_owned(),
        Value::String(JSON_SCHEMA_2020_12_URI.to_owned()),
    );
    schema
}

pub(crate) fn tool_output_schema() -> Value {
    ensure_schema_2020_12(json!({
        "type": "object",
        "properties": {
            "content": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string" },
                        "text": { "type": "string" },
                        "mimeType": { "type": "string" }
                    },
                    "required": ["type", "text"]
                }
            },
            "atlas_output_format": { "type": "string" },
            "atlas_requested_output_format": { "type": "string" },
            "atlas_fallback_reason": { "type": "string" }
        },
        "required": ["content", "atlas_output_format", "atlas_requested_output_format"]
    }))
}

pub(crate) fn descriptor_meta(descriptor_kind: &str, category: &str) -> Value {
    json!({
        "atlas:descriptorKind": descriptor_kind,
        "atlas:category": category,
        "atlas:generatedBy": "atlas-mcp",
        "atlas:schemaDraft": JSON_SCHEMA_2020_12_URI,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        JSON_SCHEMA_2020_12_URI, ensure_schema_2020_12, human_title, validate_descriptor_name,
    };
    use serde_json::json;

    #[test]
    fn descriptor_name_validation_accepts_current_tool_style() {
        validate_descriptor_name("build_or_update_graph").expect("valid name");
        validate_descriptor_name("query.graph").expect("valid name");
    }

    #[test]
    fn descriptor_name_validation_rejects_invalid_names() {
        assert!(validate_descriptor_name("bad name").is_err());
        assert!(validate_descriptor_name("-leading-dash").is_err());
    }

    #[test]
    fn human_title_is_stable() {
        assert_eq!(
            human_title("build_or_update_graph"),
            "Build Or Update Graph"
        );
        assert_eq!(human_title("mcp.query_sql"), "MCP Query SQL");
    }

    #[test]
    fn schema_helper_injects_2020_12_draft_uri() {
        let schema = ensure_schema_2020_12(json!({"type": "object"}));
        assert_eq!(schema["$schema"], json!(JSON_SCHEMA_2020_12_URI));
    }
}
