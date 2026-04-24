use super::*;
use serde_json::{Value, json};

fn parity_seed_source_id(repo_root: &str, db_path: &str) -> String {
    let content = "x".repeat(600);
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
        "query_graph" => json!({ "text": "compute", "output_format": "json" }),
        "batch_query_graph" => json!({ "text": "compute handle_request", "output_format": "json" }),
        "get_impact_radius" => json!({ "files": ["src/service.rs"], "output_format": "json" }),
        "get_review_context" => json!({ "files": ["src/service.rs"], "output_format": "json" }),
        "detect_changes" => json!({ "working_tree": true, "output_format": "json" }),
        "build_or_update_graph" => {
            json!({ "mode": "update", "files": ["src/service.rs"], "output_format": "json" })
        }
        "traverse_graph" => {
            json!({ "from_qn": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "get_minimal_context" => json!({ "working_tree": true, "output_format": "json" }),
        "explain_change" => json!({ "files": ["src/service.rs"], "output_format": "json" }),
        "get_context" => json!({ "query": "compute", "output_format": "json" }),
        "get_session_status" => json!({ "output_format": "json" }),
        "compact_session" => json!({ "output_format": "json" }),
        "resume_session" => json!({ "mark_consumed": false, "output_format": "json" }),
        "search_saved_context" => json!({ "query": "parity-seed", "output_format": "json" }),
        "read_saved_context" => json!({ "source_id": source_id, "output_format": "json" }),
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
        "search_templates" => json!({ "kind": "html", "output_format": "json" }),
        "search_text_assets" => json!({ "kind": "config", "output_format": "json" }),
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
