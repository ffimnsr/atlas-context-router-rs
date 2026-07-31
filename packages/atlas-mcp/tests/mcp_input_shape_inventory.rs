use atlas_mcp::tool_list;
use serde_json::Value;

fn tool_properties(tool: &Value) -> &serde_json::Map<String, Value> {
    tool.pointer("/inputSchema/properties")
        .and_then(Value::as_object)
        .expect("inputSchema.properties object")
}

#[test]
fn mcp_input_shape_inventory_enforces_canonical_agent_shapes() {
    let list = tool_list();
    let tools = list["tools"].as_array().expect("tool_list tools array");

    for (tool_name, forbidden_fields) in [
        ("get_context", vec!["query", "file", "files"]),
        (
            "read_file_excerpt",
            vec![
                "line_ranges",
                "start_line",
                "end_line",
                "line",
                "before",
                "after",
            ],
        ),
        ("get_docs_section", vec!["heading", "line"]),
        (
            "get_impact_radius",
            vec!["mode", "files", "base", "staged", "working_tree"],
        ),
        (
            "get_review_context",
            vec!["mode", "files", "base", "staged", "working_tree"],
        ),
        (
            "get_minimal_context",
            vec!["mode", "files", "base", "staged", "working_tree"],
        ),
        (
            "explain_change",
            vec!["mode", "files", "base", "staged", "working_tree"],
        ),
        (
            "detect_changes",
            vec!["mode", "base", "staged", "working_tree"],
        ),
        ("batch_query_graph", vec!["queries", "text"]),
        (
            "build_or_update_graph",
            vec!["mode", "base", "staged", "files"],
        ),
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == tool_name)
            .unwrap_or_else(|| panic!("tool {tool_name} missing from tool_list"));
        let properties = tool_properties(tool);
        for field in forbidden_fields {
            assert!(
                !properties.contains_key(field),
                "tool {tool_name} must not expose removed legacy field {field}"
            );
        }
    }

    for tool_name in [
        "query_graph",
        "batch_query_graph",
        "get_impact_radius",
        "detect_changes",
        "read_saved_context",
        "save_context_artifact",
        "cross_session_search",
        "resolve_symbol",
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == tool_name)
            .unwrap_or_else(|| panic!("tool {tool_name} missing from tool_list"));
        let properties = tool_properties(tool);
        assert!(
            properties.contains_key("repo_scope"),
            "tool {tool_name} must expose canonical repo_scope object"
        );
        assert!(
            !properties.contains_key("repo_id"),
            "tool {tool_name} must not expose top-level repo_id"
        );
        assert!(
            !properties.contains_key("all_repos"),
            "tool {tool_name} must not expose top-level all_repos"
        );
    }
}

#[test]
fn tool_descriptions_avoid_hidden_precedence_phrases() {
    let list = tool_list();
    let offenders = list["tools"]
        .as_array()
        .expect("tool_list tools array")
        .iter()
        .flat_map(|tool| {
            let name = tool["name"].as_str().expect("tool name");
            collect_description_offenders(name, tool, String::new())
        })
        .collect::<Vec<_>>();

    assert!(
        offenders.is_empty(),
        "tool descriptions must not advertise hidden precedence phrases: {offenders:#?}"
    );
}

fn collect_description_offenders(name: &str, value: &Value, path: String) -> Vec<String> {
    match value {
        Value::Object(map) => map
            .iter()
            .flat_map(|(key, child)| {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if key == "description" {
                    let description = child.as_str().unwrap_or_default();
                    let lower = description.to_ascii_lowercase();
                    if lower.contains("if both are given")
                        || lower.contains("wins")
                        || lower.contains("takes precedence")
                        || lower.contains("ignored when both")
                    {
                        vec![format!("{name}:{child_path}: {description}")]
                    } else {
                        Vec::new()
                    }
                } else {
                    collect_description_offenders(name, child, child_path)
                }
            })
            .collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .flat_map(|(index, child)| {
                let child_path = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}[{index}]")
                };
                collect_description_offenders(name, child, child_path)
            })
            .collect(),
        _ => Vec::new(),
    }
}
