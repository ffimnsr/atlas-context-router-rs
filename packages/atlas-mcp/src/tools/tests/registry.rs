use super::*;
use crate::session_events::{SUPPORTED_EVENTS, is_supported_event_name};
use crate::tools::registry::{ToolResultContract, tool_result_contract};
use crate::tools::tool_descriptors;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const TOOL_REGISTRY_SNAPSHOT: &[&str] = &[
    "analyze_architecture",
    "analyze_dead_code",
    "analyze_dependency",
    "analyze_metrics",
    "analyze_patterns",
    "analyze_remove",
    "analyze_safety",
    "assess_risk",
    "batch_query_graph",
    "broker_status",
    "build_or_update_graph",
    "compact_session",
    "concept_clusters",
    "cross_file_links",
    "cross_session_search",
    "db_check",
    "debug_graph",
    "detect_changes",
    "doctor",
    "explain_change",
    "explain_query",
    "find_complex_functions",
    "find_duplicates",
    "find_large_functions",
    "find_similar_functions",
    "get_context",
    "get_context_stats",
    "get_docs_section",
    "get_global_memory",
    "get_impact_radius",
    "get_minimal_context",
    "get_review_context",
    "get_session_status",
    "infer_modules",
    "label_components",
    "list_graph_stats",
    "man",
    "postprocess_graph",
    "purge_saved_context",
    "query_graph",
    "read_file_around_match",
    "read_file_excerpt",
    "read_saved_context",
    "record_session_event",
    "repo_registry",
    "resolve_symbol",
    "resume_session",
    "save_context_artifact",
    "search_content",
    "search_decisions",
    "search_files",
    "search_saved_context",
    "search_templates",
    "search_text_assets",
    "status",
    "symbol_neighbors",
    "tool_help",
    "tool_list",
    "tool_search",
    "traverse_graph",
    "wake_up",
];

fn manual_snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
        .join("manual_docs")
        .join(format!("{name}.json"))
}

fn manual_contract_snapshot(tool_name: &str, repo_root: &str, db_path: &str) -> Value {
    let response = call(
        "tool_help",
        Some(&json!({ "name": tool_name, "output_format": "json" })),
        repo_root,
        db_path,
    )
    .expect("tool_help response");
    let payload: Value = serde_json::from_str(&unwrap_tool_text(response)).expect("json payload");
    json!({
        "resolved_tool_name": payload["resolved_tool_name"],
        "input_contract": payload["input_contract"],
    })
}

fn assert_manual_contract_snapshot(tool_name: &str, repo_root: &str, db_path: &str) {
    let actual = manual_contract_snapshot(tool_name, repo_root, db_path);
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(manual_snapshot_path(tool_name)).expect("read manual snapshot"),
    )
    .expect("parse manual snapshot");
    assert_eq!(actual, expected, "manual snapshot mismatch for {tool_name}");
}

