use anyhow::{Result, bail};
use regex::Regex;
use rmcp::model::{
    Icon, Prompt, PromptArgument, Resource, ResourceTemplate, Tool, ToolAnnotations,
};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

pub(crate) const JSON_SCHEMA_2020_12_URI: &str = "https://json-schema.org/draft/2020-12/schema";

pub(crate) type ToolDescriptor = Tool;
pub(crate) type PromptDescriptor = Prompt;
pub(crate) type PromptArgumentDescriptor = PromptArgument;
pub(crate) type ResourceDescriptor = Resource;
pub(crate) type ResourceTemplateDescriptor = ResourceTemplate;
pub(crate) type IconDescriptor = Icon;
pub(crate) type ToolDescriptorAnnotations = ToolAnnotations;

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

pub(crate) fn validate_mcp_schema(schema: &Value) -> Result<()> {
    validate_mcp_schema_inner(schema, schema, "$")
}

fn validate_mcp_schema_inner(root: &Value, current: &Value, path: &str) -> Result<()> {
    match current {
        Value::Object(object) => {
            if object.contains_key("x-mcp-header") {
                bail!(
                    "schema path {path} uses unsupported x-mcp-header annotation; Atlas does not expose header-bound schema fields"
                );
            }
            if let Some(reference) = object.get("$ref") {
                let reference = reference
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("schema path {path} has non-string $ref"))?;
                if !reference.starts_with("#/") {
                    bail!(
                        "schema path {path} uses non-local $ref '{reference}'; only local #/... references are allowed"
                    );
                }
                let pointer = &reference[1..];
                if root.pointer(pointer).is_none() {
                    bail!("schema path {path} points to unresolved $ref '{reference}'");
                }
            }
            for (key, value) in object {
                let child_path = format!("{path}/{key}");
                validate_mcp_schema_inner(root, value, &child_path)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let child_path = format!("{path}/{index}");
                validate_mcp_schema_inner(root, item, &child_path)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
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
        validate_descriptor_name, validate_mcp_schema,
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
        validate_mcp_schema(&schema).expect("normalized output schema valid for MCP");
    }

    #[test]
    fn mcp_schema_validation_accepts_local_resolvable_refs() {
        let schema = ensure_schema_2020_12(json!({
            "type": "object",
            "properties": {
                "node": { "$ref": "#/$defs/node" }
            },
            "$defs": {
                "node": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    }
                }
            }
        }));
        validate_mcp_schema(&schema).expect("local refs allowed");
    }

    #[test]
    fn mcp_schema_validation_rejects_external_refs() {
        let schema = ensure_schema_2020_12(json!({
            "type": "object",
            "properties": {
                "node": { "$ref": "https://example.com/node.schema.json" }
            }
        }));
        let error = validate_mcp_schema(&schema).expect_err("external refs rejected");
        assert!(
            error
                .to_string()
                .contains("only local #/... references are allowed")
        );
    }

    #[test]
    fn mcp_schema_validation_rejects_x_mcp_header_annotations() {
        let schema = ensure_schema_2020_12(json!({
            "type": "object",
            "properties": {
                "authorization": {
                    "type": "string",
                    "x-mcp-header": "Authorization"
                }
            }
        }));
        let error = validate_mcp_schema(&schema).expect_err("x-mcp-header rejected");
        assert!(
            error
                .to_string()
                .contains("unsupported x-mcp-header annotation")
        );
    }
}
