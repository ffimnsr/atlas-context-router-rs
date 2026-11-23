use super::*;

pub(super) fn target_tool_examples(tool_name: &str, input_schema: &Value) -> Vec<String> {
    if let Some(examples) = special_target_tool_examples(tool_name) {
        return examples;
    }

    let props = input_schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut args = Map::new();
    for field_name in required {
        if let Some(field) = props.get(field_name) {
            args.insert(field_name.to_owned(), example_value(field_name, field));
        }
    }
    if args.is_empty()
        && let Some(primary) = first_example_field(&props)
        && let Some(field) = props.get(primary)
    {
        args.insert(primary.to_owned(), example_value(primary, field));
    }

    let examples = vec![truncate_text(
        &json!({
            "name": tool_name,
            "arguments": Value::Object(args),
        })
        .to_string(),
        MAX_EXAMPLE_CHARS,
    )];
    examples
}

fn first_example_field(props: &Map<String, Value>) -> Option<&str> {
    [
        "text",
        "query",
        "name",
        "qname",
        "symbol",
        "tool_name",
        "file",
        "files",
    ]
    .into_iter()
    .find(|field| props.contains_key(*field))
}

fn example_value(field_name: &str, field: &Value) -> Value {
    if let Some(default_value) = field.get("default") {
        return default_value.clone();
    }
    if let Some(values) = field.get("enum").and_then(Value::as_array)
        && let Some(first) = values.first()
    {
        return first.clone();
    }
    match field.get("type") {
        Some(Value::String(kind)) => match kind.as_str() {
            "boolean" => Value::Bool(true),
            "integer" => Value::from(1),
            "number" => Value::from(1),
            "array" => Value::Array(Vec::new()),
            "string" if field_name == "output_format" => Value::String("json".to_owned()),
            "string" => Value::String(format!("<{field_name}>")),
            _ => Value::String(format!("<{field_name}>")),
        },
        _ => Value::String(format!("<{field_name}>")),
    }
}

pub(crate) fn suggest_tool_names(input: &str) -> Vec<String> {
    let needle = input.to_ascii_lowercase();
    let mut scored = tool_descriptors()
        .into_iter()
        .map(|tool| {
            let hay = tool.name.to_ascii_lowercase();
            let starts_with = hay.starts_with(&needle);
            let contains = hay.contains(&needle);
            let distance = strsim::levenshtein(&needle, &hay);
            (starts_with, contains, distance, tool.name)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .reverse()
            .then(left.1.cmp(&right.1).reverse())
            .then(left.2.cmp(&right.2))
            .then(left.3.len().cmp(&right.3.len()))
            .then(left.3.cmp(&right.3))
    });
    scored
        .into_iter()
        .filter(|(starts_with, contains, distance, _)| *starts_with || *contains || *distance <= 6)
        .take(MAX_SUGGESTIONS)
        .map(|(_, _, _, name)| name.to_string())
        .collect()
}

pub(super) fn tool_result_contract_label(tool_name: &str) -> &'static str {
    match tool_result_contract(tool_name) {
        super::registry::ToolResultContract::StableObject => "stable-object",
        super::registry::ToolResultContract::TextOnly => "text-only",
    }
}

pub(super) struct TruncatedText {
    pub(super) value: String,
    pub(super) truncated: bool,
}

pub(super) fn truncate_with_flag(value: &str, max_chars: usize) -> TruncatedText {
    let truncated = value.chars().count() > max_chars;
    let value = truncate_text(value, max_chars);
    TruncatedText { value, truncated }
}

pub(crate) fn truncate_text(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    let mut truncated = chars.into_iter().take(keep).collect::<String>();
    truncated.push('…');
    truncated
}

