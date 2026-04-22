use super::*;

#[test]
fn tool_list_includes_explain_change() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    assert!(
        tools
            .iter()
            .any(|t| t.get("name") == Some(&"explain_change".into()))
    );
}

#[test]
fn tool_list_includes_get_context() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    assert!(
        tools
            .iter()
            .any(|t| t.get("name") == Some(&"get_context".into())),
        "tools/list must include get_context"
    );
}

#[test]
fn unknown_tool_returns_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let result = call("unknown_tool_xyz", None, "/ignored", &db_path);
    assert!(result.is_err(), "unknown tool must return an error");
    assert!(result.unwrap_err().to_string().contains("unknown tool"));
}

#[test]
fn tool_list_schema_has_required_fields() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("<missing>");
        assert!(
            tool.get("description").is_some(),
            "tool {name} must have description"
        );
        assert!(
            tool.pointer("/inputSchema/type").is_some(),
            "tool {name} must have inputSchema.type"
        );
    }
}

#[test]
fn tool_list_documents_output_format() {
    let list = tool_list();
    let tools = list
        .get("tools")
        .and_then(|value| value.as_array())
        .unwrap();

    for tool in tools {
        let props = tool
            .pointer("/inputSchema/properties")
            .and_then(|value| value.as_object())
            .expect("inputSchema properties");
        assert!(
            props.contains_key("output_format"),
            "tool must document output_format"
        );
    }
}

#[test]
fn tool_list_all_tools_default_to_toon() {
    let list = tool_list();
    let tools = list
        .get("tools")
        .and_then(|value| value.as_array())
        .expect("tools array");

    for tool in tools {
        let description = tool
            .pointer("/inputSchema/properties/output_format/description")
            .and_then(|value| value.as_str())
            .expect("output_format description");
        assert_eq!(description, DEFAULT_OUTPUT_DESCRIPTION);
    }
}

#[test]
fn tool_result_value_falls_back_to_json_when_toon_is_empty() {
    let rendered =
        tool_result_value(&serde_json::json!({}), OutputFormat::Toon).expect("tool result");

    assert_eq!(unwrap_tool_format(&rendered), "json");
    assert!(rendered.get("atlas_fallback_reason").is_some());
}

#[test]
fn invalid_output_format_returns_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let args = serde_json::json!({ "query": "compute", "output_format": "xml" });
    let result = call("get_context", Some(&args), "/ignored", &db_path);

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("unsupported output_format")
    );
}

#[test]
fn tool_list_includes_analysis_tools() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    for name in &[
        "analyze_safety",
        "analyze_remove",
        "analyze_dead_code",
        "analyze_dependency",
    ] {
        assert!(
            tools
                .iter()
                .any(|t| t.get("name") == Some(&serde_json::Value::String((*name).to_owned()))),
            "tools/list must include {name}"
        );
    }
}
