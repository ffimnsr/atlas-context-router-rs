use super::examples::json_literal;
use super::*;

pub(super) fn schema_has_property(schema: &Value, field_name: &str) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(field_name))
}

fn input_variant(
    value: &str,
    required_companion_fields: &[&str],
    minimal_example: Value,
) -> ToolManualInputVariant {
    ToolManualInputVariant {
        value: value.to_owned(),
        required_companion_fields: required_companion_fields
            .iter()
            .map(|field| (*field).to_owned())
            .collect(),
        minimal_example,
    }
}

pub(super) fn target_input_family() -> ToolManualInputFamily {
    let variants = vec![
        input_variant(
            "query",
            &["target.query"],
            json!({
                "target": { "kind": "query", "query": "compute" }
            }),
        ),
        input_variant(
            "file",
            &["target.file"],
            json!({
                "target": { "kind": "file", "file": "src/lib.rs" }
            }),
        ),
        input_variant(
            "files",
            &["target.files"],
            json!({
                "target": { "kind": "files", "files": ["src/lib.rs"] }
            }),
        ),
    ];
    ToolManualInputFamily {
        family_name: "target".to_owned(),
        family_kind: "discriminated_object".to_owned(),
        discriminant_field: Some("target.kind".to_owned()),
        accepted_values: variants
            .iter()
            .map(|variant| variant.value.clone())
            .collect(),
        mutually_exclusive_legacy_fields: Vec::new(),
        variants,
    }
}

pub(super) fn excerpt_selector_input_family() -> ToolManualInputFamily {
    let variants = vec![
        input_variant(
            "range",
            &["selector.start_line", "selector.end_line"],
            json!({
                "file": "src/lib.rs",
                "selector": { "kind": "range", "start_line": 10, "end_line": 20 }
            }),
        ),
        input_variant(
            "ranges",
            &["selector.line_ranges"],
            json!({
                "file": "src/lib.rs",
                "selector": {
                    "kind": "ranges",
                    "line_ranges": [{ "start_line": 10, "end_line": 20 }]
                }
            }),
        ),
        input_variant(
            "context",
            &["selector.line"],
            json!({
                "file": "src/lib.rs",
                "selector": { "kind": "context", "line": 42, "before": 2, "after": 2 }
            }),
        ),
    ];
    ToolManualInputFamily {
        family_name: "selector".to_owned(),
        family_kind: "discriminated_object".to_owned(),
        discriminant_field: Some("selector.kind".to_owned()),
        accepted_values: variants
            .iter()
            .map(|variant| variant.value.clone())
            .collect(),
        mutually_exclusive_legacy_fields: Vec::new(),
        variants,
    }
}

pub(super) fn docs_selector_input_family() -> ToolManualInputFamily {
    let variants = vec![
        input_variant(
            "heading",
            &["selector.heading"],
            json!({
                "file": "README.md",
                "selector": { "kind": "heading", "heading": "document.install" }
            }),
        ),
        input_variant(
            "line",
            &["selector.line"],
            json!({
                "file": "README.md",
                "selector": { "kind": "line", "line": 42 }
            }),
        ),
    ];
    ToolManualInputFamily {
        family_name: "selector".to_owned(),
        family_kind: "discriminated_object".to_owned(),
        discriminant_field: Some("selector.kind".to_owned()),
        accepted_values: variants
            .iter()
            .map(|variant| variant.value.clone())
            .collect(),
        mutually_exclusive_legacy_fields: Vec::new(),
        variants,
    }
}

pub(super) fn change_source_input_family(include_files: bool) -> ToolManualInputFamily {
    let mut variants = Vec::new();
    if include_files {
        variants.push(input_variant(
            "files",
            &["change_source.files"],
            json!({
                "change_source": { "kind": "files", "files": ["src/lib.rs"] }
            }),
        ));
    }
    variants.push(input_variant(
        "base",
        &["change_source.base"],
        json!({
            "change_source": { "kind": "base", "base": "origin/main" }
        }),
    ));
    variants.push(input_variant(
        "staged",
        &[],
        json!({
            "change_source": { "kind": "staged" }
        }),
    ));
    variants.push(input_variant(
        "working_tree",
        &[],
        json!({
            "change_source": { "kind": "working_tree" }
        }),
    ));
    ToolManualInputFamily {
        family_name: "change_source".to_owned(),
        family_kind: "discriminated_object".to_owned(),
        discriminant_field: Some("change_source.kind".to_owned()),
        accepted_values: variants
            .iter()
            .map(|variant| variant.value.clone())
            .collect(),
        mutually_exclusive_legacy_fields: Vec::new(),
        variants,
    }
}