fn parity_seed_source_id(repo_root: &str, db_path: &str) -> String {
    let content = std::iter::repeat_n("parity seed artifact content with safe spacing", 20)
        .collect::<Vec<_>>()
        .join(" ");
    let args = json!({
        "content": content,
        "label": "parity-seed",
        "output_format": "json"
    });

    let response = call("save_context_artifact", Some(&args), repo_root, db_path)
        .expect("seed saved artifact");
    let body: Value = serde_json::from_str(&unwrap_tool_text(response)).expect("parse save json");
    body.get("source_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .expect("save_context_artifact must return source_id for parity seed")
        .to_owned()
}

fn parity_args(tool_name: &str, source_id: &str) -> Value {
    match tool_name {
        "list_graph_stats" => json!({ "output_format": "json" }),
        "tool_list" => json!({ "output_format": "json" }),
        "tool_search" => json!({ "query": "query", "output_format": "json" }),
        "tool_help" => json!({ "name": "query_graph", "output_format": "json" }),
        "man" => json!({ "namespace": "mcp", "tool_name": "query_graph", "output_format": "json" }),
        "query_graph" => json!({ "text": "compute", "output_format": "json" }),
        "batch_query_graph" => json!({
            "items": [{ "text": "compute" }, { "text": "handle_request" }],
            "output_format": "json"
        }),
        "get_impact_radius" => {
            json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" })
        }
        "get_review_context" => {
            json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" })
        }
        "detect_changes" => {
            json!({ "change_source": { "kind": "working_tree" }, "output_format": "json" })
        }
        "build_or_update_graph" => {
            json!({ "operation": { "kind": "update", "change_source": { "kind": "files", "files": ["src/service.rs"] } }, "output_format": "json" })
        }
        "postprocess_graph" => {
            json!({ "changed_only": true, "stage": "flows", "dry_run": true, "output_format": "json" })
        }
        "traverse_graph" => {
            json!({ "from_qn": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "get_minimal_context" => {
            json!({ "change_source": { "kind": "working_tree" }, "output_format": "json" })
        }
        "explain_change" => {
            json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" })
        }
        "get_context" => {
            json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" })
        }
        "analyze_architecture" => json!({ "output_format": "json" }),
        "analyze_metrics" => json!({ "output_format": "json" }),
        "assess_risk" => {
            json!({ "symbol": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "analyze_patterns" => json!({ "output_format": "json" }),
        "find_large_functions" => {
            json!({ "threshold": 2, "mode": "large", "output_format": "json" })
        }
        "find_complex_functions" => {
            json!({ "complexity_threshold": 1, "output_format": "json" })
        }
        "find_similar_functions" => {
            json!({ "symbol": "compute", "output_format": "json" })
        }
        "find_duplicates" => json!({ "output_format": "json" }),
        "infer_modules" => json!({ "output_format": "json" }),
        "label_components" => json!({ "output_format": "json" }),
        "get_session_status" => json!({ "output_format": "json" }),
        "compact_session" => json!({ "output_format": "json" }),
        "resume_session" => json!({ "mark_consumed": false, "output_format": "json" }),
        "record_session_event" => {
            json!({ "event": "user-prompt", "payload": { "prompt": "parity" }, "output_format": "json" })
        }
        "wake_up" => json!({ "output_format": "json" }),
        "search_saved_context" => json!({ "query": "parity-seed", "output_format": "json" }),
        "search_decisions" => json!({ "query": "parity-seed", "output_format": "json" }),
        "read_saved_context" => json!({ "source_id": source_id, "output_format": "json" }),
        "repo_registry" => json!({ "output_format": "json" }),
        "save_context_artifact" => json!({
            "content": "parity preview payload".repeat(40),
            "label": "parity-save",
            "output_format": "json"
        }),
        "get_context_stats" => json!({ "output_format": "json" }),
        "purge_saved_context" => json!({ "keep_days": 365, "output_format": "json" }),
        "cross_session_search" => json!({ "query": "parity-seed", "output_format": "json" }),
        "get_global_memory" => json!({ "limit": 5, "output_format": "json" }),
        "symbol_neighbors" => {
            json!({ "qname": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "cross_file_links" => json!({ "file": "src/service.rs", "output_format": "json" }),
        "concept_clusters" => json!({ "files": ["src/service.rs"], "output_format": "json" }),
        "search_files" => json!({ "pattern": "*.rs", "output_format": "json" }),
        "search_content" => json!({ "query": "compute", "output_format": "json" }),
        "read_file_excerpt" => {
            json!({ "file": "src/service.rs", "selector": { "kind": "range", "start_line": 1, "end_line": 3 }, "output_format": "json" })
        }
        "get_docs_section" => {
            json!({ "file": "README.md", "selector": { "kind": "heading", "heading": "document.overview" }, "output_format": "json" })
        }
        "read_file_around_match" => {
            json!({ "file": "src/service.rs", "query": "compute", "output_format": "json" })
        }
        "search_templates" => json!({ "kind": "html", "output_format": "json" }),
        "search_text_assets" => json!({ "kind": "config", "output_format": "json" }),
        "broker_status" => json!({ "output_format": "json" }),
        "status" => json!({ "output_format": "json" }),
        "doctor" => json!({ "output_format": "json" }),
        "db_check" => json!({ "output_format": "json" }),
        "debug_graph" => json!({ "output_format": "json" }),
        "explain_query" => json!({ "text": "compute", "output_format": "json" }),
        "resolve_symbol" => json!({ "name": "compute", "output_format": "json" }),
        "analyze_safety" => {
            json!({ "symbol": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "analyze_remove" => {
            json!({ "symbols": ["src/service.rs::fn::compute"], "output_format": "json" })
        }
        "analyze_dead_code" => json!({ "summary": true, "output_format": "json" }),
        "analyze_dependency" => {
            json!({ "symbol": "src/service.rs::fn::compute", "output_format": "json" })
        }
        other => panic!("missing parity args for tool {other}"),
    }
}

#[test]
fn exported_registry_includes_tool_inventory_helpers() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    assert!(
        tools
            .iter()
            .any(|t| t.get("name") == Some(&"tool_list".into()))
    );
    assert!(
        tools
            .iter()
            .any(|t| t.get("name") == Some(&"tool_search".into()))
    );
    assert!(
        tools
            .iter()
            .any(|t| t.get("name") == Some(&"tool_help".into()))
    );
    assert!(tools.iter().any(|t| t.get("name") == Some(&"man".into())));
    assert!(
        tools
            .iter()
            .any(|t| t.get("name") == Some(&"repo_registry".into()))
    );
}

#[test]
fn tool_inventory_list_returns_compact_runtime_catalog() {
    let fixture = setup_git_mcp_fixture();
    let response = call(
        "tool_list",
        Some(&json!({
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("tool_list response");

    let payload: Value =
        serde_json::from_str(&unwrap_tool_text(response.clone())).expect("json payload");
    assert!(payload["total_tools"].as_u64().unwrap() >= 4);
    assert!(
        payload["tools"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["name"] == json!("tool_help")))
    );
    assert_provenance(&response, &fixture.repo_root, &fixture.db_path);
}

#[test]
fn tool_inventory_search_finds_query_graph() {
    let fixture = setup_git_mcp_fixture();
    let response = call(
        "tool_search",
        Some(&json!({
            "query": "query",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("tool_search response");

    let payload: Value =
        serde_json::from_str(&unwrap_tool_text(response.clone())).expect("json payload");
    assert!(payload["matches"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["name"] == json!("query_graph"))
    }));
    assert_provenance(&response, &fixture.repo_root, &fixture.db_path);
}

#[test]
fn tool_help_returns_same_manual_payload_shape() {
    let fixture = setup_git_mcp_fixture();
    let response = call(
        "tool_help",
        Some(&json!({
            "name": "query_graph",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("tool_help response");

    let payload: Value =
        serde_json::from_str(&unwrap_tool_text(response.clone())).expect("json payload");
    assert_eq!(payload["resolved_tool_name"], json!("query_graph"));
    assert_eq!(payload["requested_namespace"], json!("mcp"));
    assert!(payload["input_contract"]["canonical_form"].is_string());
    assert!(
        payload["usage"]["target_tool_call_examples"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item
                .as_str()
                .is_some_and(|text| text.contains("who calls compute"))))
    );
    assert_provenance(&response, &fixture.repo_root, &fixture.db_path);
}

#[test]
fn tool_help_query_grammar_examples_cover_query_graph_get_context_and_resolve_symbol() {
    let fixture = setup_git_mcp_fixture();
    for tool_name in ["query_graph", "get_context", "resolve_symbol"] {
        let response = call(
            "tool_help",
            Some(&json!({ "name": tool_name, "output_format": "json" })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("tool_help response");
        let payload: Value =
            serde_json::from_str(&unwrap_tool_text(response)).expect("json payload");
        let examples = payload["usage"]["target_tool_call_examples"]
            .as_array()
            .expect("examples array");
        assert!(!examples.is_empty(), "examples required for {tool_name}");
    }
}

#[test]
fn man_tool_returns_structured_manual_payload() {
    let fixture = setup_git_mcp_fixture();
    let response = call(
        "man",
        Some(&json!({
            "namespace": "mcp",
            "tool_name": "query_graph",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("man response");

    assert_eq!(unwrap_tool_format(&response), "json");
    let payload: Value =
        serde_json::from_str(&unwrap_tool_text(response.clone())).expect("json payload");
    assert_eq!(payload["resolved_tool_name"], json!("query_graph"));
    assert_eq!(payload["usage"]["cli"], json!("man mcp query_graph"));
    assert!(payload["input_contract"]["canonical_form"].is_string());
    assert!(
        payload["input_args"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
    assert_provenance(&response, &fixture.repo_root, &fixture.db_path);
}

#[test]
fn man_unknown_tool_suggestions_are_deterministic() {
    let fixture = setup_git_mcp_fixture();
    let response = call(
        "man",
        Some(&json!({
            "namespace": "mcp",
            "tool_name": "query_grap",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("man error response");

    assert_eq!(response["isError"], json!(true));
    assert_eq!(
        response["structuredContent"]["details"]["suggestions"][0],
        json!("query_graph")
    );
}

#[test]
fn man_hidden_internal_tool_is_not_documented() {
    let fixture = setup_git_mcp_fixture();
    let response = call(
        "man",
        Some(&json!({
            "namespace": "mcp",
            "tool_name": "__test_sleep",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("man hidden error response");

    assert_eq!(response["isError"], json!(true));
    assert_eq!(
        response["structuredContent"]["details"]["reason"],
        json!("hidden_or_internal_tool")
    );
}

#[test]
fn tool_help_manual_contract_snapshots_match_for_high_frequency_tools() {
    let fixture = setup_git_mcp_fixture();
    for tool_name in [
        "get_context",
        "read_file_excerpt",
        "get_docs_section",
        "detect_changes",
        "build_or_update_graph",
        "batch_query_graph",
    ] {
        assert_manual_contract_snapshot(tool_name, &fixture.repo_root, &fixture.db_path);
    }
}

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
fn tool_list_includes_get_context_and_is_cacheable_and_sorted() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    assert!(
        tools
            .iter()
            .any(|t| t.get("name") == Some(&"get_context".into())),
        "tools/list must include get_context"
    );
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(
        names, sorted,
        "tools/list must stay sorted for cache stability"
    );
    assert_eq!(list["resultType"], serde_json::json!("complete"));
    assert_eq!(list["ttlMs"], serde_json::json!(300000));
    assert_eq!(list["cacheScope"], serde_json::json!("public"));
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
fn record_session_event_description_lists_only_supported_events() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    let tool = tools
        .iter()
        .find(|t| t.get("name") == Some(&json!("record_session_event")))
        .expect("record_session_event must be in registry");
    let description = tool["description"].as_str().expect("description");

    // The key lifecycle events must be listed explicitly and be supported.
    for event in [
        "session-start",
        "user-prompt",
        "pre-tool-use",
        "post-tool-use",
        "pre-compact",
        "post-compact",
        "stop",
        "session-end",
        "file-changed",
        "tool-failure",
        "error",
    ] {
        assert!(
            description.contains(event),
            "registry description must list event {event}"
        );
        assert!(is_supported_event_name(event));
    }

    // Every hyphenated token in the description must be a supported event
    // name, so the registry can never describe a conflicting event.
    for token in description.split(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
        if token.contains('-') {
            assert!(
                is_supported_event_name(token),
                "registry description lists unsupported event name: {token}"
            );
        }
    }

    // The exported supported list is the single source of truth and stays
    // non-empty.
    assert!(SUPPORTED_EVENTS.len() >= 20);
}

#[test]
fn tool_list_matches_registry_snapshot() {
    let list = tool_list();
    let names = list
        .get("tools")
        .and_then(|value| value.as_array())
        .expect("tools array")
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(|value| value.as_str())
                .expect("tool name")
        })
        .collect::<Vec<_>>();

    assert_eq!(names, TOOL_REGISTRY_SNAPSHOT);
}

#[test]
fn tool_list_names_are_unique() {
    let list = tool_list();
    let names = list
        .get("tools")
        .and_then(|value| value.as_array())
        .expect("tools array")
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(|value| value.as_str())
                .expect("tool name")
        })
        .collect::<Vec<_>>();
    let unique = names.iter().copied().collect::<BTreeSet<_>>();

    assert_eq!(unique.len(), names.len(), "tool_list must not repeat names");
}

#[test]
fn tool_result_value_falls_back_to_json_when_toon_is_empty() {
    let rendered =
        tool_result_value(&serde_json::json!({}), OutputFormat::Toon).expect("tool result");

    assert_eq!(unwrap_tool_format(&rendered), "json");
    assert!(fallback_reason(&rendered).is_some());
}

#[test]
fn invalid_output_format_returns_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "xml" });
    let result = call("get_context", Some(&args), "/ignored", &db_path)
        .expect("invalid output_format should return tool error result");

    assert_eq!(result["isError"], serde_json::json!(true));
    assert!(
        result["structuredContent"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("unsupported output_format"))
    );
}

#[test]
fn search_content_invalid_regex_returns_strict_guidance() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_repo_file(
        dir.path(),
        "src/lib.rs",
        "pub enum Command {\n    Context { value: String },\n}\n",
    );
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let args = json!({
        "query": "Command::Context|Context {",
        "is_regex": true,
        "output_format": "json"
    });

    let result = call(
        "search_content",
        Some(&args),
        dir.path().to_str().expect("repo root"),
        &db_path,
    )
    .expect("invalid regex must return tool error result");

    assert_eq!(result["isError"], json!(true));
    assert_eq!(result["structuredContent"]["code"], json!("invalid_input"));
    let message = result["structuredContent"]["message"]
        .as_str()
        .expect("message");
    let detail = result["structuredContent"]["details"]["detail"]
        .as_str()
        .expect("detail");
    assert!(
        message.contains("invalid regex pattern for search_content"),
        "expected strict regex guidance, got: {message}"
    );
    assert!(
        detail.contains("Set is_regex=false for literal text search"),
        "expected literal-search guidance, got detail: {detail}"
    );
    assert!(
        detail.contains(r"Command::Context|Context \{"),
        "expected escaped regex example, got detail: {detail}"
    );
}

#[test]
fn tool_list_includes_analysis_and_insight_tools() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    for name in &[
        "analyze_architecture",
        "analyze_metrics",
        "assess_risk",
        "analyze_patterns",
        "find_large_functions",
        "find_complex_functions",
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

#[test]
fn every_listed_tool_dispatches_with_parity_fixture_args() {
    let fixture = setup_git_mcp_fixture();
    let source_id = parity_seed_source_id(&fixture.repo_root, &fixture.db_path);
    let tool_list_value = tool_list();
    let tools = tool_list_value["tools"].as_array().expect("tools array");

    for tool in tools {
        let name = tool["name"].as_str().expect("tool name");
        let args = parity_args(name, &source_id);
        let response = call(name, Some(&args), &fixture.repo_root, &fixture.db_path)
            .unwrap_or_else(|error| panic!("tool {name} failed to dispatch: {error}"));

        assert_eq!(unwrap_tool_format(&response), "json", "tool {name}");
        assert_provenance(&response, &fixture.repo_root, &fixture.db_path);

        let text = unwrap_tool_text(response);
        serde_json::from_str::<Value>(&text).unwrap_or_else(|error| {
            panic!("tool {name} returned invalid json: {error}; body={text}")
        });
    }
}

#[test]
fn content_companion_tools_describe_companion_lookup_contract() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();

    for name in &[
        "search_text_assets",
        "search_templates",
        "search_content",
        "search_files",
    ] {
        let tool = tools
            .iter()
            .find(|t| t.get("name") == Some(&serde_json::Value::String((*name).to_owned())))
            .unwrap_or_else(|| panic!("tool {name} not found in registry"));
        let desc = tool["description"].as_str().unwrap_or_default();
        assert!(
            desc.contains("companion")
                || desc.contains("graph evidence")
                || desc.contains("graph tools"),
            "tool {name} description must reference graph/content companion contract, got: {desc}"
        );
    }
}

#[test]
fn get_context_description_mentions_content_asset_merging() {
    let list = tool_list();
    let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
    let tool = tools
        .iter()
        .find(|t| t.get("name") == Some(&serde_json::Value::String("get_context".to_owned())))
        .expect("get_context must be in registry");
    let desc = tool["description"].as_str().unwrap_or_default();
    assert!(
        desc.contains("bounded selection") || desc.contains("bounded"),
        "get_context description must mention bounded selection policy, got: {desc}"
    );
    assert!(
        desc.contains("config") || desc.contains("templates") || desc.contains("non-code"),
        "get_context description must mention non-code companion use case, got: {desc}"
    );
}

#[test]
fn get_docs_section_reports_freshness_when_doc_changes_after_index() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        fixture._dir.path(),
        "README.md",
        "# Overview\nfixture docs changed\n## Install\nstep\n",
    );

    let response = call(
        "get_docs_section",
        Some(&json!({
            "file": "README.md",
            "selector": { "kind": "heading", "heading": "document.overview" },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_docs_section response");

    assert_eq!(
        response
            .pointer("/atlas_freshness/stale")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        response
            .pointer("/atlas_freshness/stale_result_files/0")
            .and_then(|v| v.as_str()),
        Some("README.md")
    );
}

#[test]
fn exported_input_schemas_reject_known_legacy_ambiguous_field_groups() {
    let list = tool_list();
    let tools = list["tools"].as_array().expect("tools array");

    let forbidden_top_level_fields = [
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
            "detect_changes",
            vec!["mode", "base", "staged", "working_tree"],
        ),
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
        ("batch_query_graph", vec!["queries", "text"]),
    ];

    for (tool_name, forbidden_fields) in forbidden_top_level_fields {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == json!(tool_name))
            .unwrap_or_else(|| panic!("tool {tool_name} missing from registry"));
        let properties = tool["inputSchema"]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("tool {tool_name} missing input properties"));
        for field in forbidden_fields {
            assert!(
                !properties.contains_key(field),
                "tool {tool_name} must not expose legacy top-level field {field}"
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
            .find(|tool| tool["name"] == json!(tool_name))
            .unwrap_or_else(|| panic!("tool {tool_name} missing from registry"));
        let properties = tool["inputSchema"]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("tool {tool_name} missing input properties"));
        assert!(properties.contains_key("repo_scope"));
        assert!(
            !properties.contains_key("repo_id"),
            "tool {tool_name} must not expose legacy top-level repo_id"
        );
        assert!(
            !properties.contains_key("all_repos"),
            "tool {tool_name} must not expose legacy top-level all_repos"
        );
    }
}

#[test]
fn tool_descriptions_do_not_hide_precedence_rules() {
    let list = tool_list();
    let tools = list["tools"].as_array().expect("tools array");
    let banned_phrases = [" wins", "takes precedence", "ignored when both"];

    for tool in tools {
        let name = tool["name"].as_str().expect("tool name");
        let description = tool["description"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        for phrase in banned_phrases {
            assert!(
                !description.contains(phrase),
                "tool {name} description must not contain hidden precedence phrase {phrase:?}: {description}"
            );
        }
    }
}

#[test]
fn stable_object_tools_export_object_structured_content_schema() {
    for descriptor in tool_descriptors() {
        let name = descriptor.name.as_str();
        if tool_result_contract(name) != ToolResultContract::StableObject {
            continue;
        }
        let output_schema = descriptor
            .output_schema
            .as_ref()
            .unwrap_or_else(|| panic!("tool {name} missing outputSchema"));
        assert_eq!(
            output_schema.get("type").and_then(|value| value.as_str()),
            Some("object"),
            "tool {name} stable-object contract must expose object outputSchema"
        );
    }
}