pub(super) fn json_literal(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_lookup_uses_exact_visible_name() {
        let doc = tool_manual("mcp", "query_graph").expect("manual doc");
        assert_eq!(doc.name, "query_graph");
        assert_eq!(doc.resolved_tool_name, "query_graph");
        assert_eq!(doc.requested_tool_name, "query_graph");
        assert!(
            doc.input_args
                .iter()
                .any(|field| field.name == "text" && !field.required)
        );
    }

    #[test]
    fn manual_lookup_rejects_unknown_namespace() {
        let error = tool_manual("sql", "query_graph").expect_err("must fail");
        assert!(error.to_string().contains("only 'mcp' is supported"));
    }

    #[test]
    fn man_namespace_rejection_is_structured_invalid_input() {
        let response = tool_man(
            Some(&json!({ "namespace": "cli", "tool_name": "query_graph" })),
            OutputFormat::Json,
        )
        .expect("man response");
        assert_eq!(
            response["structuredContent"]["code"],
            json!("invalid_input")
        );
        assert_eq!(
            response["structuredContent"]["message"],
            json!("unsupported manual namespace 'cli'; only 'mcp' is supported")
        );
        assert_eq!(
            response["structuredContent"]["details"]["accepted_namespaces"],
            json!(["mcp"])
        );
    }

    #[test]
    fn manual_lookup_suggests_nearest_visible_tools() {
        let result = lookup_tool_manual("mcp", "query_grap").expect_err("must fail");
        let ManualLookupError::UnknownTool { suggestions, .. } = result else {
            panic!("expected unknown tool error");
        };
        assert_eq!(suggestions.first().map(String::as_str), Some("query_graph"));
    }

    #[test]
    fn manual_lookup_suggests_edit_distance_one_names() {
        let result = lookup_tool_manual("mcp", "traverse_grah").expect_err("must fail");
        let ManualLookupError::UnknownTool { suggestions, .. } = result else {
            panic!("expected unknown tool error");
        };
        assert_eq!(
            suggestions.first().map(String::as_str),
            Some("traverse_graph")
        );
    }

    #[test]
    fn manual_lookup_hides_internal_tools() {
        let result = lookup_tool_manual("mcp", "__test_sleep").expect_err("must fail");
        assert!(matches!(
            result,
            ManualLookupError::HiddenInternalTool { .. }
        ));
    }

    #[test]
    fn text_renderer_includes_required_sections() {
        let doc = tool_manual("mcp", "query_graph").expect("manual doc");
        let text = render_tool_manual_text(&doc);
        for section in [
            "structure:",
            "input_args:",
            "input_contract:",
            "output_response:",
            "usage:",
            "error_cases:",
        ] {
            assert!(text.contains(section), "missing section {section}");
        }
        assert!(text.contains("man mcp query_graph"));
        assert!(text.contains("tool_usage:"));
        assert!(text.contains("few_shot_prompts:"));
    }

    #[test]
    fn graph_tool_manual_examples_use_direct_agent_shapes() {
        let build = tool_manual("mcp", "build_graph").expect("build graph manual");
        assert_eq!(
            build.usage.target_tool_call_examples,
            vec![r#"{"name":"build_graph","arguments":{}}"#.to_owned()]
        );
        assert!(
            build
                .usage
                .few_shot_prompts
                .iter()
                .any(|prompt| prompt.contains("`build_graph` with `{}`"))
        );

        let update = tool_manual("mcp", "update_graph").expect("update graph manual");
        assert!(
            update
                .usage
                .target_tool_call_examples
                .iter()
                .any(|example| example.contains(r#""change_source":{"kind":"working_tree"}"#))
        );
        assert!(
            update
                .usage
                .few_shot_prompts
                .iter()
                .any(|prompt| prompt.contains(
                    "`update_graph` with `{ \"change_source\": { \"kind\": \"working_tree\" } }`"
                ))
        );
    }

    #[test]
    fn diagnostic_and_inventory_tools_use_concrete_manual_examples() {
        let trace = tool_manual("mcp", "trace_calls").expect("trace_calls manual");
        assert!(
            trace
                .usage
                .target_tool_call_examples
                .iter()
                .any(|example| example.contains(r#""direction":"up""#))
        );
        assert!(
            trace
                .usage
                .target_tool_call_examples
                .iter()
                .any(|example| example.contains(r#""to":"reconcile_call_targets""#))
        );

        let embed = tool_manual("mcp", "embed_chunks").expect("embed_chunks manual");
        assert!(
            embed
                .usage
                .target_tool_call_examples
                .iter()
                .any(|example| example.contains(r#""dry_run":true"#))
        );

        let explain = tool_manual("mcp", "explain_query").expect("explain_query manual");
        assert!(
            explain
                .usage
                .target_tool_call_examples
                .iter()
                .any(|example| example.contains(r#""regex":"handle|HANDLE|Handle_""#))
        );

        let list = tool_manual("mcp", "list_symbols").expect("list_symbols manual");
        assert!(
            list.usage
                .target_tool_call_examples
                .iter()
                .any(|example| example.contains(r#""subpath":"src/auth""#))
        );

        for doc in [&trace, &embed, &explain, &list] {
            assert!(
                !doc.usage
                    .target_tool_call_examples
                    .iter()
                    .any(|example| example.contains('<') || example.contains('>')),
                "{} keeps placeholder examples",
                doc.name
            );
        }
    }

    #[test]
    fn every_visible_tool_manual_includes_usage_examples_and_few_shots() {
        for tool in tool_descriptors() {
            let doc = tool_manual("mcp", &tool.name).expect("manual doc");
            assert!(
                !doc.usage.tool_usage.is_empty(),
                "{} missing tool usage guidance",
                tool.name
            );
            assert!(
                !doc.usage.target_tool_call_examples.is_empty(),
                "{} missing target call examples",
                tool.name
            );
            assert!(
                !doc.usage.few_shot_prompts.is_empty(),
                "{} missing few-shot prompts",
                tool.name
            );
        }
    }

    #[test]
    fn memory_session_and_overview_tools_have_tailored_guidance() {
        for name in [
            "memory_store",
            "memory_recall",
            "memory_ops",
            "feedback_record",
            "memoir_create",
            "memoir_add_concept",
            "memoir_link",
            "memoir_inspect",
            "memoir_delete",
            "experience_inspect",
            "save_context_artifact",
            "search_saved_context",
            "read_saved_context",
            "purge_saved_context",
            "get_session_status",
            "resume_session",
            "wake_up",
            "record_session_event",
            "compact_session",
            "overview_show",
            "overview_search",
            "overview_export",
            "overview_ingest",
            "overview_refresh_plan",
            "get_global_memory",
            "build_graph",
            "update_graph",
            "postprocess_graph",
            "get_metrics",
        ] {
            let doc = tool_manual("mcp", name).expect("manual doc");
            assert!(
                !doc.usage
                    .tool_usage
                    .iter()
                    .any(|guidance| guidance
                        .starts_with("Use when request matches tool description")),
                "{name} falls back to generic usage guidance"
            );
        }
    }

    #[test]
    fn state_changing_tools_expose_warning_and_annotations() {
        for name in [
            "build_graph",
            "update_graph",
            "postprocess_graph",
            "compact_session",
            "record_session_event",
            "wake_up",
            "purge_saved_context",
            "save_context_artifact",
            "memory_store",
            "feedback_record",
            "memoir_create",
            "memoir_add_concept",
            "memoir_link",
            "overview_ingest",
            "overview_refresh_plan",
        ] {
            let doc = tool_manual("mcp", name).expect("manual doc");
            assert!(
                doc.tool_structure.annotations.state_changing,
                "{name} must be state-changing"
            );
            assert!(
                !doc.tool_structure.annotations.read_only,
                "{name} must not be read-only"
            );
            assert!(
                doc.usage
                    .tool_usage
                    .iter()
                    .any(|guidance| guidance.starts_with("State-changing tool:")),
                "{name} missing state-changing warning"
            );
        }
    }

    #[test]
    fn manual_lookup_exposes_target_contract_for_get_context() {
        let doc = tool_manual("mcp", "get_context").expect("manual doc");
        let family = doc
            .input_contract
            .families
            .iter()
            .find(|family| family.family_name == "target")
            .expect("target family");
        assert_eq!(family.discriminant_field.as_deref(), Some("target.kind"));
        assert_eq!(family.accepted_values, vec!["query", "file", "files"]);
        assert!(family.mutually_exclusive_legacy_fields.is_empty());
    }

    #[test]
    fn manual_lookup_exposes_change_source_contract_for_update_graph() {
        let doc = tool_manual("mcp", "update_graph").expect("manual doc");
        assert!(
            doc.input_contract
                .families
                .iter()
                .any(|family| family.discriminant_field.as_deref() == Some("change_source.kind"))
        );
    }
}