pub(super) fn batch_query_items_input_family() -> ToolManualInputFamily {
    let variants = vec![input_variant(
        "items",
        &["items"],
        json!({
            "items": [{ "text": "compute" }]
        }),
    )];
    ToolManualInputFamily {
        family_name: "query_collection".to_owned(),
        family_kind: "preferred_field".to_owned(),
        discriminant_field: Some("items".to_owned()),
        accepted_values: vec!["items".to_owned()],
        mutually_exclusive_legacy_fields: Vec::new(),
        variants,
    }
}

pub(super) fn repo_scope_input_family() -> ToolManualInputFamily {
    let variants = vec![
        input_variant(
            "current",
            &[],
            json!({
                "repo_scope": { "kind": "current" }
            }),
        ),
        input_variant(
            "repo_id",
            &["repo_scope.repo_id"],
            json!({
                "repo_scope": { "kind": "repo_id", "repo_id": "primary" }
            }),
        ),
        input_variant(
            "all",
            &[],
            json!({
                "repo_scope": { "kind": "all" }
            }),
        ),
    ];
    ToolManualInputFamily {
        family_name: "repo_scope".to_owned(),
        family_kind: "discriminated_object".to_owned(),
        discriminant_field: Some("repo_scope.kind".to_owned()),
        accepted_values: variants
            .iter()
            .map(|variant| variant.value.clone())
            .collect(),
        mutually_exclusive_legacy_fields: Vec::new(),
        variants,
    }
}

pub(super) fn schema_fields(schema: &Value) -> Vec<ToolManualField> {
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();

    let mut names = props.keys().cloned().collect::<Vec<_>>();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            props
                .get(&name)
                .map(|field| schema_field(&name, field, &required))
        })
        .collect()
}

fn schema_field(
    name: &str,
    field: &Value,
    required: &std::collections::BTreeSet<String>,
) -> ToolManualField {
    ToolManualField {
        name: name.to_owned(),
        field_type: schema_type(field),
        required: required.contains(name),
        default_value: explicit_default_value(name, field),
        enum_values: field
            .get("enum")
            .and_then(Value::as_array)
            .map(|values| values.iter().map(json_literal).collect())
            .unwrap_or_default(),
        description: truncate_text(
            &schema_field_description(name, field),
            MAX_DESCRIPTION_CHARS,
        ),
    }
}

fn schema_type(field: &Value) -> String {
    match field.get("type") {
        Some(Value::String(kind)) => {
            if kind == "array" {
                if let Some(items) = field.get("items") {
                    format!("array<{}>", nested_schema_type(items))
                } else {
                    "array".to_owned()
                }
            } else {
                kind.clone()
            }
        }
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | "),
        _ => nested_schema_type(field),
    }
}

fn nested_schema_type(field: &Value) -> String {
    match field.get("type") {
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" | "),
        _ if field.get("properties").is_some() => "object".to_owned(),
        _ if field.get("enum").is_some() => "enum".to_owned(),
        _ => "unknown".to_owned(),
    }
}

fn explicit_default_value(name: &str, field: &Value) -> Option<String> {
    if let Some(default_value) = field.get("default") {
        return Some(json_literal(default_value));
    }

    let description = field.get("description").and_then(Value::as_str)?;
    parse_default_from_description(name, description)
}

fn parse_default_from_description(_name: &str, description: &str) -> Option<String> {
    let lower = description.to_ascii_lowercase();
    let marker = "default ";
    let start = lower.find(marker)? + marker.len();
    let tail = &description[start..];
    let trimmed = tail
        .trim_start_matches([':', ' '])
        .split([')', '.', ';', ','])
        .next()
        .unwrap_or_default()
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.trim_matches('`').trim_matches('\'').to_owned())
}

pub(super) fn top_level_shape(label: &str, schema: &Value) -> String {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(Map::len)
        .unwrap_or(0);
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    format!("{label}: object with {properties} top-level fields ({required} required)")
}
