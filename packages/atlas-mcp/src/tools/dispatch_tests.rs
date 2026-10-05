use super::*;
use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};
use atlas_store_sqlite::Store;
use jsonschema::{Draft, JSONSchema};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn setup_repo() -> (TempDir, PathBuf, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(
        src_dir.join("lib.rs"),
        "pub mod service;\npub fn greet() -> &'static str { \"hi\" }\n",
    )
    .expect("write fixture source");
    fs::write(
        src_dir.join("service.rs"),
        "pub fn compute() -> i32 { 1 }\n",
    )
    .expect("write fixture service source");
    fs::write(
        src_dir.join("api.rs"),
        "pub fn handle_request() -> i32 { crate::service::compute() }\n",
    )
    .expect("write fixture api source");
    let tests_dir = dir.path().join("tests");
    fs::create_dir_all(&tests_dir).expect("create tests dir");
    fs::write(
        tests_dir.join("service_test.rs"),
        "#[test]\nfn compute_test() { assert_eq!(crate::service::compute(), 1); }\n",
    )
    .expect("write fixture test source");
    fs::write(
        dir.path().join("README.md"),
        "# Fixture Repo\n\n## Status\n\nFixture status content.\n",
    )
    .expect("write fixture readme");
    fs::create_dir_all(dir.path().join("config")).expect("create config dir");
    fs::write(dir.path().join("config/app.toml"), "name = \"fixture\"\n")
        .expect("write fixture config");
    fs::create_dir_all(dir.path().join("templates")).expect("create templates dir");
    fs::write(
        dir.path().join("templates/index.html"),
        "<html><body>{{ greet }}</body></html>\n",
    )
    .expect("write fixture template");
    fs::create_dir_all(dir.path().join("queries")).expect("create queries dir");
    fs::write(dir.path().join("queries/example.sql"), "select 1;\n").expect("write fixture sql");
    git(dir.path(), &["init", "--quiet"]);
    git(dir.path(), &["config", "user.name", "Atlas Tests"]);
    git(
        dir.path(),
        &["config", "user.email", "atlas-tests@example.com"],
    );
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "--quiet", "-m", "fixture baseline"]);
    let db_path = dir.path().join(".atlas").join("worldtree.db");
    (dir, db_path.clone(), db_path.to_string_lossy().into_owned())
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn make_node(kind: NodeKind, name: &str, qn: &str, file: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_owned(),
        qualified_name: qn.to_owned(),
        file_path: file.to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: None,
        is_test: kind == NodeKind::Test,
        file_hash: format!("hash:{file}"),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    }
}

fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file.to_owned(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: None,
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    }
}

fn seed_schema_graph(db_path: &str) {
    let mut store = Store::open(db_path).expect("open store");

    let compute = make_node(
        NodeKind::Function,
        "compute",
        "src/service.rs::fn::compute",
        "src/service.rs",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "src/service.rs",
            "hash:src/service.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute),
            &[],
        )
        .expect("seed service graph");

    let handle = make_node(
        NodeKind::Function,
        "handle_request",
        "src/api.rs::fn::handle_request",
        "src/api.rs",
    );
    let handle_calls_compute = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "src/api.rs",
            "hash:src/api.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&handle),
            &[handle_calls_compute],
        )
        .expect("seed api graph");

    let compute_test = make_node(
        NodeKind::Test,
        "compute_test",
        "tests/service_test.rs::fn::compute_test",
        "tests/service_test.rs",
    );
    let test_targets_compute = make_edge(
        EdgeKind::Tests,
        "tests/service_test.rs::fn::compute_test",
        "src/service.rs::fn::compute",
        "tests/service_test.rs",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "tests/service_test.rs",
            "hash:tests/service_test.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute_test),
            &[test_targets_compute],
        )
        .expect("seed test graph");
}

#[test]
fn every_tool_descriptor_name_routes_through_dispatcher() {
    let (repo_dir, _db_path, db_path) = setup_repo();
    let repo_root = repo_dir.path().to_string_lossy().into_owned();

    for tool in super::super::registry::tool_list()["tools"]
        .as_array()
        .expect("tools array")
    {
        let name = tool["name"].as_str().expect("tool name");
        let result = call(
            name,
            Some(&json!({"output_format": "json"})),
            &repo_root,
            &db_path,
        );
        if let Err(error) = result {
            assert!(
                !error.to_string().starts_with("unknown tool:"),
                "descriptor name must dispatch: {name}"
            );
        }
    }
}

fn assert_matches_output_schema(name: &str, response: &serde_json::Value, schema: &JSONSchema) {
    let structured = response
        .get("structuredContent")
        .expect("structuredContent")
        .clone();
    assert!(
        structured.is_object(),
        "{name} structuredContent must be object when outputSchema exists"
    );
    if let Err(errors) = schema.validate(&structured) {
        let details = errors
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        panic!("{name} output schema mismatch:\n{details}\nvalue={structured:#}");
    }
}

