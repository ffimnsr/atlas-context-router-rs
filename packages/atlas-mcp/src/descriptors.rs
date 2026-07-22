use anyhow::{Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
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
    #[serde(rename = "outputSchema", skip_serializing_if = "Option::is_none")]
    pub(crate) output_schema: Option<Value>,
    pub(crate) annotations: ToolAnnotations,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceDescriptor {
    pub(crate) uri: String,
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) mime_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) icons: Vec<IconDescriptor>,
    #[serde(rename = "_meta")]
    pub(crate) meta: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceTemplateDescriptor {
    pub(crate) uri_template: String,
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) mime_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
#[serde(rename_all = "camelCase")]
pub(crate) struct IconDescriptor {
    pub(crate) src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sizes: Option<Vec<String>>,
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

pub(crate) fn descriptor_meta(descriptor_kind: &str, category: &str) -> Value {
    json!({
        "atlas:descriptorKind": descriptor_kind,
        "atlas:category": category,
        "atlas:generatedBy": "atlas-mcp",
        "atlas:schemaDraft": JSON_SCHEMA_2020_12_URI,
    })
}

fn normalized_success_metadata_properties() -> Map<String, Value> {
    Map::from_iter([
        ("tool".to_owned(), json!({ "type": "string" })),
        (
            "generated_at".to_owned(),
            json!({ "type": "string", "description": "RFC 3339 generation timestamp for this success payload." }),
        ),
        ("truncated".to_owned(), json!({ "type": "boolean" })),
        ("truncation_reason".to_owned(), json!({ "type": "string" })),
        (
            "warnings".to_owned(),
            json!({ "type": "array", "items": { "type": "string" } }),
        ),
        ("budget_status".to_owned(), json!({ "type": "string" })),
        ("budget_hit".to_owned(), json!({ "type": "boolean" })),
        ("budget_name".to_owned(), json!({ "type": "string" })),
        ("budget_limit".to_owned(), json!({ "type": "integer" })),
        ("budget_observed".to_owned(), json!({ "type": "integer" })),
        ("partial".to_owned(), json!({ "type": "boolean" })),
        ("safe_to_answer".to_owned(), json!({ "type": "boolean" })),
        ("atlas_provenance".to_owned(), json!({ "type": "object" })),
        ("atlas_freshness".to_owned(), json!({ "type": "object" })),
    ])
}

pub(crate) fn normalized_tool_output_schema(
    properties: Value,
    required: &[&str],
    defs: Option<Value>,
) -> Value {
    let mut all_properties = normalized_success_metadata_properties();
    if let Some(tool_properties) = properties.as_object() {
        for (key, value) in tool_properties {
            all_properties.insert(key.clone(), value.clone());
        }
    }

    let mut schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": Value::Object(all_properties),
        "required": required,
    });

    if let Some(defs) = defs
        && let Some(schema_object) = schema.as_object_mut()
    {
        schema_object.insert("$defs".to_owned(), defs);
    }

    ensure_schema_2020_12(schema)
}

#[cfg(test)]
mod tests {
    use super::{
        JSON_SCHEMA_2020_12_URI, ensure_schema_2020_12, human_title, normalized_tool_output_schema,
        validate_descriptor_name,
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

    #[test]
    fn normalized_tool_output_schema_includes_shared_metadata_fields() {
        let schema = normalized_tool_output_schema(
            json!({
                "summary": { "type": "object" },
                "items": { "type": "array", "items": { "type": "string" } }
            }),
            &["summary", "items"],
            Some(json!({
                "demo": { "type": "string" }
            })),
        );

        assert_eq!(schema["$schema"], json!(JSON_SCHEMA_2020_12_URI));
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["properties"]["tool"]["type"], json!("string"));
        assert_eq!(
            schema["properties"]["generated_at"]["type"],
            json!("string")
        );
        assert_eq!(schema["properties"]["warnings"]["type"], json!("array"));
        assert_eq!(schema["properties"]["summary"]["type"], json!("object"));
        assert_eq!(schema["required"], json!(["summary", "items"]));
        assert_eq!(schema["$defs"]["demo"]["type"], json!("string"));
    }
}
