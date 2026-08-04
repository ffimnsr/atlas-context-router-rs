//! Registry-parity tests: discover/list/get_prompt/read_resource/complete,
//! subscription filtering, and bootstrap notification planning.

use super::*;
use crate::prompts;
use crate::resources;
use crate::spec;
use crate::tools;
use rmcp::ServerHandler;
use rmcp::model::{
    ArgumentInfo, CompleteRequestParams, ErrorCode, GetPromptRequestParams, Reference,
    SubscriptionFilter,
};

#[test]
fn get_info_matches_current_spec_server_info() {
    let server = server();
    let info = server.get_info();
    let expected = spec::server_info();
    assert_eq!(info.protocol_version.as_str(), spec::MCP_PROTOCOL_VERSION);
    assert_eq!(info.server_info.name, expected.name);
    assert_eq!(info.server_info.version, expected.version);
    assert_eq!(
        info.server_info.description.as_deref(),
        Some(expected.description.as_str())
    );
    assert!(info.capabilities.tools.is_some());
    assert!(info.capabilities.completions.is_some());
    assert!(info.capabilities.extensions.is_some());
    assert_eq!(
        info.capabilities
            .prompts
            .as_ref()
            .and_then(|capability| capability.list_changed),
        Some(true)
    );
    assert_eq!(
        info.capabilities
            .resources
            .as_ref()
            .and_then(|capability| capability.subscribe),
        Some(true)
    );
    assert_eq!(
        info.capabilities
            .resources
            .as_ref()
            .and_then(|capability| capability.list_changed),
        Some(true)
    );
    assert!(info.capabilities.supports_tasks());
}

#[test]
fn discover_uses_official_result_and_current_cache_policy() {
    let discover = server().discover_result();
    assert_eq!(discover.supported_versions.len(), 1);
    assert_eq!(
        discover.supported_versions[0].as_str(),
        spec::MCP_PROTOCOL_VERSION
    );
    assert_eq!(
        discover.instructions.as_deref(),
        Some(spec::DISCOVER_INSTRUCTIONS)
    );
    assert_eq!(discover.ttl_ms, spec::DISCOVER_CACHE_TTL_MS);
    assert_eq!(
        discover.server_info().expect("server info").name,
        spec::server_info().name
    );
}

#[test]
fn list_tools_names_match_current_tool_registry() {
    let actual = server()
        .list_tools_result()
        .expect("tools")
        .tools
        .into_iter()
        .map(|tool| tool.name.into_owned())
        .collect::<Vec<_>>();
    let expected = json_array_strings(&tools::tool_list()["tools"], "name");
    assert_eq!(actual, expected);
}