fn schema_test_artifact_content() -> String {
    "schema-test artifact payload ".repeat(32)
}

fn schema_test_args(name: &str, saved_source_id: &str) -> serde_json::Value {
    match name {
        "list_graph_stats" => json!({"output_format": "json"}),
        "tool_list" => json!({"output_format": "json"}),
        "tool_search" => json!({"query": "query", "output_format": "json"}),
        "tool_help" => json!({"name": "query_graph", "output_format": "json"}),
        "man" => {
            json!({"namespace": "mcp", "tool_name": "query_graph", "output_format": "json"})
        }
        "query_graph" => json!({"text": "greet", "output_format": "json"}),
        "batch_query_graph" => {
            json!({"items": [{"text": "greet"}], "output_format": "json"})
        }
        "get_impact_radius" => {
            json!({"change_source": {"kind": "files", "files": ["src/lib.rs"]}, "output_format": "json"})
        }
        "get_review_context" => {
            json!({"change_source": {"kind": "files", "files": ["src/lib.rs"]}, "output_format": "json"})
        }
        "detect_changes" => {
            json!({"change_source": {"kind": "working_tree"}, "output_format": "json"})
        }
        "build_graph" => json!({"output_format": "json"}),
        "update_graph" => {
            json!({"change_source": {"kind": "files", "files": ["src/lib.rs"]}, "output_format": "json"})
        }
        "postprocess_graph" => json!({"dry_run": true, "output_format": "json"}),
        "traverse_graph" => {
            json!({"from_qn": "src/lib.rs::fn::greet", "output_format": "json"})
        }
        "get_minimal_context" => {
            json!({"change_source": {"kind": "working_tree"}, "output_format": "json"})
        }
        "explain_change" => {
            json!({"change_source": {"kind": "files", "files": ["src/lib.rs"]}, "output_format": "json"})
        }
        "get_context" => {
            json!({"target": {"kind": "query", "query": "greet"}, "output_format": "json"})
        }
        "analyze_architecture" => json!({"output_format": "json"}),
        "analyze_metrics" => json!({"output_format": "json"}),
        "assess_risk" => {
            json!({"symbol": "src/service.rs::fn::compute", "output_format": "json"})
        }
        "analyze_patterns" => json!({"output_format": "json"}),
        "find_large_functions" => json!({"output_format": "json"}),
        "find_complex_functions" => json!({"output_format": "json"}),
        "find_similar_functions" => {
            json!({"symbol": "greet", "output_format": "json"})
        }
        "find_duplicates" => json!({"output_format": "json"}),
        "infer_modules" => json!({"output_format": "json"}),
        "label_components" => json!({"output_format": "json"}),
        "get_session_status" => json!({"output_format": "json"}),
        "compact_session" => json!({"output_format": "json"}),
        "resume_session" => json!({"output_format": "json"}),
        "record_session_event" => {
            json!({"event": "user-prompt", "payload": {"prompt": "schema-test"}, "output_format": "json"})
        }
        "wake_up" => json!({"output_format": "json"}),
        "search_saved_context" => json!({"query": "schema-test", "output_format": "json"}),
        "search_decisions" => json!({"query": "schema-test", "output_format": "json"}),
        "read_saved_context" => json!({"source_id": saved_source_id, "output_format": "json"}),
        "save_context_artifact" => json!({
            "content": schema_test_artifact_content(),
            "label": "schema-test-artifact-second",
            "source_type": "mcp_artifact",
            "content_type": "text/plain",
            "output_format": "json"
        }),
        "get_context_stats" => json!({"output_format": "json"}),
        "purge_saved_context" => json!({"keep_days": 36500, "output_format": "json"}),
        "cross_session_search" => json!({"query": "schema-test", "output_format": "json"}),
        "get_global_memory" => json!({"output_format": "json"}),
        "memory_store" => json!({"text": "schema-test memory", "output_format": "json"}),
        "memory_recall" => json!({"query": "schema-test", "output_format": "json"}),
        "feedback_record" => json!({
            "predicted": "schema-test dead",
            "actual": "schema-test alive",
            "output_format": "json"
        }),
        "symbol_neighbors" => {
            json!({"qname": "src/lib.rs::fn::greet", "output_format": "json"})
        }
        "cross_file_links" => json!({"file": "src/lib.rs", "output_format": "json"}),
        "concept_clusters" => json!({"files": ["src/lib.rs"], "output_format": "json"}),
        "search_files" => json!({"pattern": "*.rs", "output_format": "json"}),
        "search_content" => json!({"query": "greet", "output_format": "json"}),
        "read_file_excerpt" => {
            json!({"file": "src/lib.rs", "selector": {"kind": "range", "start_line": 1, "end_line": 1}, "output_format": "json"})
        }
        "get_docs_section" => {
            json!({"file": "README.md", "selector": {"kind": "heading", "heading": "Status"}, "output_format": "json"})
        }
        "read_file_around_match" => {
            json!({"file": "src/lib.rs", "query": "greet", "output_format": "json"})
        }
        "search_templates" => json!({"output_format": "json"}),
        "search_text_assets" => json!({"output_format": "json"}),
        "repo_registry" => json!({"output_format": "json"}),
        "broker_status" => json!({"output_format": "json"}),
        "status" => json!({"output_format": "json"}),
        "doctor" => json!({"output_format": "json"}),
        "db_check" => json!({"output_format": "json"}),
        "debug_graph" => json!({"output_format": "json"}),
        "explain_query" => json!({"text": "greet", "output_format": "json"}),
        "resolve_symbol" => json!({"name": "greet", "output_format": "json"}),
        "list_symbols" => json!({"kind": "function", "limit": 10, "output_format": "json"}),
        "analyze_safety" => {
            json!({"symbol": "src/service.rs::fn::compute", "output_format": "json"})
        }
        "analyze_remove" => {
            json!({"symbols": ["src/service.rs::fn::compute"], "output_format": "json"})
        }
        "analyze_dead_code" => json!({"output_format": "json"}),
        "analyze_dependency" => {
            json!({"symbol": "src/service.rs::fn::compute", "output_format": "json"})
        }
        other => panic!("missing schema test args for {other}"),
    }
}

#[test]
fn tools_with_output_schema_never_emit_array_or_scalar_structured_content() {
    let (repo_dir, _db_path, db_path) = setup_repo();
    let repo_root = repo_dir.path().to_string_lossy().into_owned();

    let _build = call(
        "build_graph",
        Some(&json!({"output_format": "json"})),
        &repo_root,
        &db_path,
    )
    .expect("build graph");
    seed_schema_graph(&db_path);

    let saved = call(
        "save_context_artifact",
        Some(&json!({
            "content": schema_test_artifact_content(),
            "label": "schema-test-artifact-shape",
            "source_type": "mcp_artifact",
            "content_type": "text/plain",
            "output_format": "json"
        })),
        &repo_root,
        &db_path,
    )
    .expect("seed saved context artifact");
    let saved_source_id = saved["structuredContent"]["source_id"]
        .as_str()
        .expect("saved source id");

    for tool in super::super::registry::tool_descriptors() {
        if tool.output_schema.is_none() {
            continue;
        }
        let name = tool.name.as_ref();
        let args = schema_test_args(name, saved_source_id);
        let value = call(name, Some(&args), &repo_root, &db_path)
            .unwrap_or_else(|error| panic!("{name} should succeed for shape test: {error}"));
        let structured = value
            .get("structuredContent")
            .unwrap_or_else(|| panic!("{name} missing structuredContent"));
        assert!(
            structured.is_object(),
            "{name} structuredContent must stay object-valued for normalized tools"
        );
    }
}

#[test]
fn tools_with_output_schema_emit_schema_compatible_structured_content() {
    let (repo_dir, _db_path, db_path) = setup_repo();
    let repo_root = repo_dir.path().to_string_lossy().into_owned();

    let build = call(
        "build_graph",
        Some(&json!({"output_format": "json"})),
        &repo_root,
        &db_path,
    )
    .expect("build graph");
    let _ = build;
    seed_schema_graph(&db_path);

    let saved = call(
        "save_context_artifact",
        Some(&json!({
            "content": schema_test_artifact_content(),
            "label": "schema-test-artifact-seed",
            "source_type": "mcp_artifact",
            "content_type": "text/plain",
            "output_format": "json"
        })),
        &repo_root,
        &db_path,
    )
    .expect("seed saved context artifact");
    let saved_source_id = saved["structuredContent"]["source_id"]
        .as_str()
        .expect("saved source id");

    for tool in super::super::registry::tool_descriptors() {
        let Some(output_schema) = tool.output_schema.as_ref() else {
            continue;
        };
        let schema_value = serde_json::Value::Object((**output_schema).clone());
        let schema = JSONSchema::options()
            .with_draft(Draft::Draft202012)
            .compile(&schema_value)
            .unwrap_or_else(|error| panic!("{} output schema should compile: {error}", tool.name));
        let name = tool.name.as_ref();
        let args = schema_test_args(name, saved_source_id);
        let value = call(name, Some(&args), &repo_root, &db_path)
            .unwrap_or_else(|error| panic!("{name} should succeed for schema test: {error}"));
        assert_matches_output_schema(name, &value, &schema);
    }
}