#[test]
fn list_prompts_names_match_current_prompt_registry() {
    let actual = server()
        .list_prompts_result()
        .expect("prompts")
        .prompts
        .into_iter()
        .map(|prompt| prompt.name)
        .collect::<Vec<_>>();
    let expected = prompts::prompt_descriptors()
        .into_iter()
        .map(|prompt| prompt.name)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn list_resources_uris_match_current_resource_registry() {
    let actual = server()
        .list_resources_result(None)
        .expect("resources")
        .resources
        .into_iter()
        .map(|resource| resource.uri)
        .collect::<Vec<_>>();
    let current = resources::resources_list(None).expect("resource list");
    let expected = json_array_strings(&current["resources"], "uri");
    assert_eq!(actual, expected);
}

#[test]
fn list_resource_templates_uri_templates_match_current_registry() {
    let actual = server()
        .list_resource_templates_result(None)
        .expect("resource templates")
        .resource_templates
        .into_iter()
        .map(|template| template.uri_template)
        .collect::<Vec<_>>();
    let current = resources::resources_templates_list(None).expect("template list");
    let expected = json_array_strings(&current["resourceTemplates"], "uriTemplate");
    assert_eq!(actual, expected);
}

#[test]
fn get_prompt_names_match_current_prompt_registry() {
    let fixture = ToolFixture::new();
    let actual = fixture
        .server
        .list_prompts_result()
        .expect("prompts")
        .prompts
        .into_iter()
        .map(|prompt| prompt.name)
        .collect::<Vec<_>>();
    let expected = prompts::prompt_descriptors()
        .into_iter()
        .map(|prompt| prompt.name)
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn get_prompt_requires_required_arguments() {
    let fixture = ToolFixture::new();

    let inspect_error = fixture
        .server
        .get_prompt_result("inspect_symbol", Some(&serde_json::Map::new()))
        .expect_err("inspect_symbol missing symbol must fail");
    assert_eq!(inspect_error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        inspect_error
            .message
            .as_ref()
            .contains("missing required argument: symbol")
    );

    let plan_error = fixture
        .server
        .get_prompt_result("plan_refactor", Some(&serde_json::Map::new()))
        .expect_err("plan_refactor missing target must fail");
    assert_eq!(plan_error.code, ErrorCode::INVALID_PARAMS);
    assert!(
        plan_error
            .message
            .as_ref()
            .contains("missing required argument: target")
    );
}

#[test]
fn get_prompt_text_matches_current_prompt_renderer() {
    let fixture = ToolFixture::new();
    let request = GetPromptRequestParams::new("inspect_symbol").with_arguments(
        serde_json::from_value(json!({
            "symbol": "src/lib.rs::fn::greet",
            "question": "What depends on this?"
        }))
        .expect("prompt args"),
    );
    let current = prompts::prompt_get(
        "inspect_symbol",
        Some(&json!({
            "symbol": "src/lib.rs::fn::greet",
            "question": "What depends on this?"
        })),
    )
    .expect("handrolled prompt");
    let rmcp = fixture
        .server
        .get_prompt_result(&request.name, request.arguments.as_ref())
        .expect("rmcp prompt");

    assert_eq!(
        rmcp.description.as_deref(),
        current.get("description").and_then(Value::as_str)
    );
    assert_eq!(
        rmcp.messages
            .first()
            .and_then(|message| message.content.as_text())
            .map(|text| text.text.as_str()),
        current
            .pointer("/messages/0/content/text")
            .and_then(Value::as_str)
    );
}

#[test]
fn read_resource_docs_index_matches_current_resource_renderer() {
    let fixture = ToolFixture::new();
    assert_read_resource_matches_handrolled(&fixture, "atlas://docs/index");
}

#[test]
fn read_resource_health_status_matches_current_resource_renderer() {
    let fixture = ToolFixture::new();
    assert_read_resource_matches_handrolled(&fixture, "atlas://health/status");
}

#[test]
fn read_resource_graph_provenance_matches_current_resource_renderer() {
    let fixture = ToolFixture::new();
    assert_read_resource_matches_handrolled(&fixture, "atlas://graph/provenance");
}

#[test]
fn read_resource_saved_context_matches_current_resource_renderer() {
    let fixture = ToolFixture::new();
    crate::tools::call(
        "save_context_artifact",
        Some(&json!({
            "content": "saved context fixture body",
            "kind": "note",
            "source_id": "src-completion-123",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("save context artifact");
    assert_read_resource_matches_handrolled(&fixture, "atlas://saved-context/src-completion-123");
}

#[test]
fn read_resource_tool_docs_matches_current_resource_renderer() {
    let fixture = ToolFixture::new();
    assert_read_resource_matches_handrolled(&fixture, "atlas://tool-docs/query_graph");
}

#[test]
fn read_resource_prompt_docs_matches_current_resource_renderer() {
    let fixture = ToolFixture::new();
    assert_read_resource_matches_handrolled(&fixture, "atlas://prompt-docs/review_change");
}

#[test]
fn read_resource_docs_section_matches_current_resource_renderer() {
    let fixture = ToolFixture::new();
    assert_read_resource_matches_handrolled(&fixture, "atlas://docs/README.md#document.status");
}

#[test]
fn list_resources_preserves_cursor_pagination_behavior() {
    let fixture = ToolFixture::new();
    let actual = fixture
        .server
        .list_resources_result(Some(
            serde_json::from_value(json!({ "cursor": "offset:1" })).expect("paginated params"),
        ))
        .expect("rmcp resources list");
    let expected = resources::resources_list(Some(&json!({ "cursor": "offset:1" })))
        .expect("current resources list");
    assert_eq!(
        actual
            .resources
            .iter()
            .map(|resource| resource.uri.clone())
            .collect::<Vec<_>>(),
        json_array_strings(&expected["resources"], "uri")
    );
    assert_eq!(
        actual.next_cursor.as_deref(),
        expected.get("nextCursor").and_then(Value::as_str)
    );
}

#[test]
fn list_resource_templates_preserves_cursor_pagination_behavior() {
    let fixture = ToolFixture::new();
    let actual = fixture
        .server
        .list_resource_templates_result(Some(
            serde_json::from_value(json!({ "cursor": "offset:1" })).expect("paginated params"),
        ))
        .expect("rmcp resource templates list");
    let expected = resources::resources_templates_list(Some(&json!({ "cursor": "offset:1" })))
        .expect("current resource templates list");
    assert_eq!(
        actual
            .resource_templates
            .iter()
            .map(|template| template.uri_template.clone())
            .collect::<Vec<_>>(),
        json_array_strings(&expected["resourceTemplates"], "uriTemplate")
    );
    assert_eq!(
        actual.next_cursor.as_deref(),
        expected.get("nextCursor").and_then(Value::as_str)
    );
}

#[test]
fn complete_tool_name_matches_current_completion() {
    let fixture = ToolFixture::new();
    assert_completion_matches_handrolled(
        &fixture,
        CompleteRequestParams::new(
            Reference::for_prompt("tools/call"),
            ArgumentInfo::new("name", "get_"),
        ),
    );
}

#[test]
fn complete_prompt_arguments_matches_current_completion() {
    let fixture = ToolFixture::new();
    assert_completion_matches_handrolled(
        &fixture,
        CompleteRequestParams::new(
            Reference::for_prompt("inspect_symbol"),
            ArgumentInfo::new("symbol", "gre"),
        ),
    );
}

#[test]
fn complete_resource_uri_matches_current_completion() {
    let fixture = ToolFixture::new();
    assert_completion_matches_handrolled(
        &fixture,
        CompleteRequestParams::new(
            Reference::for_resource("atlas://docs/index"),
            ArgumentInfo::new("uri", "atlas://"),
        ),
    );
}

#[test]
fn complete_source_id_matches_current_completion() {
    let fixture = ToolFixture::new();
    crate::tools::call(
        "save_context_artifact",
        Some(&json!({
            "content": "saved context fixture body",
            "kind": "note",
            "source_id": "src-completion-123",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("save context artifact");
    assert_completion_matches_handrolled(
        &fixture,
        CompleteRequestParams::new(
            Reference::for_resource("atlas://saved-context/{source_id}"),
            ArgumentInfo::new("source_id", "src-"),
        ),
    );
}

#[test]
fn complete_docs_heading_matches_current_completion() {
    let fixture = ToolFixture::new();
    let request = CompleteRequestParams::new(
        Reference::for_resource("atlas://docs/{file}#{heading}"),
        ArgumentInfo::new("heading", "document.st"),
    )
    .with_context(
        serde_json::from_value(json!({
            "arguments": { "file": "README.md" }
        }))
        .expect("completion context"),
    );
    assert_completion_matches_handrolled(&fixture, request);
}

#[test]
fn complete_git_ref_matches_current_completion() {
    let fixture = ToolFixture::new();
    assert_completion_matches_handrolled(
        &fixture,
        CompleteRequestParams::new(
            Reference::for_prompt("review_change"),
            ArgumentInfo::new("base", "ma"),
        ),
    );
}

#[test]
fn complete_unsupported_field_returns_empty_set() {
    let fixture = ToolFixture::new();
    let result = fixture
        .server
        .complete_result(CompleteRequestParams::new(
            Reference::for_prompt("tools/call"),
            ArgumentInfo::new("output_format", "j"),
        ))
        .expect("rmcp completion");
    assert_eq!(result.completion.values, Vec::<String>::new());
    assert_eq!(result.completion.has_more, Some(false));
}

#[test]
fn accepted_subscription_filter_is_supported_subset() {
    let fixture = ToolFixture::new();
    let requested = SubscriptionFilter::builder()
        .tools_list_changed()
        .prompts_list_changed()
        .resources_list_changed()
        .resource_subscription("atlas://docs/index")
        .resource_subscription("atlas://prompt-docs/review_change")
        .build();
    let accepted = fixture
        .server
        .accepted_subscription_filter_result(&requested);
    assert_eq!(accepted, requested);
}

#[test]
fn bootstrap_notification_plan_only_emits_requested_categories() {
    let fixture = ToolFixture::new();
    let accepted = SubscriptionFilter::builder()
        .tools_list_changed()
        .resource_subscription("atlas://docs/index")
        .resource_subscription("atlas://prompt-docs/review_change")
        .build();
    let plan = fixture.server.bootstrap_notification_plan(&accepted);
    assert_eq!(
        plan,
        super::BootstrapNotificationPlan {
            send_tool_list_changed: true,
            send_prompt_list_changed: false,
            send_resource_list_changed: false,
            resource_updates: vec![
                "atlas://docs/index".to_owned(),
                "atlas://prompt-docs/review_change".to_owned(),
            ],
        }
    );
}
